//! General Feed source adapters.
//!
//! A collector's job is discovery: it answers "which URLs did this source
//! surface, and what does it call them". It does not extract. Building the item
//! — title, author, content — belongs to `media::fetch`, so a repository found
//! on Trending and the same repository pasted by hand land at the same quality
//! instead of at 300 characters versus 20k (#79).
//!
//! They do not own opportunity scoring (Scouting) and never execute
//! provider-generated code. Every run is bounded by configuration and feeds the
//! same revisioned evaluation path as manual ingest.

use crate::config::FeedSourceConfig;
use crate::{CommsError, Result};

const MAX_SOURCE_ITEMS: usize = 30;

/// One URL a source surfaced, plus what that source calls it. The label is
/// origin provenance, not content — it ends up on the `feed_origins` row that
/// tells the reader where an item came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub url: String,
    pub label: Option<String>,
    /// The item's content as the source delivered it, for a source whose items cannot be
    /// fetched one by one. `None` means `media::fetch` builds the item from the URL.
    pub content: Option<Delivered>,
}

/// Title, author and body (HTML or text) a source delivered with a URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub title: String,
    pub author: Option<String>,
    pub body: String,
}

impl Discovered {
    fn new(url: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            label: Some(label.into()),
            content: None,
        }
    }
}

pub fn fetch(source: &FeedSourceConfig) -> Result<Vec<Discovered>> {
    match source.adapter.as_str() {
        "github-trending" => fetch_github_trending(source),
        "arxiv" => fetch_arxiv_query(source),
        "reddit-rss" => fetch_reddit_rss(source),
        other => Err(CommsError::Config(format!(
            "unknown Feed source adapter '{other}'"
        ))),
    }
}

/// The item `kind` this adapter's URLs land on, as `media::detect` reads them.
///
/// A third `match` on the same string, beside [`fetch`] and [`source_url`], because it answers a
/// third question about one adapter and putting it anywhere else would make the adapter list
/// live in two files. `None` for an adapter whose items are not identifiable by kind.
///
/// It exists so a stored row can be matched back to the source that would produce it *today*.
/// `captured_via` is empty on every row in this deployment, so the origin is not recorded and
/// cannot be read; `kind` is, and `media::detect` derives it from the URL on both paths — the
/// collector's and a hand-paste. Matching on it is therefore the same question ingest asked,
/// asked again, rather than a second rule about what a URL means.
///
/// The asymmetry is deliberate and is the safety property: `article` and `youtube` are what a
/// hand-pasted URL falls back to, no declared source produces them, so they match nothing here
/// and a re-derivation leaves them at whatever they had. Fail-closed, by not appearing.
pub fn item_kind(adapter: &str) -> Option<&'static str> {
    match adapter {
        "github-trending" => Some("github"),
        "arxiv" => Some("arxiv"),
        "reddit-rss" => Some("reddit"),
        _ => None,
    }
}

pub fn source_url(source: &FeedSourceConfig) -> String {
    match source.adapter.as_str() {
        "github-trending" => {
            let language = source
                .language
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("/{}", value.trim()))
                .unwrap_or_default();
            let since = normalized_since(source.since.as_deref());
            format!("https://github.com/trending{language}?since={since}")
        }
        "arxiv" => "https://export.arxiv.org/api/query".into(),
        "reddit-rss" => format!(
            "https://www.reddit.com/r/{}/top/.rss?t={}",
            source.query.as_deref().unwrap_or_default().trim(),
            match source.since.as_deref() {
                Some("day") => "day",
                Some("month") => "month",
                _ => "week",
            }
        ),
        _ => String::new(),
    }
}

fn fetch_github_trending(source: &FeedSourceConfig) -> Result<Vec<Discovered>> {
    let url = source_url(source);
    let body = http_client()?
        .get(&url)
        .send()?
        .error_for_status()?
        .text()?;
    Ok(parse_github_trending(
        &body,
        bounded_limit(source.limit),
        normalized_since(source.since.as_deref()),
    ))
}

/// Read the repository URLs off a Trending page. The card's visible text is
/// deliberately ignored: it is 162–389 characters of star counts, and the
/// repository extractor reads description, topics, license and README from the
/// same URL.
fn parse_github_trending(body: &str, limit: usize, since: &str) -> Vec<Discovered> {
    body.split("<article")
        .skip(1)
        .filter_map(|chunk| chunk.split_once("</article>").map(|(article, _)| article))
        .filter_map(|article| {
            let heading = article.split_once("<h2")?.1.split_once("</h2>")?.0;
            let href = attribute(heading, "href")?;
            let path = href.trim_matches('/');
            let mut parts = path.split('/');
            let owner = parts.next()?.trim();
            let repo = parts.next()?.trim();
            if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
                return None;
            }
            Some(Discovered::new(
                format!("https://github.com/{owner}/{repo}"),
                format!("GitHub Trending ({since})"),
            ))
        })
        .take(limit)
        .collect()
}

fn fetch_arxiv_query(source: &FeedSourceConfig) -> Result<Vec<Discovered>> {
    let query = source
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CommsError::Config(format!("arXiv source '{}' needs query", source.id)))?;
    let limit = bounded_limit(source.limit);
    let response = http_client()?
        .get(source_url(source))
        .query(&[
            ("search_query", query),
            ("start", "0"),
            ("max_results", &limit.to_string()),
            ("sortBy", "submittedDate"),
            ("sortOrder", "descending"),
        ])
        .send()?
        .error_for_status()?
        .text()?;
    Ok(parse_arxiv_entries(&response, limit))
}

/// Read the paper URLs off an Atom response. Title, authors and abstract are
/// not lifted here — `media::fetch_arxiv` reads them from the same identifier,
/// and having two paths build an arXiv item is how they drift apart.
fn parse_arxiv_entries(body: &str, limit: usize) -> Vec<Discovered> {
    body.split("<entry>")
        .skip(1)
        .filter_map(|chunk| chunk.split_once("</entry>").map(|(entry, _)| entry))
        .filter_map(|entry| {
            let id = xml_field(entry, "id")?;
            let url = id
                .replace("http://arxiv.org/abs/", "https://arxiv.org/abs/")
                .replace("http://export.arxiv.org/abs/", "https://arxiv.org/abs/");
            if !url.starts_with("https://arxiv.org/abs/") {
                return None;
            }
            Some(Discovered::new(url, "arXiv query"))
        })
        .take(limit)
        .collect()
}

/// A subreddit's top posts from its public Atom feed. Reddit refuses the per-post JSON to an
/// unauthenticated caller (403) and rate-limits the per-post feed (429), so each entry's own
/// title, author and HTML body are delivered with its URL instead of fetched again.
fn fetch_reddit_rss(source: &FeedSourceConfig) -> Result<Vec<Discovered>> {
    let subreddit = source
        .query
        .as_deref()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .ok_or_else(|| {
            CommsError::Config(format!(
                "reddit-rss source '{}' needs a subreddit name in query",
                source.id
            ))
        })?;
    let body = http_client()?
        .get(source_url(source))
        .send()?
        .error_for_status()?
        .text()?;
    Ok(parse_reddit_rss(
        &body,
        subreddit,
        &source.title_tokens,
        bounded_limit(source.limit),
    ))
}

fn parse_reddit_rss(
    body: &str,
    subreddit: &str,
    title_tokens: &[String],
    limit: usize,
) -> Vec<Discovered> {
    let prefix = format!("https://www.reddit.com/r/{subreddit}/comments/");
    let wanted: Vec<String> = title_tokens
        .iter()
        .map(|token| token.to_lowercase())
        .collect();
    body.split("<entry>")
        .skip(1)
        .filter_map(|chunk| chunk.split_once("</entry>").map(|(entry, _)| entry))
        .filter_map(|entry| {
            let url = entry
                .split("<link ")
                .skip(1)
                .find_map(|link| attribute(link, "href"))
                .filter(|href| href.starts_with(&prefix))?;
            let title = xml_field(entry, "title")?;
            let words: Vec<String> = title
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .map(str::to_string)
                .collect();
            if !wanted.is_empty() && !wanted.iter().any(|token| words.contains(token)) {
                return None;
            }
            let author = entry
                .split_once("<author>")
                .and_then(|(_, rest)| xml_field(rest, "name"));
            let body = entry
                .split_once("<content")
                .and_then(|(_, rest)| rest.split_once('>'))
                .and_then(|(_, rest)| rest.split_once("</content>"))
                .map(|(content, _)| decode_entities(content))
                .unwrap_or_default();
            Some(Discovered {
                url,
                label: Some(format!("r/{subreddit}")),
                content: Some(Delivered {
                    title,
                    author,
                    body,
                }),
            })
        })
        .take(limit)
        .collect()
}

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(sjel_http::client(
        sjel_http::Purpose::new("comms-sources"),
        std::time::Duration::from_secs(30),
    )?)
}

fn bounded_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_SOURCE_ITEMS)
}

fn normalized_since(value: Option<&str>) -> &str {
    match value {
        Some("weekly") => "weekly",
        Some("monthly") => "monthly",
        _ => "daily",
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let rest = tag.split_once(&marker)?.1;
    Some(decode_entities(rest.split_once('"')?.0).trim().to_string())
}

fn xml_field(body: &str, tag: &str) -> Option<String> {
    let start = body.find(&format!("<{tag}>"))? + tag.len() + 2;
    let rest = &body[start..];
    let end = rest.find(&format!("</{tag}>"))?;
    Some(
        decode_entities(&rest[..end])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

// Entity decoding is one job with one home (#77). This file's own copy also
// decoded `&amp;` first, so `&amp;lt;` came back as a real `<` — the same
// double-decode the shared version is ordered to avoid.
use crate::extraction::decode_basic_entities as decode_entities;

#[cfg(test)]
mod tests {
    use super::*;

    const REDDIT_ATOM: &str = r#"<feed><title>top</title>
      <entry><author><name>/u/synthetic</name></author>
        <content type="html">&lt;div class="md"&gt;&lt;p&gt;Margins look &amp;amp; feel strong.&lt;/p&gt;&lt;/div&gt;</content>
        <link href="https://www.reddit.com/r/synthsub/comments/abc123/dd_synthex/" />
        <title>[DD] Synthex: why the order book matters</title></entry>
      <entry><author><name>/u/other</name></author>
        <content type="html">&lt;p&gt;meme&lt;/p&gt;</content>
        <link href="https://www.reddit.com/r/synthsub/comments/def456/oddly/" />
        <title>Oddly satisfying chart</title></entry>
      <entry><link href="https://example.org/elsewhere" /><title>DD offsite</title></entry>
    </feed>"#;

    #[test]
    fn a_reddit_feed_delivers_its_posts_and_keeps_only_titled_tokens() {
        let all = parse_reddit_rss(REDDIT_ATOM, "synthsub", &[], 30);
        assert_eq!(
            all.len(),
            2,
            "the offsite link is not a post of the subreddit"
        );
        let first = &all[0];
        assert_eq!(
            first.url,
            "https://www.reddit.com/r/synthsub/comments/abc123/dd_synthex/"
        );
        let content = first.content.as_ref().expect("delivered");
        assert_eq!(content.title, "[DD] Synthex: why the order book matters");
        assert_eq!(content.author.as_deref(), Some("/u/synthetic"));
        assert!(content
            .body
            .contains("<p>Margins look &amp; feel strong.</p>"));

        let dd = parse_reddit_rss(REDDIT_ATOM, "synthsub", &["DD".into()], 30);
        assert_eq!(dd.len(), 1, "\"oddly\" holds dd only inside a word");
    }

    #[test]
    fn a_collector_yields_urls_and_provenance_but_no_content() {
        let html = r#"
          <article class="Box-row">
            <h2><a href="/owner/first">owner / first</a></h2>
            <p class="col-9">A useful repository</p>
            <span>123 stars today</span>
          </article>
        "#;
        let found = parse_github_trending(html, 10, "daily");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://github.com/owner/first");
        assert_eq!(found[0].label.as_deref(), Some("GitHub Trending (daily)"));

        // The card's visible text — the 300-character version of the repo —
        // must not survive into anything the item is built from.
        let carried = format!("{:?}", found[0]);
        assert!(
            !carried.contains("123 stars today") && !carried.contains("A useful repository"),
            "collector leaked card text: {carried}"
        );
    }

    #[test]
    fn parses_bounded_github_trending_cards() {
        let html = r#"
          <article class="Box-row">
            <h2><a href="/owner/first">owner / first</a></h2>
            <p class="col-9">A useful repository</p>
            <span>123 stars today</span>
          </article>
          <article class="Box-row">
            <h2><a href="/other/second">other / second</a></h2>
            <p>Second repository</p>
          </article>
        "#;
        let items = parse_github_trending(html, 1, "daily");
        assert_eq!(items.len(), 1, "the per-run bound still applies");
        assert_eq!(items[0].url, "https://github.com/owner/first");
    }

    #[test]
    fn parses_arxiv_atom_entries() {
        let xml = r#"
          <feed>
            <entry>
              <id>http://arxiv.org/abs/2607.12345v1</id>
              <title> An explainable ranking system </title>
              <summary> The abstract. </summary>
              <author><name>Ada Example</name></author>
              <author><name>Lin Example</name></author>
            </entry>
          </feed>
        "#;
        let items = parse_arxiv_entries(xml, 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://arxiv.org/abs/2607.12345v1");
        assert_eq!(items[0].label.as_deref(), Some("arXiv query"));
        // Authors and abstract come from media::fetch_arxiv, which reads the
        // same identifier — one path builds an arXiv item, not two.
    }

    /// The kinds `item_kind` answers for must be the kinds those adapters' URLs actually land
    /// on, or a re-derivation matches rows to the wrong source. Asserted against
    /// `media::detect` through a real URL of each shape rather than against a copy of the
    /// mapping, because a copy would agree with itself while both drifted from ingest.
    #[test]
    fn declared_kinds_match_what_ingest_stores() {
        for (adapter, url) in [
            ("github-trending", "https://github.com/larsboes/Sjel"),
            ("arxiv", "https://arxiv.org/abs/2501.00001"),
        ] {
            let declared = item_kind(adapter).expect("adapter names a kind");
            assert_eq!(
                declared,
                crate::media::kind_for_url(url),
                "{adapter} claims kind '{declared}', but ingest files {url} elsewhere"
            );
        }
    }

    /// The safety property, and the whole reason a re-derivation can be run at all: a kind no
    /// adapter produces must answer `None`, so a hand-pasted URL — which falls back to `article`,
    /// or to `youtube` for a video — is matched to no declared source and keeps the class it has.
    /// If this ever answers `Some`, a bulk re-derivation would start lowering the class of
    /// whatever the operator pasted, which is the one thing it must never do.
    #[test]
    fn a_kind_no_adapter_produces_matches_nothing() {
        for kind in ["article", "youtube", "instagram", "reddit", "podcast", ""] {
            assert_eq!(item_kind(kind), None, "'{kind}' must match no adapter");
        }
        assert_eq!(
            item_kind("github"),
            None,
            "the KIND is not the ADAPTER name"
        );
    }
}

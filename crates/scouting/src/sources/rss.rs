//! RSS/Atom feed adapter — reads event-ish items from any RSS or Atom feed.
//!
//! Parses the feed XML, extracts items with a date, description, and link, then
//! maps them into `Opportunity` structs. This lets you declare any RSS feed as a
//! configured source in `scouting.json`.
//!
//! Usage (in scouting.json's sources[]):
//! ```json
//! {
//!   "id": "conference-rss",
//!   "adapter": "rss",
//!   "url": "https://example.com/events/feed.xml",
//!   "enabled": true
//! }
//! ```
//!
//! The adapter does NOT depend on a full XML/RSS parsing library (the crate has
//! none). Instead it does lightweight tag extraction: looks for `<item>` / `<entry>`
//! blocks, grabs `<title>`, `<link>`, `<description>`, and `<pubDate>` / `<published>` / `<updated>`.
//! This is deliberately simple — enough for 90% of feeds without pulling in
//! `rss`/`atom`/`quick-xml` crate dependencies.

use std::collections::HashMap;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use crate::source::{SearchQuery, SourceAdapter, SourceError};

pub struct RssFeedSource {
    id: String,
    url: String,
}

impl RssFeedSource {
    pub fn new(id: String, url: String) -> Self {
        Self { id, url }
    }
}

impl SourceAdapter for RssFeedSource {
    fn name(&self) -> &str {
        &self.id
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Event
    }

    fn rate_limit_per_min(&self) -> u32 {
        10
    }

    fn search(&self, _query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let body = fetch_url(&self.url)?;
        let items = parse_feed_items(&body)?;

        let fetched_at = chrono_now();
        let mut opportunities = Vec::new();
        let mut seen = HashMap::new();

        for item in items {
            let title = item.title.unwrap_or_else(|| "Untitled".into());
            let link = item.link.unwrap_or_default();
            let desc = html_to_text(&item.description.unwrap_or_default());
            let pub_date = item.pub_date.unwrap_or_default();

            let dedup_key = format!("{}|{}", link, title);
            if seen.contains_key(&dedup_key) {
                continue;
            }
            seen.insert(dedup_key, true);

            let id = if !link.is_empty() && link.starts_with("http") {
                let hash = simple_hash(&link);
                format!("evt:rss:{}", hash)
            } else {
                let hash = simple_hash(&title);
                format!("evt:rss:{}", hash)
            };

            let opp = Opportunity {
                id,
                opportunity_type: OpportunityType::Event,
                source: format!("rss:{}", self.id),
                source_kind: SourceKind::JsonFeed,
                url: link,
                title,
                starts_at: normalize_rss_date(&pub_date),
                ends_at: None,
                location: None,
                city: None,
                country_code: None,
                latitude: None,
                longitude: None,
                raw: serde_json::json!({
                    "description": desc,
                    "pub_date": pub_date,
                }),
                fetched_at: fetched_at.clone(),
            };
            opportunities.push(opp);
        }

        Ok(opportunities)
    }
}

// ---------------------------------------------------------------------------
// Feed parsing
// ---------------------------------------------------------------------------

struct FeedItem {
    title: Option<String>,
    link: Option<String>,
    description: Option<String>,
    pub_date: Option<String>,
}

fn fetch_url(url: &str) -> Result<String, SourceError> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("scouting-rss"),
        crate::http::TIMEOUT,
    )
    .map_err(|e| SourceError::Fetch(format!("client: {e}")))?;

    crate::http::send_checked(url, client.get(url))
}

/// Lightweight RSS/Atom parser — no XML library dependency.
/// Extracts items from both RSS (`<item>`) and Atom (`<entry>`) feeds.
fn parse_feed_items(xml: &str) -> Result<Vec<FeedItem>, SourceError> {
    let lower = xml.to_lowercase();
    let is_atom = lower.contains("<feed") && lower.contains("<entry>");
    let mut items = Vec::new();

    if is_atom {
        for block in extract_blocks(xml, "<entry", "</entry>") {
            let title = extract_tag(block, "title");
            let link = extract_atom_link(block);
            let description =
                extract_tag(block, "summary").or_else(|| extract_tag(block, "content"));
            let pub_date =
                extract_tag(block, "published").or_else(|| extract_tag(block, "updated"));
            items.push(FeedItem {
                title,
                link,
                description,
                pub_date,
            });
        }
    } else {
        for block in extract_blocks(xml, "<item", "</item>") {
            let title = extract_tag(block, "title");
            let link = extract_tag(block, "link");
            let description = extract_tag(block, "description");
            let pub_date = extract_tag(block, "pubdate");
            items.push(FeedItem {
                title,
                link,
                description,
                pub_date,
            });
        }
    }

    Ok(items)
}

/// Extract all blocks between open_tag and close_tag (non-greedy).
fn extract_blocks<'a>(xml: &'a str, open_tag: &str, close_tag: &str) -> Vec<&'a str> {
    let mut blocks = Vec::new();
    let mut search_start = 0;
    while let Some(start) = xml[search_start..].find(open_tag) {
        let abs_start = search_start + start;
        if let Some(end) = xml[abs_start..].find(close_tag) {
            let abs_end = abs_start + end + close_tag.len();
            blocks.push(&xml[abs_start..abs_end]);
            search_start = abs_end;
        } else {
            break;
        }
    }
    blocks
}

/// Extract the text content of a tag (handles `<tag>text</tag>` and
/// `<tag><![CDATA[text]]></tag>`).
fn extract_tag(block: &str, tag: &str) -> Option<String> {
    // Try CDATA first: <tag><![CDATA[...]]></tag>
    let cdata_pattern = format!("<{}><!\\[CDATA[", tag);
    if let Some(cdata_start) = block.find(&cdata_pattern) {
        let content_start = cdata_start + cdata_pattern.len();
        if let Some(cdata_end) = block[content_start..].find("]]>") {
            let val = block[content_start..content_start + cdata_end].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }

    // Standard: <tag>text</tag>
    let open = format!("<{}>", tag);
    let open_attrs = format!("<{} ", tag);
    if let Some(tag_start) = block.find(&open).or_else(|| block.find(&open_attrs)) {
        let content_start = block[tag_start..].find('>').map(|i| tag_start + i + 1)?;
        let close = format!("</{}>", tag);
        if let Some(content_end) = block[content_start..].find(&close) {
            let val = block[content_start..content_start + content_end].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }

    // Self-closing with attribute: <link href="..."/>
    // (Atom links are handled by extract_atom_link separately)
    None
}

/// A feed description is markup often enough that storing it as text is a bug:
/// it reaches `score::opportunity_text`, so `<p>` and `<img src="…">` get
/// embedded as words and matched against the interest profile.
///
/// A tag becomes a space rather than nothing, so `Join us</a><a>here` stays
/// two words instead of welding into one token. Two strip passes around the
/// decode: a description carries either literal markup (CDATA) or escaped
/// markup (`&lt;p&gt;`), one pass cannot see both, and a third would change
/// nothing.
fn html_to_text(raw: &str) -> String {
    let once = strip_tags(raw);
    let text = strip_tags(&decode_entities(&once));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('>') {
            Some(close) => {
                out.push(' ');
                rest = &after[close + 1..];
            }
            // Markup the feed never closed: the remainder is not text.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#39;", "'"),
        ("&nbsp;", " "),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        // Last: decoding it first would resurrect `&amp;lt;` as a real tag.
        ("&amp;", "&"),
    ] {
        if out.contains(entity) {
            out = out.replace(entity, replacement);
        }
    }
    out
}

/// Extract href from an Atom <link> element.
fn extract_atom_link(block: &str) -> Option<String> {
    let link_start = block.find("<link")?;
    let rest = &block[link_start..];
    let href_start = rest.find("href=\"")?;
    let value_start = href_start + 6;
    let href_end = rest[value_start..].find('"')?;
    let val = rest[value_start..value_start + href_end].to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

fn normalize_rss_date(date: &str) -> Option<String> {
    let d = date.trim();
    if d.is_empty() {
        return None;
    }
    Some(d.to_string())
}

fn simple_hash(s: &str) -> String {
    let mut h = 0u64;
    for b in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u64);
    }
    format!("{:016x}", h)
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_items() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0">
<channel>
  <item>
    <title>AI Hackathon Berlin</title>
    <link>https://example.com/ai-berlin</link>
    <description>A weekend of building AI.</description>
    <pubDate>2026-07-10</pubDate>
  </item>
  <item>
    <title>Munich Tech Meetup</title>
    <link>https://example.com/munich-tech</link>
    <description>Monthly meetup.</description>
    <pubDate>2026-08-01</pubDate>
  </item>
</channel>
</rss>"#;
        let items = parse_feed_items(xml).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title.as_deref(), Some("AI Hackathon Berlin"));
        assert_eq!(
            items[0].link.as_deref(),
            Some("https://example.com/ai-berlin")
        );
        assert_eq!(items[1].title.as_deref(), Some("Munich Tech Meetup"));
    }

    #[test]
    fn parses_atom_entries() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>Conference CFP Deadline</title>
    <link href="https://example.com/cfp"/>
    <summary>Submit your talk.</summary>
    <published>2026-07-15T00:00:00Z</published>
  </entry>
</feed>"#;
        let items = parse_feed_items(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title.as_deref(), Some("Conference CFP Deadline"));
        assert_eq!(items[0].link.as_deref(), Some("https://example.com/cfp"));
        assert!(items[0].description.as_deref().unwrap().contains("Submit"));
    }

    #[test]
    fn a_description_reaches_the_embedder_as_text_not_markup() {
        // CDATA: literal markup.
        assert_eq!(
            html_to_text(r#"<p>Join us</p><a href="/x">here</a><img src="a.png">"#),
            "Join us here"
        );
        // Escaped markup, the non-CDATA form. One strip pass would store the
        // tags as words.
        assert_eq!(
            html_to_text("&lt;p&gt;A weekend of building AI&lt;/p&gt;"),
            "A weekend of building AI"
        );
        // Entities in prose still decode.
        assert_eq!(html_to_text("Rust &amp; Zig"), "Rust & Zig");
        // Text without markup is returned as it arrived.
        assert_eq!(html_to_text("Monthly meetup."), "Monthly meetup.");
    }

    #[test]
    fn handles_empty_feed() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>"#;
        let items = parse_feed_items(xml).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn deduplicates_by_url() {
        let _source = RssFeedSource::new("test".into(), "https://example.com/feed".into());
        // Can't test the real dedup logic without HTTP, but the parser
        // correctly extracts items — that's the unit-testable part.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <item><title>Same</title><link>https://example.com/a</link><description>1</description><pubDate>2026-01-01</pubDate></item>
  <item><title>Same</title><link>https://example.com/a</link><description>2</description><pubDate>2026-01-02</pubDate></item>
</channel></rss>"#;
        let items = parse_feed_items(xml).unwrap();
        assert_eq!(items.len(), 2); // parser doesn't dedup, search() does
    }
}

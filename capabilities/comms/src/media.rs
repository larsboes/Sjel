//! Share-link media ingest: a URL becomes a `FeedItem` with metadata, an
//! optional transcript, and an optional summary. External processes (yt-dlp)
//! are invoked via `std::process::Command` with argument arrays only -- never a
//! shell string. Subtitles download into a temp dir that is always removed (via
//! `TmpDir`'s Drop). No raw audio/video is ever written.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::config::{self, Config};
// `cap` is the extraction stage's cap, so it lives there; the alias keeps the
// name every fetch_* arm already reads by.
use crate::extraction::{
    self, cap as cap_text, collapse_ws, decode_basic_entities, Document, InputClass,
};
use crate::normalize;
use crate::provenance::{StageProvenance, TranscriptSource};
use crate::store::{FeedItem, Store};
use crate::{CommsError, Result};

// One file per concern, the shape `src/store.rs` and `src/server/` already use in
// this capability. What stays here is the dispatcher: which kind of URL this is, the
// door every fetch goes through, and the entry points a caller names.
//
// The glob imports are private and they are what lets a sibling reach a sibling:
// each submodule opens with `use super::*`, so a name re-exported here is in scope
// there. Nothing about the split is visible outside `media` -- everything nameable
// as `media::X` before it is still nameable as `media::X`.
mod arxiv;
mod github;
mod http;
mod huggingface;
mod reddit;
mod summary;
mod ytdlp;

use arxiv::*;
use github::*;
use http::*;
use huggingface::*;
use reddit::*;
use ytdlp::*;

pub use summary::{
    summarize, summarize_item, summarize_pending, summarizer_reachable, summary_producer_revision,
    EnrichmentPass, SummarizeOutcome, SUMMARY_PROMPT_REVISION,
};

/// Transcript length at or above which `content_status` is `full` (not `thin`).
/// One threshold, read by the classifier here and by the evaluator grading a
/// legacy row whose status predates classification.
pub const CONTENT_FULL_THRESHOLD: usize = 1_000;

/// (kind, stream) for a URL. Watch/listen kinds land in `media`; read kinds in
/// `news`. A URL that matches no extractor is an `article`, which is the generic
/// fetch-and-strip path and therefore always a valid fallback.
/// The `kind` half of [`detect`], for a caller that has a URL and no item.
///
/// `sources::item_kind` claims which kind each adapter's URLs land on, and a claim that drifts
/// from this function would match stored rows to the wrong source. Exposing the real answer lets
/// that test assert against ingest instead of against a second copy of the mapping.
///
/// Test-only: production callers already have the item, and reach `detect` through `fetch`.
#[cfg(test)]
pub(crate) fn kind_for_url(url: &str) -> &'static str {
    detect(url).0
}

fn detect(url: &str) -> (&'static str, &'static str) {
    let low = url.to_lowercase();
    if low.contains("youtube.com") || low.contains("youtu.be") {
        ("youtube", "media")
    } else if low.contains("instagram.com") {
        ("instagram", "media")
    } else if parse_github_url(url).is_some() {
        ("github", "news")
    } else if arxiv_id(url).is_some() {
        ("arxiv", "news")
    } else if reddit_permalink(url).is_some() {
        ("reddit", "news")
    } else if parse_huggingface_url(url).is_some() {
        ("huggingface", "news")
    } else if low.ends_with(".mp3") || low.ends_with(".m4a") || low.contains("podcast") {
        ("podcast", "media")
    } else {
        ("article", "news")
    }
}

/// Path segments of a URL, with the scheme, host, query and fragment removed.
/// Empty segments (leading, trailing, doubled slashes) are dropped.
fn path_segments(url: &str) -> Vec<&str> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path = after_scheme.split_once('/').map(|(_, p)| p).unwrap_or("");
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// The seam between the two stages. Extraction hands over its raw output; this
/// keeps it and stores the normalized form beside it. An extractor that came
/// back with nothing stores nothing, rather than an empty string the summarizer
/// would then be handed.
fn finish_extraction(item: &mut FeedItem, raw: Option<String>, source: TranscriptSource) {
    let raw = raw.filter(|r| !r.trim().is_empty());
    item.transcript_source = source.as_str().to_string();
    let (text, status) = match &raw {
        Some(r) => normalized_body(r),
        None => (None, "none".to_string()),
    };
    item.raw_content = raw;
    item.transcript = text;
    item.content_status = status;
}

/// Reject anything that is not plain http(s) before a URL reaches an extractor.
/// `file://` would make yt-dlp and the article fetcher read the local disk, and
/// this runs behind an HTTP endpoint — the check belongs here, at the one door
/// every caller goes through, not at each call site.
fn check_scheme(url: &str) -> Result<()> {
    sjel_http::guard::check_scheme(url).map_err(|refusal| CommsError::Other(refusal.to_string()))
}

/// Refuse a URL that resolves to an address inside this machine or this network.
///
/// The rule, its allowlist escape and the two residuals it does not close are
/// `sjel_http::guard::check_destination`. What is comms' own is why the door is
/// here at all: `POST /ingest` (`server/feed.rs`) takes the URL from the request
/// body, and `server/source_handlers.rs` fetches URLs an external feed produced,
/// so an ingested link would otherwise drive a loopback-bound Axon API from the
/// outside. CodeQL rust/request-forgery reported it against `extract_article`.
/// The allowlist is [`crate::config::ingest_allowed_origins`], empty on every
/// machine but the demo's.
fn check_destination(url: &str) -> Result<()> {
    // `ingest_allowed_origins` as a function value, not a call: the guard runs it
    // only after the address check has already failed, so an ordinary fetch never
    // reads the config file. That laziness was the reason this was a free function
    // rather than a `Config` field, and passing the closure is what preserves it.
    sjel_http::guard::check_destination(url, config::ingest_allowed_origins)
        .map_err(|refusal| CommsError::Other(refusal.to_string()))
}

/// Build a `FeedItem` for a URL: metadata + transcript, no summary. Does NOT
/// persist -- the caller upserts. Never leaves temp files behind.
///
/// Split from `ingest` because summarizing is the slow half (a local model, up
/// to two minutes) while this half is what the caller is waiting to see. The
/// server returns after this and summarizes behind the response; the CLI, which
/// prints the summary, calls `ingest` and waits for both.
pub fn fetch(url: &str) -> Result<FeedItem> {
    check_scheme(url)?;
    // The one place the guard is needed, because this is the one function that
    // makes a request. Every arm below goes out through it, including yt-dlp,
    // which fetches in a subprocess the redirect policy never sees.
    // `extract_article` -- the line CodeQL anchored on -- is private and reached
    // only from here and from the GitHub and HuggingFace arms below, so it is
    // covered by domination; checking it again would mean a second DNS
    // resolution, which widens the rebinding window rather than narrowing it.
    check_destination(url)?;
    let (kind, stream) = detect(url);
    let mut item = FeedItem::new(url, stream, kind);

    // Every arm but arXiv reads the document itself. arXiv is the one source
    // that offers a stand-in when the document cannot be read, so it is the
    // one arm that decides its own answer here (#78).
    let mut source = TranscriptSource::FullText;

    let raw = match kind {
        "youtube" | "instagram" | "podcast" => {
            let meta = ytdlp_meta(url)?;
            item.title = meta.title.clone();
            item.author = author_of(&meta);
            let tmp = TmpDir::new("subs")?;
            ytdlp_transcript(url, tmp.path())
            // tmp dropped here -> subtitles removed.
        }
        "github" => {
            let target = parse_github_url(url).expect("detect() matched parse_github_url");
            let (title, author, text) = fetch_github_target(&target, url)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        "huggingface" => {
            let target =
                parse_huggingface_url(url).expect("detect() matched parse_huggingface_url");
            let (title, author, text, read) = fetch_huggingface(&target, url)?;
            item.title = title;
            item.author = author;
            source = read;
            Some(text)
        }
        "arxiv" => {
            let id = arxiv_id(url).expect("detect() matched arxiv_id");
            let (title, author, text, read) = fetch_arxiv(&id)?;
            item.title = title;
            item.author = author;
            source = read;
            Some(text)
        }
        "reddit" => {
            let permalink = reddit_permalink(url).expect("detect() matched reddit_permalink");
            let (title, author, text) = fetch_reddit(&permalink)?;
            item.title = title;
            item.author = author;
            Some(text)
        }
        _ => {
            // article
            let (title, text) = extract_article(url)?;
            item.title = title;
            Some(text)
        }
    };

    // An extractor that came back with nothing stores nothing, rather than an
    // empty string the summarizer would then be handed.
    finish_extraction(&mut item, raw, source);

    Ok(item)
}

/// Build a `FeedItem` from a URL and optional client-supplied content, title and
/// author. When content is supplied the server-side fetch is bypassed entirely,
/// which is the whole point: a page behind a login is one the operator can hand
/// over and the server must never go fetch itself.
///
/// `client` names who handed it over and is stored as the item's capture
/// provenance (#81). `None` there means this content was fetched, not captured.
pub fn fetch_with_content(
    url: &str,
    content: Option<&str>,
    title: Option<&str>,
    author: Option<&str>,
    client: Option<&str>,
) -> Result<FeedItem> {
    if content.is_none() && title.is_none() && author.is_none() {
        return fetch(url);
    }
    check_scheme(url)?;
    // No `check_destination` here, unlike `fetch`. Past this point the arm makes
    // no request -- the client already handed the document over, which is the
    // whole reason this function exists -- so resolving the host would add a DNS
    // dependency to the one path that is meant to work without the network, and
    // refuse a hand-over made offline. The no-content fall-through above returns
    // through `fetch`, which is guarded.
    let (kind, stream) = detect(url);
    let mut item = FeedItem::new(url, stream, kind);

    // A client hands over either the page's markup or text it already
    // extracted. Both are input classes, so both go through the same trait
    // rather than through a branch that hand-rolls one of them.
    let extracted = content
        .map(|raw| {
            let class = if extraction::looks_like_html(raw) {
                InputClass::Html
            } else {
                InputClass::PlainText
            };
            let out = extraction::require(class)?.extract(&Document {
                class,
                bytes: raw.as_bytes(),
            })?;
            if item.title.is_none() {
                item.title = out.title;
            }
            Ok::<String, CommsError>(out.text)
        })
        .transpose()?;

    if let Some(t) = title {
        if !t.trim().is_empty() {
            item.title = Some(t.trim().to_string());
        }
    }

    if let Some(a) = author {
        if !a.trim().is_empty() {
            item.author = Some(a.trim().to_string());
        }
    }

    // Client-supplied content is the document the client was looking at, so
    // it is full text by definition — there is no source offering a stand-in
    // in this path.
    finish_extraction(&mut item, extracted, TranscriptSource::FullText);

    // Only a body that actually came from the client is that client's capture.
    // A call that supplied a title and nothing else fetched nothing and
    // captured nothing.
    if content.is_some() {
        item.captured_via = client
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| c.chars().take(64).collect());
    }

    Ok(item)
}

/// `fetch` plus a summary. The CLI path.
pub fn ingest(url: &str, cfg: &Config) -> Result<FeedItem> {
    let mut item = fetch(url)?;
    let summary_producer = summary_producer_revision(cfg);
    if let Some(text) = &item.transcript {
        if let SummarizeOutcome::Ok(summary) = summarize(text, cfg, &item.data_class) {
            item.summary = Some(summary);
            item.summary_provenance = summary_producer.map(StageProvenance::model);
        }
    }
    Ok(item)
}

/// The payoff for retaining raw content: re-run normalization over everything
/// stored, without re-fetching a single page. Returns how many items were
/// rewritten and how many carry no raw content to work from (pre-#86 items,
/// which only a re-fetch can fix).
pub fn renormalize_all(store: &Store) -> Result<RenormalizeReport> {
    let ids = store
        .feed_ids_with_raw_content()
        .map_err(|e| CommsError::Other(e.to_string()))?;

    let mut report = RenormalizeReport::default();
    for id in ids {
        let raw = match store
            .get_raw_content(&id)
            .map_err(|e| CommsError::Other(e.to_string()))?
        {
            Some(raw) => raw,
            None => {
                report.skipped += 1;
                continue;
            }
        };

        let (text, status) = normalized_body(&raw);
        store
            .set_normalized(&id, text.as_deref(), &status)
            .map_err(|e| CommsError::Other(e.to_string()))?;
        report.updated += 1;
    }
    Ok(report)
}

/// What a re-normalization pass did. `skipped` is items with no retained raw.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RenormalizeReport {
    pub updated: usize,
    pub skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::summary::{summary_prompt, truncate_for_summary};
    use super::*;

    /// T3, on the second prefill path. This function speaks HTTP itself, so the
    /// class question has to be asked here or not at all — and the role is
    /// pointed at a port nothing listens on, so a prompt that got through would
    /// come back `http_error`. `local_refused` therefore says no request was
    /// attempted, with no network mock to be wrong about.
    /// A light rung pointed at a port nothing listens on.
    ///
    /// The window is wide enough that a probe text is `Rung::Light`, so an
    /// `over_window` verdict cannot be mistaken for a refusal, and the closed
    /// port means anything that does reach the model comes back `http_error`.
    fn unreachable_light_role() -> sjel_inference::InferenceConfig {
        serde_json::from_value(serde_json::json!({
            "backends": {
                "foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:9/v1" },
            },
            "roles": {
                "summarization_light": {
                    "backend": "foundation-models",
                    "model": "a-local-model",
                    "max_input_tokens": 4096,
                },
            },
        }))
        .expect("the probe config is well formed")
    }

    #[test]
    fn media_summarize_refuses_a_c3_item_before_any_request() {
        let cfg = Config::with_inference(unreachable_light_role());
        let text = "paragraph ".repeat(100);

        for refused in ["c3", "vault", "personal", "c4", ""] {
            assert_eq!(
                summarize(&text, &cfg, refused).error_class(),
                crate::digest::LOCAL_REFUSED,
                "{refused} was carried into a prompt"
            );
        }
        // The control: c2 is local-only, not local-forbidden, so it reaches the
        // closed port instead of being refused for its class.
        for allowed in ["c0", "c1", "c2"] {
            assert_ne!(
                summarize(&text, &cfg, allowed).error_class(),
                crate::digest::LOCAL_REFUSED,
                "{allowed} lost its local processing"
            );
        }
    }

    #[test]
    fn detect_kinds() {
        assert_eq!(detect("https://www.youtube.com/watch?v=x").0, "youtube");
        assert_eq!(detect("https://youtu.be/x").0, "youtube");
        assert_eq!(detect("https://www.instagram.com/reel/x").0, "instagram");
        assert_eq!(detect("https://cdn.example.com/ep12.mp3").0, "podcast");
        assert_eq!(
            detect("https://example.com/some-podcast-episode").0,
            "podcast"
        );
        assert_eq!(detect("https://blog.example.com/post").0, "article");
        assert_eq!(detect("https://blog.example.com/post").1, "news");
        assert_eq!(detect("https://youtu.be/x").1, "media");
    }

    #[test]
    fn detect_share_link_kinds() {
        assert_eq!(detect("https://github.com/larsboes/Axon").0, "github");
        assert_eq!(detect("https://github.com/larsboes/Axon").1, "news");
        assert_eq!(detect("https://arxiv.org/abs/2501.12345").0, "arxiv");
        assert_eq!(
            detect("https://www.reddit.com/r/rust/comments/abc123/some_title/").0,
            "reddit"
        );
        // Deeper GitHub paths (issues, PRs, files) are github target extractions.
        assert_eq!(
            detect("https://github.com/larsboes/Axon/issues/7").0,
            "github"
        );
        assert_eq!(detect("https://github.com/larsboes").0, "article");
        // A subreddit listing has no single post to ingest.
        assert_eq!(detect("https://www.reddit.com/r/rust/").0, "article");
    }

    #[test]
    fn github_repo_parses_root_paths_only() {
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon"),
            Some(("larsboes".into(), "Axon".into()))
        );
        // Trailing slash, query and .git suffix are all the same repo.
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon.git?tab=readme"),
            Some(("larsboes".into(), "Axon".into()))
        );
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon/"),
            Some(("larsboes".into(), "Axon".into()))
        );
        assert_eq!(
            github_repo("https://github.com/larsboes/Axon/blob/main/README.md"),
            None
        );
        assert_eq!(github_repo("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn arxiv_id_covers_abs_pdf_and_legacy_ids() {
        assert_eq!(
            arxiv_id("https://arxiv.org/abs/2501.12345").as_deref(),
            Some("2501.12345")
        );
        // The version is part of the identity and survives.
        assert_eq!(
            arxiv_id("https://arxiv.org/pdf/2501.12345v2").as_deref(),
            Some("2501.12345v2")
        );
        assert_eq!(
            arxiv_id("https://arxiv.org/pdf/2501.12345v2.pdf").as_deref(),
            Some("2501.12345v2")
        );
        // Legacy archive-prefixed ids keep their slash.
        assert_eq!(
            arxiv_id("https://arxiv.org/abs/cs/0112017").as_deref(),
            Some("cs/0112017")
        );
        assert_eq!(arxiv_id("https://arxiv.org/list/cs.AI/recent"), None);
        assert_eq!(arxiv_id("https://example.com/abs/2501.12345"), None);
    }

    #[test]
    fn reddit_permalink_canonicalizes_post_urls() {
        assert_eq!(
            reddit_permalink("https://www.reddit.com/r/rust/comments/abc123/some_title/")
                .as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc123")
        );
        // old.reddit.com and a slugless permalink resolve to the same canonical form.
        assert_eq!(
            reddit_permalink("https://old.reddit.com/r/rust/comments/abc123").as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc123")
        );
        assert_eq!(reddit_permalink("https://www.reddit.com/r/rust/"), None);
        assert_eq!(
            reddit_permalink("https://www.reddit.com/user/someone"),
            None
        );
    }

    #[test]
    fn xml_field_reads_first_match_decoded() {
        let entry =
            "<entry><title>A &amp; B\n  study</title><summary>  abstract text </summary></entry>";
        assert_eq!(xml_field(entry, "title").as_deref(), Some("A & B study"));
        assert_eq!(
            xml_field(entry, "summary").as_deref(),
            Some("abstract text")
        );
        assert_eq!(xml_field(entry, "author"), None);
    }

    #[test]
    fn ytdlp_args_carry_impersonate_when_requested() {
        let meta = ytdlp_meta_args("https://youtu.be/x", true);
        assert!(
            meta.windows(2).any(|w| w == ["--impersonate", "chrome"]),
            "meta args must include --impersonate chrome: {meta:?}"
        );
        assert_eq!(meta.last().unwrap(), "https://youtu.be/x", "url is last");

        let subs = ytdlp_sub_args("https://youtu.be/x", std::path::Path::new("/tmp/d"), true);
        assert!(subs.windows(2).any(|w| w == ["--impersonate", "chrome"]));
        assert!(subs
            .windows(2)
            .any(|w| w == ["--sub-langs", "en,en-orig,de"]));
        assert_eq!(subs.last().unwrap(), "https://youtu.be/x");
    }

    #[test]
    fn ytdlp_args_omit_impersonate_on_fallback() {
        let meta = ytdlp_meta_args("https://youtu.be/x", false);
        assert!(
            !meta.iter().any(|a| a == "--impersonate"),
            "fallback drops the flag"
        );
        let subs = ytdlp_sub_args("https://youtu.be/x", std::path::Path::new("/tmp/d"), false);
        assert!(!subs.iter().any(|a| a == "--impersonate"));
    }

    #[test]
    fn truncate_for_summary_marks_only_when_cut() {
        assert_eq!(truncate_for_summary("short", 15_000), "short");
        let long = "a".repeat(20_000);
        let out = truncate_for_summary(&long, 15_000);
        assert!(out.ends_with("…[truncated]"), "cut text carries the marker");
        assert_eq!(out.chars().count(), 15_000 + "…[truncated]".chars().count());
    }

    #[test]
    fn summary_prompt_requires_english_output() {
        let prompt = summary_prompt("Ein deutschsprachiger Quelltext.");
        assert!(prompt.contains("Write in English"));
        assert!(prompt.contains("Content:\nEin deutschsprachiger Quelltext."));
    }

    #[test]
    fn parse_vtt_strips_and_dedupes() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n1\n00:00:00.000 --> 00:00:02.000\nHello <c>world</c>\n\n2\n00:00:02.000 --> 00:00:04.000\nHello world\nHello world\nsecond line\n";
        let out = parse_vtt(vtt);
        assert_eq!(out, "Hello world\nsecond line");
    }

    #[test]
    fn strip_tags_removes_spans() {
        assert_eq!(strip_tags("a <b>bold</b> c"), "a bold c");
        assert_eq!(strip_tags("<00:00:01.000><c> hi </c>"), " hi ");
    }

    #[test]
    fn an_item_records_which_of_the_two_it_read() {
        // #78 asks for this as a field rather than as an `Abstract:` prefix
        // inside the text: a prefix has to be parsed back out by every reader,
        // and the embedder would score it as if the paper had said it.
        let mut abstract_only = FeedItem::new("https://arxiv.org/abs/1", "news", "arxiv");
        finish_extraction(
            &mut abstract_only,
            Some("We show that transit feeds drift.".into()),
            TranscriptSource::Abstract,
        );
        assert_eq!(abstract_only.transcript_source, "abstract");

        let mut paper = FeedItem::new("https://arxiv.org/abs/2", "news", "arxiv");
        finish_extraction(
            &mut paper,
            Some("1 Introduction. The full paper.".into()),
            TranscriptSource::FullText,
        );
        assert_eq!(paper.transcript_source, "full-text");

        // Independent of content_status, which measures length: both of these
        // are `thin`, and they are not the same thing.
        assert_eq!(abstract_only.content_status, "thin");
        assert_eq!(paper.content_status, "thin");
    }

    #[test]
    fn the_arxiv_chain_has_a_reader_at_every_rung() {
        // The three rungs of `arxiv_full_text`, asserted as a set rather than
        // by walking them: the two HTML hosts share one reader, and the PDF
        // rung stopped being unreachable when xberg registered (#77).
        assert_eq!(ARXIV_HTML_HOSTS.len(), 2);
        assert!(ARXIV_HTML_HOSTS[0].contains("arxiv.org"));
        assert!(ARXIV_HTML_HOSTS[1].contains("ar5iv"));
        assert!(extraction::for_class(InputClass::Html).is_some());
        assert!(
            extraction::for_class(InputClass::Pdf).is_some(),
            "no PDF reader: papers with no LaTeX source would silently store \
             their abstract, which is the state #78 existed to end"
        );
    }

    #[test]
    fn fetch_with_content_bypasses_fetch_and_sets_status() {
        let url = "https://example.com/protected";
        let markdown = "# Protected Document\n\nThis is supplied content bypass.";
        let item = fetch_with_content(
            url,
            Some(markdown),
            Some("Supplied Title"),
            Some("Supplied Author"),
            Some("axon-clip"),
        )
        .unwrap();

        assert_eq!(item.url, url);
        assert_eq!(item.title.as_deref(), Some("Supplied Title"));
        assert_eq!(item.author.as_deref(), Some("Supplied Author"));
        assert_eq!(item.transcript.as_deref(), Some(markdown));
        assert_eq!(item.content_status, "thin");

        // Long content gets 'full' status
        let long_md = "a".repeat(1200);
        let item_full = fetch_with_content(url, Some(&long_md), None, None, None).unwrap();
        assert_eq!(item_full.content_status, "full");
    }

    #[test]
    fn a_captured_item_says_which_client_handed_it_over() {
        let url = "https://example.com/behind-a-login";
        let body = "# Members only\n\nThe part the server could never fetch.";

        let captured = fetch_with_content(url, Some(body), None, None, Some("axon-clip")).unwrap();
        assert_eq!(captured.captured_via.as_deref(), Some("axon-clip"));

        // Handed over by something that did not name itself: still a capture,
        // but there is nothing truthful to record about who.
        let anonymous = fetch_with_content(url, Some(body), None, None, None).unwrap();
        assert_eq!(anonymous.captured_via, None);

        // A client name on a call that supplied no body fetched nothing and
        // captured nothing, so it must not be labelled a capture.
        let title_only =
            fetch_with_content(url, None, Some("Just a title"), None, Some("axon-clip")).unwrap();
        assert_eq!(title_only.captured_via, None);
    }

    #[test]
    fn arxiv_id_extraction_and_pdf_url_mapping() {
        let abs_url = "https://arxiv.org/abs/2501.12345v1";
        let id = arxiv_id(abs_url).unwrap();
        assert_eq!(id, "2501.12345v1");

        let pdf_url = format!("https://arxiv.org/pdf/{id}.pdf");
        assert_eq!(pdf_url, "https://arxiv.org/pdf/2501.12345v1.pdf");
    }

    #[test]
    fn parse_github_deep_paths() {
        let repo_url = "https://github.com/larsboes/Axon";
        assert_eq!(
            parse_github_url(repo_url),
            Some(GitHubTarget::Repo {
                owner: "larsboes".into(),
                repo: "Axon".into()
            })
        );

        let issue_url = "https://github.com/larsboes/Axon/issues/80";
        assert_eq!(
            parse_github_url(issue_url),
            Some(GitHubTarget::Issue {
                owner: "larsboes".into(),
                repo: "Axon".into(),
                number: 80
            })
        );

        let blob_url = "https://github.com/larsboes/Axon/blob/main/capabilities/comms/src/media.rs";
        assert_eq!(
            parse_github_url(blob_url),
            Some(GitHubTarget::Blob {
                owner: "larsboes".into(),
                repo: "Axon".into(),
                branch: "main".into(),
                path: "capabilities/comms/src/media.rs".into()
            })
        );
    }

    #[test]
    fn parse_huggingface_urls() {
        let model_url = "https://huggingface.co/mlx-community/multilingual-e5-base-mlx";
        assert_eq!(
            parse_huggingface_url(model_url),
            Some(HuggingFaceTarget::Model {
                model_id: "mlx-community/multilingual-e5-base-mlx".into()
            })
        );

        let dataset_url = "https://huggingface.co/datasets/glue";
        assert_eq!(
            parse_huggingface_url(dataset_url),
            Some(HuggingFaceTarget::Dataset {
                dataset_id: "glue".into()
            })
        );

        let paper_url = "https://huggingface.co/papers/2501.12345";
        assert_eq!(
            parse_huggingface_url(paper_url),
            Some(HuggingFaceTarget::Paper {
                paper_id: "2501.12345".into()
            })
        );
    }

    #[test]
    fn extraction_and_normalization_keep_separate_outputs() {
        let raw = "Menu\nWe use cookies. Accept all\n\nThe actual article body.\n";
        let mut item = FeedItem::new("https://example.com/a", "news", "article");
        finish_extraction(&mut item, Some(raw.to_string()), TranscriptSource::FullText);

        assert_eq!(
            item.raw_content.as_deref(),
            Some(raw),
            "the extractor's output is retained verbatim"
        );
        assert_eq!(item.transcript.as_deref(), Some("The actual article body."));
    }

    #[test]
    fn content_status_follows_the_normalized_text_not_the_raw_bytes() {
        // Raw clears the 1k threshold; almost all of it is boilerplate, so the
        // normalized body does not. Deriving from raw would call this `full`.
        let boilerplate = "Share on Twitter\n".repeat(80);
        let raw = format!("{boilerplate}Two short sentences of actual content.");
        assert!(raw.chars().count() >= CONTENT_FULL_THRESHOLD);

        let mut item = FeedItem::new("https://example.com/b", "news", "article");
        finish_extraction(&mut item, Some(raw), TranscriptSource::FullText);
        assert_eq!(item.content_status, "thin");
    }

    #[test]
    fn an_item_that_is_all_boilerplate_stores_no_transcript() {
        let mut item = FeedItem::new("https://example.com/c", "news", "article");
        finish_extraction(
            &mut item,
            Some("Menu\nHome\nCopy link\n".to_string()),
            TranscriptSource::FullText,
        );
        assert_eq!(item.transcript, None);
        assert_eq!(item.content_status, "none");
        assert!(
            item.raw_content.is_some(),
            "raw is kept even when all of it is dropped"
        );
    }

    /// The tests that need a real store. Own module because the module name is
    /// what CI splits on — see `cloud_run`'s and `digest`'s.
    #[cfg(test)]
    mod db_tests {
        use super::*;

        fn stored_feed(store: &crate::store::Store, data_class: &str) -> String {
            let mut item = FeedItem::new(
                &format!("https://example.com/axon-t3-summary-{data_class}"),
                "news",
                "article",
            );
            item.transcript = Some("paragraph ".repeat(100));
            item.data_class = data_class.into();
            store.upsert_feed(&item).expect("the source row is stored");
            item.id
        }

        /// A class verdict is not a failed attempt, and the enrichment drain has
        /// to say so on the row itself.
        ///
        /// `summary_attempts` is what `feed_pending_summaries` filters on: three
        /// drain ticks against a `< 3` bound park the row until the producer
        /// revision moves, so counting a refusal here would leave a `c3` item
        /// later downgraded to `c1` permanently un-summarized. The count is also
        /// printed to the operator as "Summary retries", for a request that was
        /// never made.
        ///
        /// The `c2` control is what makes the assertion mean something: it takes
        /// the same path, reaches the closed port and *does* move the ledger, so
        /// this cannot pass on a drain that stopped recording attempts at all.
        #[test]
        fn a_refused_class_moves_no_retry_ledger_through_the_drain() {
            let store = crate::store::db_tests::open_test_store("media_c3_no_retry");
            let mut cfg = Config::with_inference(unreachable_light_role());
            cfg.database_path = crate::store::db_tests::test_database("media_c3_no_retry_gate");
            let text = "paragraph ".repeat(100);
            assert_eq!(
                summarize(&text, &cfg, "c3").error_class(),
                crate::digest::LOCAL_REFUSED,
                "the outcome under test is not the refusal"
            );

            let refused = stored_feed(&store, "c3");
            assert!(
                !summarize_item(&store, &cfg, &refused).expect("the drain answers"),
                "a refused item reported a summary"
            );
            let row = store
                .get_feed(&refused)
                .expect("the row reads back")
                .expect("the row exists");
            assert_eq!(row.summary, None, "a refused class produced a summary");
            assert_eq!(row.summary_attempts, 0, "a verdict is not a failed attempt");
            assert_eq!(row.summary_last_error, None);
            assert_eq!(row.summary_next_attempt, None, "a verdict has no backoff");

            let attempted = stored_feed(&store, "c2");
            assert!(!summarize_item(&store, &cfg, &attempted).expect("the drain answers"));
            let control = store
                .get_feed(&attempted)
                .expect("the row reads back")
                .expect("the row exists");
            assert_eq!(
                control.summary_attempts, 1,
                "c2 reached the closed port, so the ledger must have moved"
            );
        }
    }
}

//! The Reddit adapter: a post's own text plus the comments worth quoting.

use super::*;

/// Canonical `www.reddit.com` comments permalink for a post URL. Only a post
/// (`/r/<sub>/comments/<id>/...`) qualifies — a subreddit listing or a user page
/// has no single piece of content to ingest.
pub(super) fn reddit_permalink(url: &str) -> Option<String> {
    let host = host_of(url);
    if !(host.ends_with("reddit.com") || host == "redd.it") {
        return None;
    }
    let seg = path_segments(url);
    let idx = seg.iter().position(|s| *s == "comments")?;
    // /r/<sub>/comments/<id> — the id segment must exist.
    let id = seg.get(idx + 1)?;
    let sub = seg.get(idx.checked_sub(1)?)?;
    Some(format!("https://www.reddit.com/r/{sub}/comments/{id}"))
}

/// A Reddit post via the `.json` view of its permalink: the selftext plus the
/// top-level comments, which is where the substance usually is. Link posts have
/// an empty selftext and still ingest — the comments carry them.
///
/// Verified 2026-07-28: Reddit answers 403 to this endpoint for every
/// unauthenticated caller — www and old, descriptive UA, API-format UA and a
/// browser UA alike. The parsing below is against the shape the endpoint still
/// returns for an authorized caller; making it reachable needs a registered
/// Reddit app and an OAuth token against `oauth.reddit.com`, which is a secret
/// only the operator provisions (CONTRIBUTING.md#secrets). Until then a Reddit paste
/// fails with the message below rather than silently landing as an empty item.
pub(super) fn fetch_reddit(permalink: &str) -> Result<(Option<String>, Option<String>, String)> {
    let http = http_client()?;
    let body = get_json(&http, &format!("{permalink}.json?raw_json=1&limit=30"), "application/json")
        .map_err(|e| {
            CommsError::Other(format!(
                "{e} — Reddit blockt unauthentifizierte Zugriffe; braucht eine registrierte App + OAuth-Token"
            ))
        })?;

    // [0] is the post listing, [1] the comment listing.
    let post = body
        .get(0)
        .and_then(|l| l.get("data"))
        .and_then(|d| d.get("children"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("data"))
        .ok_or_else(|| CommsError::Other("reddit: no post in listing".into()))?;

    let title = post
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let author = post
        .get("author")
        .and_then(|v| v.as_str())
        .map(|a| format!("u/{a}"));

    let mut text = String::new();
    if let Some(selftext) = post.get("selftext").and_then(|v| v.as_str()) {
        text.push_str(selftext.trim());
    }
    // A link post points somewhere; the target URL is part of the content.
    if let Some(link) = post.get("url_overridden_by_dest").and_then(|v| v.as_str()) {
        text.push_str(&format!("\n\nVerlinkt: {link}"));
    }

    let comments = body
        .get(1)
        .and_then(|l| l.get("data"))
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for c in comments {
        let d = match c.get("data") {
            Some(d) => d,
            None => continue,
        };
        let author = d.get("author").and_then(|v| v.as_str()).unwrap_or("?");
        let comment = d.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
        if comment.is_empty() {
            continue;
        }
        let score = d.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
        text.push_str(&format!("\n\n--- u/{author} ({score}) ---\n{comment}"));
    }

    Ok((title, author, cap_text(text.trim().to_string())))
}

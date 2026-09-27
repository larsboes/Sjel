//! The shared client for this capability's fetches, and the plain-article path.
//!
//! `http_client` is where comms' redirect policy lives: every hop is re-checked
//! through the guard, because checking only the URL the caller handed over leaves
//! `302 -> http://169.254.169.254/` as a complete bypass.

use super::*;

/// Remove `<...>` spans (VTT/HTML inline tags) from a line.
pub(super) fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The one HTTP client every extractor uses. A descriptive user agent is not
/// politeness here — Reddit and the GitHub API both reject the default one.
///
/// The redirect policy is the second half of `check_destination`: checking only
/// the URL the caller handed over leaves `302 -> http://169.254.169.254/` as a
/// complete bypass, so every hop is re-checked and the chain is capped at three.
pub(super) fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(sjel_http::builder(
        sjel_http::Purpose::new("comms-media"),
        std::time::Duration::from_secs(30),
    )
    .gzip(true)
    .redirect(reqwest::redirect::Policy::custom(|attempt| {
        // `>`, not `>=`: reqwest's `previous()` starts with the initial
        // URL, which is not a redirection (reqwest-0.12.28
        // src/redirect.rs:135 says so, and its own `Policy::limited`
        // compares the same way). With `>=` the error said three and
        // followed two.
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(CommsError::Other(format!(
                "refused: more than {MAX_REDIRECTS} redirects"
            )));
        }
        let next = attempt.url().as_str().to_string();
        match check_scheme(&next).and_then(|()| check_destination(&next)) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(e),
        }
    }))
    .build()?)
}

/// Fetch an article: GET the bytes, hand them to the HTML extractor.
///
/// The split is the point. This function owns the protocol — the client, the
/// status check — and owns no opinion about turning markup into text; that
/// lives behind `extraction::Extractor`, one implementation per input class
/// (#77). Before the trait it also hand-rolled the HTML, which is how the same
/// stripper ended up written twice with two different bugs.
pub(super) fn extract_article(url: &str) -> Result<(Option<String>, String)> {
    let http = http_client()?;
    let resp = http.get(url).send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "article fetch HTTP {}",
            resp.status()
        )));
    }
    let html = resp.text()?;
    let out = extraction::require(InputClass::Html)?.extract(&Document::html(html.as_bytes()))?;
    Ok((out.title, out.text))
}

/// Raw extraction output to canonical body plus its status, in one place, so
/// an ingest and a later re-normalization pass cannot disagree about what an
/// item says. The status is derived from the *normalized* text: a page that is
/// 90% cookie banner is `thin`, whatever its raw length said.
pub(super) fn normalized_body(raw: &str) -> (Option<String>, String) {
    let text = normalize::normalize(raw).text;
    let text = Some(text).filter(|t| !t.trim().is_empty());
    let status = match &text {
        None => "none",
        Some(t) if t.chars().count() >= CONTENT_FULL_THRESHOLD => "full",
        Some(_) => "thin",
    };
    (text, status.to_string())
}

pub(super) fn get_json(
    http: &reqwest::blocking::Client,
    url: &str,
    accept: &str,
) -> Result<serde_json::Value> {
    let resp = http.get(url).header("Accept", accept).send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "{url} -> HTTP {}",
            resp.status()
        )));
    }
    Ok(resp.json()?)
}

/// How many hops `http_client`'s redirect policy will follow. Three is enough
/// for the shorteners and the http->https->canonical-host chains real articles
/// use, and short enough that a redirect loop fails fast.
pub(super) const MAX_REDIRECTS: usize = 3;

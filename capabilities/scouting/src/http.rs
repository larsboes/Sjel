//! The one place an adapter turns an HTTP response into a body or an error.
//!
//! `reqwest::blocking::get(url).text()` hands back the body whatever the status
//! was, so an error page goes straight to serde and surfaces as a parse failure
//! about the wrong thing. Luma's real 404 body — `{"message":"Sorry, we could
//! not find what you were looking for."}` — was reported as `missing field
//! \`entries\``, which reads as upstream schema drift and sent that
//! investigation the wrong way entirely (#54, then #62 for the three siblings).
//!
//! Adapters keep building their own requests, because their headers genuinely
//! differ; what they must not keep is a private opinion about what a 404 means.
//! Classification is separated from the send so the failure path is testable
//! without a network or a fixture server.

use crate::source::SourceError;

/// How much of an error body to quote back. Enough to recognize an upstream
/// error page, short enough not to dump an HTML document into a log line.
const SNIPPET_CHARS: usize = 200;

/// How long any scouting fetch waits before it gives up.
///
/// One value for the whole capability, because the adapters all do the same
/// thing: one GET of one page from a public events site. 30 seconds is what
/// `sources/rss.rs` already used and the only timeout any of them carried --
/// the other five built a client with none, so one unresponsive host could hold
/// a scouting run open for as long as the socket stayed alive.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Decide what a completed response means. Pure: no I/O, no clock.
pub fn classify(
    url: &str,
    status: reqwest::StatusCode,
    body: String,
) -> Result<String, SourceError> {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(SourceError::RateLimited);
    }
    if !status.is_success() {
        let snippet: String = body.chars().take(SNIPPET_CHARS).collect();
        return Err(SourceError::Fetch(format!(
            "GET {url}: HTTP {status}: {snippet}"
        )));
    }
    Ok(body)
}

/// Send a request the caller has already shaped (headers, user agent, query)
/// and return its body only if the status says the body is the answer.
pub fn send_checked(
    url: &str,
    request: reqwest::blocking::RequestBuilder,
) -> Result<String, SourceError> {
    let resp = request
        .send()
        .map_err(|e| SourceError::Fetch(format!("GET {url}: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| SourceError::Fetch(format!("GET {url}: body: {e}")))?;
    classify(url, status, body)
}

/// Plain checked GET, for adapters that need no special headers.
pub fn get_checked(url: &str) -> Result<String, SourceError> {
    // 30s, the timeout src/sources/rss.rs already used for the same job. This
    // built a client with no timeout at all, so one unresponsive host held a
    // scouting run open indefinitely.
    let client = sjel_http::client(sjel_http::Purpose::new("scouting-fetch"), TIMEOUT)
        .map_err(|e| SourceError::Fetch(format!("client build: {e}")))?;
    send_checked(url, client.get(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn a_success_body_passes_through_untouched() {
        let body = classify("https://example.test/x", StatusCode::OK, "{\"a\":1}".into())
            .expect("2xx is the answer");
        assert_eq!(body, "{\"a\":1}");
    }

    /// The regression: a 404 body must not reach the caller's parser. The error
    /// has to name the status, or the next person reads a serde message and goes
    /// looking for schema drift that isn't there.
    #[test]
    fn an_error_body_becomes_a_fetch_error_naming_the_status() {
        let luma_404 = "{\"message\":\"Sorry, we could not find what you were looking for.\"}";
        let err = classify(
            "https://example.test/x",
            StatusCode::NOT_FOUND,
            luma_404.into(),
        )
        .expect_err("404 is not an answer");

        match err {
            SourceError::Fetch(msg) => {
                assert!(msg.contains("404"), "status must be named: {msg}");
                assert!(msg.contains("could not find"), "body must be quoted: {msg}");
            }
            other => panic!("expected Fetch, got {other:?}"),
        }
    }

    #[test]
    fn a_server_error_is_a_fetch_error_too() {
        let err = classify(
            "https://example.test/x",
            StatusCode::BAD_GATEWAY,
            "<html>502</html>".into(),
        )
        .expect_err("5xx is not an answer");
        assert!(matches!(err, SourceError::Fetch(_)));
    }

    /// 429 is its own variant so a caller can back off rather than treat it as
    /// a dead source.
    #[test]
    fn too_many_requests_is_rate_limited_not_fetch() {
        let err = classify(
            "https://example.test/x",
            StatusCode::TOO_MANY_REQUESTS,
            "slow down".into(),
        )
        .expect_err("429 is not an answer");
        assert!(matches!(err, SourceError::RateLimited), "got {err:?}");
    }

    /// A long error page must not be pasted whole into a log line.
    #[test]
    fn an_error_body_is_truncated() {
        let huge = "x".repeat(5_000);
        let err = classify("https://example.test/x", StatusCode::NOT_FOUND, huge)
            .expect_err("404 is not an answer");
        match err {
            SourceError::Fetch(msg) => assert!(
                msg.len() < SNIPPET_CHARS + 100,
                "snippet should be bounded, got {} chars",
                msg.len()
            ),
            other => panic!("expected Fetch, got {other:?}"),
        }
    }
}

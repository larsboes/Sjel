//! Web access extension (F7, Wave 2): read-only web fetching and search.
//!
//! Exposes two tools:
//! - `web_fetch`: fetches an HTTP(S) URL and extracts its content (HTML to structured text
//!   via `sjel-extraction`, capped at 20,000 characters).
//! - `web_search`: queries DuckDuckGo HTML search and returns parsed title, URL and snippet links.
//!
//! Enforces outbound request safety via `sjel_http::guard`:
//! - Scheme must be `http` or `https` (closes `file://` disk access).
//! - Target destination cannot resolve to loopback or private networks (closes SSRF).
//! - Sub-second cancellation via atomic stop flag checking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use crate::{Extension, Tool};
use sjel_extraction::{Builtin, Document, Extractor};
use sjel_http::guard::{check_destination, check_scheme};
use sjel_http::Purpose;

/// The name `agent.toml` and `--ext` use.
pub const NAME: &str = "web";

/// Default timeout in seconds for web requests.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const MAX_TIMEOUT_SECS: u64 = 120;

/// DuckDuckGo HTML search endpoint.
const DDG_SEARCH_URL: &str = "https://html.duckduckgo.com/html/";

pub struct Web {
    stop: Arc<AtomicBool>,
    timeout_secs: u64,
}

impl Web {
    pub fn new(stop: &Arc<AtomicBool>, timeout_secs: Option<u64>) -> Self {
        let timeout_secs = timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        Self {
            stop: Arc::clone(stop),
            timeout_secs,
        }
    }
}

impl Extension for Web {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(WebFetch {
                stop: Arc::clone(&self.stop),
                timeout_secs: self.timeout_secs,
            }),
            Box::new(WebSearch {
                stop: Arc::clone(&self.stop),
                timeout_secs: self.timeout_secs,
            }),
        ]
    }
}

/// Tool: fetch a web page and extract readable text.
pub struct WebFetch {
    stop: Arc<AtomicBool>,
    timeout_secs: u64,
}

impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch an http(s) URL and extract its readable content. Returns page title and text. \
         Blocked from internal or loopback addresses."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The http:// or https:// URL to fetch."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default 30, max 120)."
                }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "`url` is required".to_owned())?;

        let timeout_secs = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(self.timeout_secs)
            .clamp(1, MAX_TIMEOUT_SECS);

        // Security guards
        check_scheme(url).map_err(|e| format!("invalid URL scheme: {e}"))?;
        check_destination(url, Vec::new).map_err(|e| format!("destination refused: {e}"))?;

        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let timeout = Duration::from_secs(timeout_secs);
        let client = sjel_http::client(Purpose::new("agent-web-fetch"), timeout)
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let response = client
            .get(url)
            .send()
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP error {status} fetching {url}"));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|val| val.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let bytes = response
            .bytes()
            .map_err(|e| format!("failed to read response body: {e}"))?;

        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let is_html = content_type.contains("text/html")
            || sjel_extraction::looks_like_html(&String::from_utf8_lossy(&bytes));

        let doc = if is_html {
            Document::html(&bytes)
        } else {
            Document::text(&bytes)
        };

        let extraction = Builtin
            .extract(&doc)
            .map_err(|e| format!("extraction failed: {e}"))?;

        let mut out = String::new();
        if let Some(title) = extraction.title {
            out.push_str(&format!("Title: {title}\n\n"));
        }
        out.push_str(&extraction.text);
        Ok(out)
    }
}

/// Tool: search the web via DuckDuckGo.
pub struct WebSearch {
    stop: Arc<AtomicBool>,
    timeout_secs: u64,
}

impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web using DuckDuckGo. Returns source-linked search result titles, URLs, and snippets."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                },
                "num_results": {
                    "type": "integer",
                    "description": "Number of results to return (default: 5, max: 20)."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default 30, max 120)."
                }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "`query` is required".to_owned())?;

        let num_results = args
            .get("num_results")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 20) as usize;

        let timeout_secs = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(self.timeout_secs)
            .clamp(1, MAX_TIMEOUT_SECS);

        check_scheme(DDG_SEARCH_URL).map_err(|e| format!("invalid URL scheme: {e}"))?;
        check_destination(DDG_SEARCH_URL, Vec::new)
            .map_err(|e| format!("destination refused: {e}"))?;

        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let timeout = Duration::from_secs(timeout_secs);
        let client = sjel_http::client(Purpose::new("agent-web-search"), timeout)
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let search_url = format!("{DDG_SEARCH_URL}?q={}", percent_encode(query));
        let response = client
            .get(&search_url)
            .header(reqwest::header::ACCEPT, "text/html")
            .send()
            .map_err(|e| format!("search request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("search service returned HTTP {status}"));
        }

        let html = response
            .text()
            .map_err(|e| format!("failed to read search response: {e}"))?;

        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }

        let results = parse_ddg_html(&html, num_results);
        if results.is_empty() {
            return Ok("No results found.".to_owned());
        }

        let mut out = String::new();
        for (i, res) in results.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(&format!("{}. {}\nURL: {}", i + 1, res.title, res.url));
            if !res.snippet.is_empty() {
                out.push_str(&format!("\nSnippet: {}", res.snippet));
            }
        }
        Ok(out)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Parses DuckDuckGo HTML results without heavy external DOM dependencies.
/// DuckDuckGo's HTML page marks search results with `<a class="result__url" href="...uddg=...">`
/// or `<a class="result__snippet" ...>` or result link `<a class="result__a" href="...">`.
pub fn parse_ddg_html(html: &str, limit: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut rest = html;

    // Split by result block: class="result " or class="result__body"
    while let Some(pos) = rest.find("result__body") {
        let after_marker = &rest[pos + 12..];
        let (block, next) = if let Some(next_pos) = after_marker.find("result__body") {
            (&rest[pos..pos + 12 + next_pos], &after_marker[next_pos..])
        } else {
            (&rest[pos..], "")
        };
        rest = next;

        // Skip ads
        if block.contains("result--ad") {
            continue;
        }

        // Title and URL from class="result__a" href="..."
        if let Some(a_pos) = block.find("result__a") {
            let anchor_chunk = &block[..a_pos];
            let href_start = anchor_chunk.rfind("href=\"");
            let href = if let Some(hs) = href_start {
                let href_content = &anchor_chunk[hs + 6..];
                href_content.find('"').map(|he| &href_content[..he])
            } else {
                // href might be after result__a
                let after = &block[a_pos..];
                after.find("href=\"").and_then(|hs| {
                    let href_content = &after[hs + 6..];
                    href_content.find('"').map(|he| &href_content[..he])
                })
            };

            let title_chunk = &block[a_pos..];
            let title_content = title_chunk.find('>').and_then(|gt| {
                let content = &title_chunk[gt + 1..];
                content.find("</a>").map(|end_a| &content[..end_a])
            });

            if let (Some(raw_href), Some(raw_title)) = (href, title_content) {
                let clean_title = sjel_extraction::collapse_ws(
                    &sjel_extraction::decode_basic_entities(&strip_tags(raw_title)),
                );
                let clean_url = extract_ddg_target_url(raw_href);

                // Snippet from class="result__snippet"
                let snippet = if let Some(snip_pos) = block.find("result__snippet") {
                    let snip_chunk = &block[snip_pos..];
                    snip_chunk
                        .find('>')
                        .and_then(|gt| {
                            let content = &snip_chunk[gt + 1..];
                            content
                                .find("</a>")
                                .or_else(|| content.find("</td>"))
                                .map(|end| &content[..end])
                        })
                        .map(|raw_snip| {
                            sjel_extraction::collapse_ws(&sjel_extraction::decode_basic_entities(
                                &strip_tags(raw_snip),
                            ))
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                if !clean_title.is_empty() && !clean_url.is_empty() {
                    hits.push(SearchHit {
                        title: clean_title,
                        url: clean_url,
                        snippet,
                    });
                    if hits.len() >= limit {
                        break;
                    }
                }
            }
        }
    }

    hits
}

/// DuckDuckGo hrefs often wrap destination URLs in `/l/?uddg=<encoded-url>` or are direct links.
fn extract_ddg_target_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let encoded = &href[pos + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        let val = &encoded[..end];
        percent_decode(val)
    } else if href.starts_with("http://") || href.starts_with("https://") {
        href.to_owned()
    } else if href.starts_with("//") {
        format!("https:{href}")
    } else {
        String::new()
    }
}

fn strip_tags(s: &str) -> String {
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

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddg_url_decoding_extracts_target() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&rut=123";
        assert_eq!(extract_ddg_target_url(href), "https://www.rust-lang.org/");
    }

    #[test]
    fn ddg_html_parser_extracts_hits() {
        let sample_html = concat!(
            "<div class=\"result result--ad\">Ad result</div>",
            "<div class=\"result__body\">",
            "<h2 class=\"result__title\">",
            "<a class=\"result__a\" href=\"/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F\">The <b>Rust</b> Programming Language</a>",
            "</h2>",
            "<a class=\"result__snippet\" href=\"#\">A book on learning <b>Rust</b> from ground up.</a>",
            "</div>",
            "<div class=\"result__body\">",
            "<h2 class=\"result__title\">",
            "<a class=\"result__a\" href=\"https://github.com/rust-lang/rust\">Rust Repository</a>",
            "</h2>",
            "<a class=\"result__snippet\" href=\"#\">Empowering everyone to build reliable and efficient software.</a>",
            "</div>"
        );

        let hits = parse_ddg_html(sample_html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/book/");
        assert_eq!(hits[0].snippet, "A book on learning Rust from ground up.");

        assert_eq!(hits[1].title, "Rust Repository");
        assert_eq!(hits[1].url, "https://github.com/rust-lang/rust");
        assert_eq!(
            hits[1].snippet,
            "Empowering everyone to build reliable and efficient software."
        );
    }

    #[test]
    fn web_fetch_refuses_file_scheme_and_private_addresses() {
        let stop = Arc::new(AtomicBool::new(false));
        let fetch = WebFetch {
            stop: Arc::clone(&stop),
            timeout_secs: 10,
        };

        // File scheme refusal
        let res = fetch.run(&json!({ "url": "file:///etc/passwd" }));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("invalid URL scheme"));

        // Loopback refusal (SSRF guard)
        let res = fetch.run(&json!({ "url": "http://127.0.0.1:8080/secret" }));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("destination refused"));
    }

    #[test]
    fn web_fetch_respects_stop_flag() {
        let stop = Arc::new(AtomicBool::new(true));
        let fetch = WebFetch {
            stop: Arc::clone(&stop),
            timeout_secs: 10,
        };

        let res = fetch.run(&json!({ "url": "https://example.com" }));
        assert_eq!(res, Err("stopped".to_owned()));
    }
}

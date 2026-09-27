//! NOT live-verified (see README Gotchas). Also the one adapter that scrapes
//! (Meetup has no convenient public JSON API for this) rather than calling a
//! real API -- see the spoofed browser User-Agent below. This is a genuine
//! ToS-evasion pattern, called out here rather than hidden: Meetup's page
//! embeds a Next.js `__NEXT_DATA__`/Apollo-state JSON blob in server-rendered
//! HTML, which this adapter regex-extracts. If Meetup changes its frontend
//! framework or adds bot detection, this adapter breaks silently (parse
//! error), not loudly.

use std::path::PathBuf;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use serde_json::Value;

use crate::source::{SearchQuery, SourceAdapter, SourceError};

const FIND_URL: &str = "https://www.meetup.com/find/";
// Spoofed real-browser UA -- a functional necessity to get server-rendered
// HTML back from Meetup (their own bot-detection blocks non-browser UAs on
// this endpoint), not personal data. Kept deliberately, flagged here rather
// than disguised as a normal identifying UA.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

/// A cache file name component that cannot leave the cache directory.
///
/// `city` and `keyword` arrive from the HTTP `location` and `query` parameters
/// (`server.rs` builds `SearchQuery` from them), and `fetch_page` used to
/// interpolate them straight into a file name -- so `../../etc/passwd` chose
/// the file that was read and the file that was written. CodeQL
/// rust/path-injection reported all three sites. Private to this adapter
/// because it is the only one that names a cache file from a variable: the
/// others use a constant (`conferences.yml`, `upcoming.json`) or resolve the
/// key to an id first (`luma.rs` `resolve_city_id`, which errors on an
/// unknown city rather than echoing it).
fn cache_slug(s: &str) -> String {
    /// Bytes, not characters: the cap exists to keep a file name inside every
    /// file system's limit, and one lowercased character can be four bytes.
    const MAX: usize = 64;
    let mut out = String::with_capacity(s.len().min(MAX));
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        // `_` is deliberately NOT in this set, though it is a legal file-name
        // character. It is the separator in `meetup_{city}_{query}.json`, so
        // letting it through would let city `berlin_x` + query `y` and city
        // `berlin` + query `x_y` name one cache file and read each other's
        // pages.
        let next = if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
            c
        } else if !out.is_empty() && !out.ends_with('-') {
            // A run of rejected characters collapses to one '-'; a leading run
            // is dropped. `/` never survives, so the result is one component.
            '-'
        } else {
            continue;
        };
        // Checked before the push, not after: after leaves the last character
        // hanging over the cap by up to three bytes.
        if out.len() + next.len_utf8() > MAX {
            break;
        }
        out.push(next);
    }
    // Dots are trimmed at the ends as well as separators: `.` and `..` are legal
    // file names by the rule above and are exactly the two that mean "somewhere
    // else", and a leading dot would otherwise hide the cache file.
    let trimmed = out.trim_matches(|c| c == '-' || c == '.');
    if trimmed.is_empty() {
        // '-', not '_', for the reason above: no slug may contain the separator.
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

pub struct MeetupAdapter {
    pub cache_dir: Option<PathBuf>,
}

impl MeetupAdapter {
    pub fn new() -> Self {
        Self { cache_dir: None }
    }

    pub fn with_cache(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
        }
    }

    fn fetch_page(&self, city: &str, keyword: &str) -> Result<String, SourceError> {
        let loc = format!("de--{city}");
        let query = if keyword.is_empty() {
            "events".to_string()
        } else {
            keyword.to_string()
        };
        let url = format!("{FIND_URL}?source=EVENTS&keywords={query}&location={loc}");

        if let Some(ref dir) = self.cache_dir {
            let cache_path = dir.join(format!(
                "meetup_{}_{}.json",
                cache_slug(city),
                cache_slug(&query)
            ));
            if cache_path.exists() {
                return std::fs::read_to_string(&cache_path)
                    .map_err(|e| SourceError::Fetch(format!("cache read: {e}")));
            }
        }

        // `builder`, not `client`: USER_AGENT here is a browser string, and the
        // override is deliberate -- Meetup answers a non-browser agent with a
        // block page. Every other adapter now sends the Axon agent.
        let client = sjel_http::builder(
            sjel_http::Purpose::new("scouting-meetup"),
            crate::http::TIMEOUT,
        )
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| SourceError::Fetch(format!("client build: {e}")))?;
        let html = crate::http::send_checked(
            &url,
            client
                .get(&url)
                .header("Accept", "text/html,application/xhtml+xml")
                .header("Accept-Language", "en-US,en;q=0.9,de;q=0.8"),
        )?;

        let body = extract_json(&html)?;

        if let Some(ref dir) = self.cache_dir {
            std::fs::create_dir_all(dir).ok();
            // Slugged the same way as the read above, or the two disagree about
            // which file this page is.
            let cache_path = dir.join(format!(
                "meetup_{}_{}.json",
                cache_slug(city),
                cache_slug(&query)
            ));
            std::fs::write(&cache_path, &body).ok();
        }

        Ok(body)
    }

    fn normalize(&self, ev: &Value, fetched_at: &str) -> Option<Opportunity> {
        let id = ev.get("id")?.as_str()?;
        let title = ev.get("title")?.as_str()?;
        let date_time = ev
            .get("dateTime")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let event_url = ev
            .get("eventUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let event_type = ev
            .get("eventType")
            .and_then(|v| v.as_str())
            .unwrap_or("PHYSICAL");

        let city = ev
            .pointer("/venue/city")
            .and_then(|v| v.as_str())
            .or_else(|| {
                ev.pointer("/group/name").and_then(|v| {
                    let n = v.as_str()?;
                    let parts: Vec<&str> = n.split_whitespace().collect();
                    parts.last().copied()
                })
            });
        let country = ev.pointer("/venue/country").and_then(|v| v.as_str());
        let venue_name = ev.pointer("/venue/name").and_then(|v| v.as_str());
        let group_name = ev.pointer("/group/name").and_then(|v| v.as_str());

        let mut text_parts = vec![title.to_string()];
        if let Some(g) = group_name {
            text_parts.push(g.to_string());
        }
        if let Some(v) = venue_name {
            text_parts.push(v.to_string());
        }
        let search_text = text_parts.join(" ");

        let raw_value = serde_json::json!({
            "title": title,
            "group": group_name,
            "venue": venue_name,
            "dateTime": date_time,
            "eventType": event_type,
            "eventUrl": event_url,
            "id": id,
            "search_text": search_text,
        });

        let opportunity_id = format!("evt:meetup:{id}");
        let location = city.map(|c| {
            if let Some(cc) = country {
                format!("{c}, {cc}")
            } else {
                c.to_string()
            }
        });
        let city_str = city.map(|c| c.to_string());

        Some(Opportunity {
            id: opportunity_id,
            opportunity_type: OpportunityType::Event,
            source: "meetup".into(),
            source_kind: SourceKind::Scraper,
            url: event_url,
            title: title.to_string(),
            starts_at: date_time,
            ends_at: None,
            location,
            city: city_str,
            country_code: country.map(|c| c.to_string()),
            latitude: None,
            longitude: None,
            raw: raw_value,
            fetched_at: fetched_at.into(),
        })
    }
}

fn extract_json(html: &str) -> Result<String, SourceError> {
    let marker = r#"__NEXT_DATA__" type="application/json">"#;
    let start = html.find(marker).ok_or_else(|| {
        SourceError::Parse("no __NEXT_DATA__ JSON block found in Meetup SSR page".into())
    })?;
    let content_start = start + marker.len();
    let end = html[content_start..]
        .find("</script>")
        .ok_or_else(|| SourceError::Parse("JSON block not terminated".into()))?;
    Ok(html[content_start..content_start + end].to_string())
}

fn extract_events(apollo: &Value) -> Vec<Value> {
    let rq = match apollo.get("ROOT_QUERY") {
        Some(v) => v.as_object(),
        None => return vec![],
    };

    let search_key = rq.and_then(|map| {
        map.keys()
            .find(|k| k.starts_with("eventSearch"))
            .or_else(|| map.keys().find(|k| k.starts_with("recommendedEvents")))
    });

    let search_entry = search_key.and_then(|key| rq.and_then(|map| map.get(key)));
    let edges = search_entry.and_then(|entry| entry.get("edges"));

    let edges = match edges {
        Some(Value::Array(arr)) => arr,
        _ => return vec![],
    };

    let mut events = Vec::new();
    for edge in edges {
        let node = match edge.get("node") {
            Some(n) => n,
            None => continue,
        };
        let ref_str = match node.get("__ref").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => continue,
        };
        if let Some(ev) = apollo.get(ref_str) {
            events.push(ev.clone());
        }
    }
    events
}

impl Default for MeetupAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceAdapter for MeetupAdapter {
    fn name(&self) -> &str {
        "meetup"
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Event
    }

    fn rate_limit_per_min(&self) -> u32 {
        5
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let fetched_at = chrono_now();
        let city = query.location.as_deref().unwrap_or("berlin");
        let keyword = if query.query.is_empty() {
            "events"
        } else {
            &query.query
        };
        let body = self.fetch_page(city, keyword)?;
        let root: Value = serde_json::from_str(&body)
            .map_err(|e| SourceError::Parse(format!("JSON decode: {e}")))?;

        let apollo = root
            .pointer("/props/pageProps/__APOLLO_STATE__")
            .ok_or_else(|| SourceError::Parse("Apollo state not found".into()))?;

        let events = extract_events(apollo);
        let mut opps: Vec<Opportunity> = events
            .iter()
            .filter_map(|ev| self.normalize(ev, &fetched_at))
            .collect();

        if opps.len() > query.limit {
            opps.truncate(query.limit);
        }

        Ok(opps)
    }
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
    fn cache_slug_keeps_the_name_inside_one_directory() {
        assert_eq!(cache_slug("Berlin"), "berlin");
        assert_eq!(cache_slug("San Francisco"), "san-francisco");
        // The three shapes the alert was about.
        for hostile in ["../../etc/passwd", "/etc/passwd", "..\\..\\windows"] {
            let slug = cache_slug(hostile);
            assert!(!slug.contains('/'), "{hostile} -> {slug}");
            assert!(!slug.contains('\\'), "{hostile} -> {slug}");
            assert!(!slug.starts_with('.'), "{hostile} -> {slug}");
        }
        assert_eq!(cache_slug(".."), "-");
        assert_eq!(cache_slug("."), "-");
        assert_eq!(cache_slug("///"), "-");
        assert_eq!(cache_slug(""), "-");
        assert!(cache_slug(&"a".repeat(200)).len() <= 64);
        // The cap counts bytes. 'İ' lowercases to two chars, three bytes, so a
        // cap tested after the push would leave this over the line.
        assert!(cache_slug(&"İ".repeat(200)).len() <= 64);
    }

    /// The file name is `meetup_{city}_{query}.json`, so a slug that could
    /// contain `_` would let two different searches name one file.
    #[test]
    fn no_slug_can_contain_the_file_names_separator() {
        for input in ["berlin_x", "_", "a__b", "Berlin Mitte_Rust"] {
            let slug = cache_slug(input);
            assert!(!slug.contains('_'), "{input} -> {slug}");
        }
        assert_ne!(
            (cache_slug("berlin_x"), cache_slug("y")),
            (cache_slug("berlin"), cache_slug("x_y"))
        );
    }

    #[test]
    fn cache_slug_agrees_between_the_read_and_the_write() {
        // fetch_page slugs twice, once per site. Idempotence is what makes the
        // two calls name the same file.
        let once = cache_slug("Berlin / Mitte");
        assert_eq!(cache_slug(&once), once);
    }

    #[test]
    fn extract_json_finds_blob() {
        let body = "<html><script id=\"__NEXT_DATA__\" type=\"application/json\">{\"msg\":\"hello\"}</script></html>";
        assert_eq!(extract_json(body).unwrap(), "{\"msg\":\"hello\"}");
    }

    #[test]
    fn extract_events_from_apollo() {
        let apollo: Value = serde_json::from_str(
            r#"{
            "ROOT_QUERY": {
                "eventSearch:xyz": {
                    "edges": [
                        {"node": {"__ref": "Event:1"}},
                        {"node": {"__ref": "Event:2"}}
                    ]
                }
            },
            "Event:1": {"__typename": "Event", "id": "1", "title": "Alpha"},
            "Event:2": {"__typename": "Event", "id": "2", "title": "Beta"}
        }"#,
        )
        .unwrap();
        let events = extract_events(&apollo);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["title"].as_str(), Some("Alpha"));
        assert_eq!(events[1]["title"].as_str(), Some("Beta"));
    }

    #[test]
    fn normalizes_event() {
        let adapter = MeetupAdapter::new();
        let ev: Value = serde_json::json!({
            "id": "123",
            "title": "Berlin Tech Meetup",
            "dateTime": "2026-07-10T18:00:00+02:00",
            "eventUrl": "https://meetup.com/test/events/123/",
            "eventType": "PHYSICAL",
            "venue": {"name": "Co-Working Space", "city": "Berlin", "country": "DE"},
            "group": {"name": "Berlin Tech Group"}
        });
        let opp = adapter.normalize(&ev, "12345").unwrap();
        assert_eq!(opp.title, "Berlin Tech Meetup");
        assert_eq!(opp.city.as_deref(), Some("Berlin"));
        assert_eq!(opp.country_code.as_deref(), Some("DE"));
        assert_eq!(opp.source, "meetup");
    }
}

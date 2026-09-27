//! Live-verified 2026-07-30 (see README § Verdict). Two scopes, one payload
//! shape:
//!   * `LumaScope::Discover` — a city's public discover feed
//!     (`/discover/get-paginated-events`).
//!   * `LumaScope::Calendar` — one Luma calendar's own items
//!     (`/calendar/get-items`), which is what "track a calendar" means.
//!
//! Both endpoints return the same `entries[].event` object, so one parse and
//! one `normalize()` serve both.
//!
//! Three bugs this file carried until that verification, all invisible to the
//! fixture tests:
//!   1. `fetch_with_headers` never looked at the HTTP status, so Luma's 404
//!      body (`{"message": ...}`) reached serde and surfaced as
//!      `parse: events decode: missing field 'entries'` — a parse bug report
//!      for what was really a dead place id.
//!   2. `FALLBACK_CITIES` was consulted first and never refreshed. 19 of its
//!      20 ids 404 today; only Berlin still resolves. The live
//!      `bootstrap-page` lookup that would have fixed this was written but
//!      left `#[allow(dead_code)]`, so it never ran.
//!   3. The pagination cursor is `next_cursor`, not `pagination_cursor`, so
//!      every multi-page fetch silently stopped after page one.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use serde::{Deserialize, Serialize};

use crate::source::{SearchQuery, SourceAdapter, SourceError};

const API_BASE: &str = "https://api2.luma.com";
// Update once Axon goes public (no public GitHub remote yet, see PROJECTS.md).

/// Last-resort bootstrap map, used only when the live `bootstrap-page` lookup
/// fails (offline, rate limited). **Known stale**: probed 2026-07-30, only the
/// Berlin id still resolves; the other 19 return 404. Kept as a degraded
/// offline path rather than deleted — a stale id now produces a loud
/// `SourceError::Fetch` with the HTTP status, not a bogus parse error. Bonn,
/// Cologne and Frankfurt are not Luma discover places at all and were never
/// resolvable.
const FALLBACK_CITIES: &[(&str, &str)] = &[("berlin", "discplace-gCfX0s3E9Hgo3rG")];

#[derive(Debug, Deserialize, Serialize)]
struct BootstrapPage {
    places: Vec<PlaceEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PlaceEntry {
    place: PlaceInfo,
}

#[derive(Debug, Deserialize, Serialize)]
struct PlaceInfo {
    name: Option<String>,
    api_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct EventsResponse {
    entries: Vec<EventEntry>,
    has_more: Option<bool>,
    /// Luma renamed this from `pagination_cursor`; the alias keeps an older
    /// response shape working rather than silently paging once and stopping.
    #[serde(alias = "pagination_cursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct EventEntry {
    event: EventInfo,
}

/// The subset of Luma's event object this adapter maps. Every field below was
/// confirmed present in a live response on 2026-07-30; the former
/// `description` field was not (Luma does not return it on either list
/// endpoint) and has been dropped rather than left defaulting to `""`.
#[derive(Debug, Deserialize, Serialize)]
struct EventInfo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    api_id: String,
    /// A slug (`"claude-fq5h"`), not a URL — `normalize` prefixes the host.
    #[serde(default)]
    url: String,
    /// UTC instant (`"2026-07-30T16:00:00.000Z"`), paired with `timezone`.
    start_at: Option<String>,
    end_at: Option<String>,
    /// IANA zone the event is held in. Kept because the calendar promotion
    /// needs to know the instant is UTC, not local wall time.
    timezone: Option<String>,
    location_type: Option<String>,
    /// Which Luma calendar published this event — the id a `luma-calendar`
    /// source entry declares.
    calendar_api_id: Option<String>,
    geo_address_info: Option<GeoAddress>,
}

/// Live shape as of 2026-07-30. The former `city_state`/`full_address` fields
/// exist only inside `localized.<lang>`, never at this level, so they always
/// deserialized to `""`; both were unused and are gone.
#[derive(Debug, Deserialize, Serialize)]
struct GeoAddress {
    city: Option<String>,
    /// Full country name (`"Germany"`), not an ISO code — see `normalize`.
    country: Option<String>,
    address: Option<String>,
    /// Present on a minority of events (15 of 58 on the public Claude
    /// Community calendar, checked 2026-08-01). Luma nests the pair rather
    /// than putting `latitude`/`longitude` on the address itself, so this
    /// needs its own struct.
    place_coordinate: Option<PlaceCoordinate>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PlaceCoordinate {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

/// Which Luma surface an adapter instance reads.
#[derive(Debug, Clone)]
pub enum LumaScope {
    /// A city's public discover feed. `SearchQuery::location` picks the city.
    Discover,
    /// One Luma calendar, by its `cal-…` api id.
    Calendar { api_id: String },
}

#[derive(Debug)]
pub struct LumaAdapter {
    pub cache_dir: Option<PathBuf>,
    /// Set when this adapter was built from a `sources[]` entry, so `name()`
    /// answers that entry's id rather than the shared adapter type. Two tracked
    /// calendars are two sources and need two `source_state` rows.
    source_id: Option<String>,
    scope: LumaScope,
    fallback_city_ids: HashMap<String, String>,
    /// Live `bootstrap-page` result, fetched at most once per process.
    live_city_ids: OnceLock<HashMap<String, String>>,
}

impl LumaAdapter {
    pub fn new() -> Self {
        Self {
            cache_dir: None,
            source_id: None,
            scope: LumaScope::Discover,
            fallback_city_ids: FALLBACK_CITIES
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            live_city_ids: OnceLock::new(),
        }
    }

    /// Track one Luma calendar. `api_id` must be a `cal-…` id; Luma exposes no
    /// public slug→id lookup, so a slug is rejected here instead of being
    /// guessed into a URL that 404s.
    /// Tags this adapter with the `sources[]` id it was built from. Only the
    /// config path calls it; hand-constructed adapters keep the type name.
    pub fn with_source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    pub fn for_calendar(api_id: impl Into<String>) -> Result<Self, SourceError> {
        let api_id = api_id.into();
        if !api_id.starts_with("cal-") {
            return Err(SourceError::Fetch(format!(
                "'{api_id}' is not a Luma calendar api id (expected 'cal-…'); \
                 read it from an event's calendar_api_id — Luma has no public slug lookup"
            )));
        }
        Ok(Self {
            scope: LumaScope::Calendar { api_id },
            ..Self::new()
        })
    }

    pub fn with_cache(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
            ..Self::new()
        }
    }

    /// Live bootstrap first, hardcoded table only as a fallback. The old order
    /// was the reverse, which is why a stale table shadowed a healthy endpoint
    /// for every city but one.
    fn resolve_city_id(&self, city: &str) -> Result<String, SourceError> {
        let key = city.trim().to_lowercase();
        let live = self.live_city_ids.get_or_init(|| match self.fetch_bootstrap() {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!("warning: luma bootstrap-page lookup failed ({error}); falling back to the built-in city table");
                HashMap::new()
            }
        });
        if let Some(id) = live.get(&key) {
            return Ok(id.clone());
        }
        if let Some(id) = self.fallback_city_ids.get(&key) {
            return Ok(id.clone());
        }
        let mut known: Vec<&str> = live.keys().map(String::as_str).collect();
        known.sort_unstable();
        Err(SourceError::Fetch(format!(
            "'{city}' is not a Luma discover place. Known places: {}",
            if known.is_empty() {
                "(bootstrap unavailable)".to_string()
            } else {
                known.join(", ")
            }
        )))
    }

    fn fetch_bootstrap(&self) -> Result<HashMap<String, String>, SourceError> {
        let url = format!("{API_BASE}/discover/bootstrap-page");
        let body = fetch_with_headers(&url)?;
        let page: BootstrapPage = serde_json::from_str(&body)
            .map_err(|e| SourceError::Parse(format!("bootstrap decode: {e}")))?;
        let mut ids = HashMap::new();
        for entry in page.places {
            if let (Some(name), Some(api_id)) = (entry.place.name, entry.place.api_id) {
                ids.insert(name.to_lowercase(), api_id);
            }
        }
        Ok(ids)
    }

    /// Pages through one list endpoint. Both scopes share this: the response
    /// envelope (`entries`/`has_more`/`next_cursor`) is identical.
    fn fetch_paginated(
        &self,
        base_url: &str,
        cache_key: &str,
        limit: usize,
    ) -> Result<Vec<EventInfo>, SourceError> {
        if let Some(ref dir) = self.cache_dir {
            let path = dir.join(format!("luma_{cache_key}.json"));
            if path.exists() {
                let body = fs::read_to_string(&path)
                    .map_err(|e| SourceError::Fetch(format!("cache read {path:?}: {e}")))?;
                return Self::parse_events_response(&body);
            }
        }

        let page_limit = limit.clamp(1, 40);
        let mut all_events = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = format!("{base_url}&pagination_limit={page_limit}");
            if let Some(ref c) = cursor {
                url.push_str(&format!("&pagination_cursor={c}"));
            }

            let body = fetch_with_headers(&url)?;
            let resp: EventsResponse = serde_json::from_str(&body)
                .map_err(|e| SourceError::Parse(format!("events decode: {e}")))?;
            let page_count = resp.entries.len();
            all_events.extend(resp.entries.into_iter().map(|e| e.event));

            if all_events.len() >= limit {
                all_events.truncate(limit);
                break;
            }
            if page_count == 0 || !resp.has_more.unwrap_or(false) {
                break;
            }
            match resp.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        Ok(all_events)
    }

    fn parse_events_response(body: &str) -> Result<Vec<EventInfo>, SourceError> {
        let resp: EventsResponse = serde_json::from_str(body)
            .map_err(|e| SourceError::Parse(format!("events decode: {e}")))?;
        Ok(resp.entries.into_iter().map(|e| e.event).collect())
    }

    fn normalize(&self, ev: EventInfo, fallback_city: &str, fetched_at: &str) -> Opportunity {
        let url = if ev.url.starts_with("http") {
            ev.url.clone()
        } else {
            format!("https://lu.ma/{}", ev.url)
        };
        let opportunity_id = format!("evt:luma:{}", ev.api_id);
        let (city, country) = match ev.geo_address_info {
            Some(ref geo) => (
                geo.city
                    .clone()
                    .filter(|c| !c.is_empty())
                    .or_else(|| non_empty(fallback_city)),
                geo.country.clone(),
            ),
            None => (non_empty(fallback_city), None),
        };
        let location = ev
            .geo_address_info
            .as_ref()
            .and_then(|geo| geo.address.clone())
            .filter(|a| !a.is_empty())
            .or_else(|| city.clone());
        let coordinate = ev
            .geo_address_info
            .as_ref()
            .and_then(|geo| geo.place_coordinate.as_ref());
        let (latitude, longitude) = match coordinate {
            // Half a pair is not a location. Luma has not been seen sending
            // one, but taking only the complete pair means a consumer never
            // has to ask whether the other half was dropped.
            Some(c) => match (c.latitude, c.longitude) {
                (Some(lat), Some(lng)) => (Some(lat), Some(lng)),
                _ => (None, None),
            },
            None => (None, None),
        };
        // Serialized after the borrows above so the whole event object — the
        // UTC instants and the IANA `timezone` in particular — survives into
        // `raw` as the evidence the calendar promotion reads.
        let raw_value = serde_json::to_value(&ev).unwrap_or(serde_json::Value::Null);

        Opportunity {
            id: opportunity_id,
            opportunity_type: OpportunityType::Event,
            source: "luma".into(),
            source_kind: SourceKind::Api,
            url,
            title: ev.name,
            starts_at: ev.start_at,
            ends_at: ev.end_at,
            location,
            city,
            // Luma sends a full country name ("Germany"), not an ISO code.
            // Passed through unconverted rather than half-mapped — nothing
            // downstream reads it as a code today.
            country_code: country,
            latitude,
            longitude,
            raw: raw_value,
            fetched_at: fetched_at.into(),
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn fetch_with_headers(url: &str) -> Result<String, SourceError> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("scouting-luma"),
        crate::http::TIMEOUT,
    )
    .map_err(|e| SourceError::Fetch(format!("client build: {e}")))?;
    // The status check that made this adapter honest (#54) now lives in
    // crate::http, so the three siblings that had the same gap share it (#62)
    // instead of carrying three copies that can drift apart.
    crate::http::send_checked(
        url,
        client
            .get(url)
            .header("Origin", "https://luma.com")
            .header("Referer", "https://luma.com/discover"),
    )
}

impl Default for LumaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceAdapter for LumaAdapter {
    fn name(&self) -> &str {
        self.source_id.as_deref().unwrap_or("luma")
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Event
    }

    fn rate_limit_per_min(&self) -> u32 {
        20
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let fetched_at = chrono_now();
        let (base_url, cache_key, fallback_city) = match self.scope {
            LumaScope::Discover => {
                let city = query.location.as_deref().unwrap_or("berlin");
                let city_id = self.resolve_city_id(city)?;
                (
                    format!(
                        "{API_BASE}/discover/get-paginated-events?discover_place_api_id={city_id}"
                    ),
                    city_id,
                    city.to_string(),
                )
            }
            LumaScope::Calendar { ref api_id } => (
                // `period=future` is what makes this useful for a calendar:
                // the same calendar's ICS export ships every past event too.
                format!("{API_BASE}/calendar/get-items?calendar_api_id={api_id}&period=future"),
                api_id.clone(),
                String::new(),
            ),
        };

        let events = self.fetch_paginated(&base_url, &cache_key, query.limit)?;
        Ok(events
            .into_iter()
            .map(|e| self.normalize(e, &fallback_city, &fetched_at))
            .collect())
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

    /// Verbatim trim of a live `/calendar/get-items` entry (calendar
    /// `cal-TOpA5LAFfuDeFpu`, fetched 2026-07-30) — the fixture this file
    /// previously carried described a response shape Luma no longer sends.
    const LIVE_ENTRY: &str = r#"{
        "entries": [
            {
                "event": {
                    "api_id": "evt-E8mj424DVKBXFb4",
                    "calendar_api_id": "cal-TOpA5LAFfuDeFpu",
                    "name": "Berlin | Claude in the Wild",
                    "url": "claude-fq5h",
                    "start_at": "2026-07-30T16:00:00.000Z",
                    "end_at": "2026-07-30T19:00:00.000Z",
                    "timezone": "Europe/Berlin",
                    "location_type": "offline",
                    "geo_address_info": {
                        "city": "Berlin",
                        "country": "Germany",
                        "address": "Factory Berlin Mitte",
                        "localized": { "de": { "city_state": "Berlin, Deutschland" } }
                    }
                }
            }
        ],
        "has_more": true,
        "next_cursor": "eyJzdiI6IjIwMjYifQ"
    }"#;

    /// A second verbatim trim from the same calendar, fetched 2026-08-01 —
    /// this one carries `place_coordinate`, which 15 of that day's 58 entries
    /// did and the Berlin entry above did not. Kept as its own fixture rather
    /// than pasted into `LIVE_ENTRY`, because inventing a coordinate onto a
    /// response Luma really sent would make that fixture's "verbatim" claim
    /// false, and both cases need covering anyway.
    const LIVE_ENTRY_WITH_COORDINATE: &str = r#"{
        "entries": [
            {
                "event": {
                    "api_id": "evt-omUyAd4SNk3ONBm",
                    "calendar_api_id": "cal-TOpA5LAFfuDeFpu",
                    "name": "San Diego | Claude Impact Lab",
                    "url": "claude-08pt",
                    "start_at": "2026-08-01T16:00:00.000Z",
                    "end_at": "2026-08-03T01:00:00.000Z",
                    "timezone": "America/Los_Angeles",
                    "location_type": "offline",
                    "geo_address_info": {
                        "city": "San Diego",
                        "country": "United States",
                        "address": "Kiln",
                        "place_coordinate": {
                            "latitude": 33.0247756,
                            "longitude": -117.0774447
                        }
                    }
                }
            }
        ],
        "has_more": false,
        "next_cursor": null
    }"#;

    #[test]
    fn keeps_the_coordinate_when_luma_sends_one() {
        let adapter = LumaAdapter::for_calendar("cal-TOpA5LAFfuDeFpu").unwrap();
        let events = LumaAdapter::parse_events_response(LIVE_ENTRY_WITH_COORDINATE).unwrap();
        let opp = adapter.normalize(
            events.into_iter().next().unwrap(),
            "",
            "2026-08-01T00:00:00Z",
        );
        assert_eq!(opp.latitude, Some(33.0247756));
        assert_eq!(opp.longitude, Some(-117.0774447));
    }

    #[test]
    fn an_event_without_a_coordinate_stays_unlocated() {
        let adapter = LumaAdapter::for_calendar("cal-TOpA5LAFfuDeFpu").unwrap();
        let events = LumaAdapter::parse_events_response(LIVE_ENTRY).unwrap();
        let opp = adapter.normalize(
            events.into_iter().next().unwrap(),
            "",
            "2026-08-01T00:00:00Z",
        );
        assert_eq!(
            opp.city.as_deref(),
            Some("Berlin"),
            "the address half still parses"
        );
        assert_eq!(opp.latitude, None, "no coordinate is None, never a zero");
        assert_eq!(opp.longitude, None);
    }

    #[test]
    fn parses_live_response_shape() {
        let events = LumaAdapter::parse_events_response(LIVE_ENTRY).unwrap();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.api_id, "evt-E8mj424DVKBXFb4");
        assert_eq!(ev.calendar_api_id.as_deref(), Some("cal-TOpA5LAFfuDeFpu"));
        assert_eq!(ev.timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(ev.start_at.as_deref(), Some("2026-07-30T16:00:00.000Z"));
    }

    #[test]
    fn reads_next_cursor_not_pagination_cursor() {
        let resp: EventsResponse = serde_json::from_str(LIVE_ENTRY).unwrap();
        assert_eq!(resp.next_cursor.as_deref(), Some("eyJzdiI6IjIwMjYifQ"));
        assert_eq!(resp.has_more, Some(true));
    }

    #[test]
    fn accepts_legacy_pagination_cursor_alias() {
        let body = r#"{"entries": [], "has_more": false, "pagination_cursor": "legacy"}"#;
        let resp: EventsResponse = serde_json::from_str(body).unwrap();
        assert_eq!(resp.next_cursor.as_deref(), Some("legacy"));
    }

    #[test]
    fn normalize_keeps_utc_instants_and_timezone_as_evidence() {
        let adapter = LumaAdapter::new();
        let ev = LumaAdapter::parse_events_response(LIVE_ENTRY)
            .unwrap()
            .pop()
            .unwrap();
        let opp = adapter.normalize(ev, "", "0");
        assert_eq!(opp.id, "evt:luma:evt-E8mj424DVKBXFb4");
        assert_eq!(opp.url, "https://lu.ma/claude-fq5h");
        assert_eq!(opp.city.as_deref(), Some("Berlin"));
        assert_eq!(opp.location.as_deref(), Some("Factory Berlin Mitte"));
        assert_eq!(opp.starts_at.as_deref(), Some("2026-07-30T16:00:00.000Z"));
        assert_eq!(opp.raw["timezone"], "Europe/Berlin");
    }

    #[test]
    fn calendar_scope_rejects_a_slug() {
        // A slug is what a human copies out of a lu.ma URL, and Luma has no
        // public slug→id lookup — so this must fail loudly, not build a URL.
        let err = LumaAdapter::for_calendar("claudecommunity").unwrap_err();
        assert!(err.to_string().contains("cal-"), "got: {err}");
        assert!(LumaAdapter::for_calendar("cal-TOpA5LAFfuDeFpu").is_ok());
    }

    #[test]
    fn fallback_city_table_still_resolves_offline() {
        let adapter = LumaAdapter::new();
        assert!(adapter.fallback_city_ids.contains_key("berlin"));
        assert!(!adapter.fallback_city_ids.contains_key("unknown-town"));
    }
}

//! Splash That brand event hubs, by hub id.
//!
//! `community-events.arcteryx.com` is not a one-off site. It runs on Splash
//! That, a white-label event platform of the same shape as Luma, so one adapter
//! keyed by hub id covers every brand hosting there. The entry point is
//! `splash-hub`, not an Arc'teryx scraper.
//!
//! ## The endpoint
//!
//! ```text
//! GET https://<subsite>/?action=ohmyhub&method=getItems&format=json
//!     &splash_hub_id=<id>
//!     &options[filter_date]=upcoming
//!     &options[deep]=0
//! ```
//!
//! Same-origin on the subsite, no cookie, no token. `api.splashthat.com/hub/<gid>/events`
//! answering 401 was a red herring during the trace: that is a different,
//! admin-side API.
//!
//! ## Three things the shape decides, each measured rather than assumed
//!
//! **`filter_date` is not optional.** Unfiltered, the hub returns its whole
//! history: 3015 records and 5.5 MB, of which 3009 are past. `upcoming` returns
//! about 10 KB. A missing filter is not a slower correct answer, it is a
//! different query.
//!
//! **`result` is an object keyed by event id, not an array.** Verified against
//! the live response on 2026-08-05. Code expecting a list fails on the first
//! record, so the keys are read and then sorted, because an object has no order
//! and a run that reshuffles its own output is a run nobody can diff.
//!
//! **`end_timestamp` is a number or an empty string, in the same response.**
//! Four of six records carried a number and two carried `""`. Typed as
//! `Value` and coerced, because a stricter type here means the whole hub fails
//! to parse over an event with no end time.
//!
//! `splash_feed_id` is deliberately not used: it scopes one card feed on the
//! page, while the hub id alone is the whole hub. One declared entry per hub.
//!
//! ## Yield, honestly
//!
//! Hub 142966 is global across its history but small at any moment. On
//! 2026-08-05 it carried 6 upcoming events: Calgary, Mammoth Lakes, Portland,
//! Chicago, London, and one with no venue recorded at all. Expect stretches
//! where a European sweep returns nothing. That is the hub's rhythm rather than
//! a fault in the fetch, and it is why this ships as a declared source an
//! operator opts into rather than a default.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http;
use crate::localtime::utc_instant_from_epoch;
use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use crate::source::{SearchQuery, SourceAdapter, SourceError};

/// Which slice of the hub to ask for. Always sent — see the module note.
const FILTER_UPCOMING: &str = "upcoming";

pub struct SplashHubAdapter {
    /// The subsite the hub is served from, e.g. `community-events.arcteryx.com`.
    /// Part of the declaration because the query is same-origin: the hub id
    /// alone does not say which host answers for it.
    host: String,
    hub_id: String,
    source_id: Option<String>,
}

impl SplashHubAdapter {
    /// Build from a declared locator of the form `<host>/<hub_id>`.
    ///
    /// One field rather than two because a hub is not addressable without both
    /// halves, and a config that can express half of one is a config that will.
    pub fn for_hub(locator: &str) -> Result<Self, String> {
        let (host, hub_id) = locator
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_matches('/')
            .split_once('/')
            .ok_or_else(|| {
                format!(
                    "splash-hub locator must be '<host>/<hub_id>', e.g. \
                     'community-events.example.com/142966' — got '{locator}'"
                )
            })?;
        if host.is_empty() || hub_id.is_empty() {
            return Err(format!(
                "splash-hub locator needs both a host and a hub id — got '{locator}'"
            ));
        }
        if !hub_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "splash-hub id must be numeric — got '{hub_id}'. It is the \
                 `splash_hub_id` in the hub page's own XHR, not a slug."
            ));
        }
        Ok(Self {
            host: host.to_string(),
            hub_id: hub_id.to_string(),
            source_id: None,
        })
    }

    pub fn with_source_id(mut self, id: String) -> Self {
        self.source_id = Some(id);
        self
    }

    fn url(&self) -> String {
        format!(
            "https://{}/?action=ohmyhub&method=getItems&format=json\
             &splash_hub_id={}&options%5Bfilter_date%5D={FILTER_UPCOMING}&options%5Bdeep%5D=0",
            self.host, self.hub_id
        )
    }

    /// Parse a hub response into events, in a stable order.
    ///
    /// Separated from the fetch so the shape is testable without a network:
    /// every surprise this adapter has had so far was in the shape.
    pub fn parse_response(body: &str) -> Result<Vec<SplashEvent>, SourceError> {
        let envelope: Envelope = serde_json::from_str(body)
            .map_err(|e| SourceError::Parse(format!("splash hub response: {e}")))?;
        let mut keyed: Vec<(String, SplashEvent)> = envelope.result.into_iter().collect();
        // An object has no order. Sorting by the id it is keyed on makes two
        // runs over an unchanged hub produce the same list.
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(keyed.into_iter().map(|(_, event)| event).collect())
    }

    fn normalize(&self, event: SplashEvent, fetched_at: &str) -> Opportunity {
        let venue = event.venue.clone().unwrap_or_default();
        let date = event.date.clone().unwrap_or_default();

        // `date.tbd` with empty timestamps is a real case in this data. It stays
        // an opportunity with no start rather than being dropped: the calendar
        // promotion already refuses anything without a usable start, so the
        // undated event is visible without being able to block a day.
        let starts_at = as_epoch(&date.start_timestamp).map(utc_instant_from_epoch);
        let ends_at = as_epoch(&date.end_timestamp).map(utc_instant_from_epoch);

        let city = non_empty(&venue.city);
        let location = non_empty(&venue.address)
            .or_else(|| non_empty(&venue.name))
            .or_else(|| city.clone());

        Opportunity {
            id: format!("evt:splash:{}", event.event_id),
            opportunity_type: OpportunityType::Event,
            source: "splash_hub".into(),
            source_kind: SourceKind::Api,
            url: non_empty(&event.domain).unwrap_or_default(),
            title: event.title.clone(),
            starts_at,
            ends_at,
            location,
            city,
            // Spelled inconsistently inside a single hub — `USA`, `United
            // States`, `Canada`, `United Kingdom`, and one empty string, all in
            // the same 6-record response. Passed through unconverted, the way
            // Luma's country is: `geo.allow_countries` matches these
            // case-insensitively and an empty one falls to `allow_unknown`.
            country_code: non_empty(&venue.country),
            latitude: venue.lat,
            longitude: venue.lng,
            raw: serde_json::to_value(&event).unwrap_or(Value::Null),
            fetched_at: fetched_at.into(),
        }
    }
}

impl SourceAdapter for SplashHubAdapter {
    fn name(&self) -> &str {
        self.source_id.as_deref().unwrap_or("splash_hub")
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Event
    }

    fn rate_limit_per_min(&self) -> u32 {
        20
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let fetched_at = chrono_now();
        let url = self.url();
        let client = sjel_http::client(
            sjel_http::Purpose::new("scouting-splash-hub"),
            crate::http::TIMEOUT,
        )
        .map_err(|e| SourceError::Fetch(format!("client build: {e}")))?;
        // Through the checked send, so a changed query contract surfaces as the
        // status it is rather than as "this hub has no events" — the failure
        // mode three sibling adapters shipped with.
        let body = http::send_checked(&url, client.get(&url))?;

        let events = Self::parse_response(&body)?;
        Ok(events
            .into_iter()
            .take(query.limit.max(1))
            .map(|event| self.normalize(event, &fetched_at))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Wire shapes. Everything optional: this is somebody else's undocumented API,
// and a required field is a bet that it stays required.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    result: HashMap<String, SplashEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SplashEvent {
    #[serde(default)]
    pub event_id: Value,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The canonical event URL.
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub splash_subdomain: String,
    #[serde(default)]
    pub event_status: String,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_featured: bool,
    #[serde(default)]
    pub rsvp_count: Value,
    #[serde(default)]
    pub tags: Vec<Value>,
    #[serde(default)]
    pub event_type: Option<Value>,
    #[serde(default)]
    pub date: Option<SplashDate>,
    #[serde(default)]
    pub venue: Option<SplashVenue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SplashDate {
    #[serde(default)]
    pub tbd: bool,
    #[serde(default)]
    pub timezone_identifier: String,
    /// Number or empty string, in the same response. See the module note.
    #[serde(default)]
    pub start_timestamp: Value,
    #[serde(default)]
    pub end_timestamp: Value,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SplashVenue {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub tbd: bool,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
}

/// Epoch seconds out of a field that may be a number, a numeric string, or an
/// empty string. `None` for anything else, including zero: a hub that means
/// "no end time" writes `""`, and 1970 is not a date this ever wants to store.
fn as_epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|secs| *secs > 0)
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from the live 2026-08-05 response: two records, the object
    /// keying, and the mixed `end_timestamp` type that a stricter shape would
    /// have failed on. A brand's public event listing, no personal data.
    const LIVE: &str = r#"{
      "result": {
        "459417976": {
          "event_id": 459417976,
          "title": "Prep Series Calgary: New Heights",
          "description": "An evening about climbing.",
          "domain": "https://community-events.example.com/newheights",
          "splash_subdomain": "prepseriescalgary",
          "event_status": "future",
          "is_private": false,
          "is_featured": false,
          "rsvp_count": 117,
          "tags": [],
          "event_type": { "name": "CAN West Event" },
          "date": {
            "tbd": false,
            "timezone_identifier": "America/Edmonton",
            "start": "08/11/2026 18:30:00",
            "start_timestamp": 1786494600,
            "end": "",
            "end_timestamp": ""
          },
          "venue": {
            "name": "Festival Hall",
            "address": "1215 10 Avenue Southeast",
            "city": "Calgary",
            "state": "AB",
            "country": "Canada",
            "tbd": false,
            "lat": 51.0405397,
            "lng": -114.0357699
          }
        },
        "459332389": {
          "event_id": 459332389,
          "title": "A London Evening",
          "domain": "https://community-events.example.com/london",
          "event_status": "future",
          "date": {
            "tbd": false,
            "timezone_identifier": "Europe/London",
            "start_timestamp": 1786000000,
            "end_timestamp": 1786010000
          },
          "venue": { "city": "London", "country": "United Kingdom", "lat": 51.5, "lng": -0.12 }
        }
      }
    }"#;

    fn adapter() -> SplashHubAdapter {
        SplashHubAdapter::for_hub("community-events.example.com/142966").expect("locator")
    }

    #[test]
    fn a_locator_names_both_the_host_and_the_hub() {
        let a = SplashHubAdapter::for_hub("community-events.example.com/142966").expect("plain");
        assert!(a.url().contains("community-events.example.com"));
        assert!(a.url().contains("splash_hub_id=142966"));

        // A pasted URL is the same declaration with noise on it.
        let pasted = SplashHubAdapter::for_hub("https://community-events.example.com/142966/");
        assert!(pasted.is_ok(), "a pasted hub URL is still a locator");
    }

    #[test]
    fn a_locator_missing_a_half_or_carrying_a_slug_is_refused() {
        assert!(SplashHubAdapter::for_hub("142966").is_err());
        assert!(SplashHubAdapter::for_hub("community-events.example.com/").is_err());
        // The adapter is not Debug, so `expect_err` cannot describe the Ok arm.
        let slug = match SplashHubAdapter::for_hub("community-events.example.com/newheights") {
            Err(detail) => detail,
            Ok(_) => panic!("a slug is not a hub id"),
        };
        assert!(slug.contains("numeric"), "got: {slug}");
    }

    /// The finding that costs 5.5 MB when forgotten.
    #[test]
    fn the_date_filter_is_always_on_the_url() {
        assert!(
            adapter().url().contains("filter_date%5D=upcoming"),
            "unfiltered returns the hub's whole history, 3009 of which are past"
        );
    }

    /// The correction to the original trace: `result` is keyed, not a list.
    #[test]
    fn a_keyed_result_object_is_read_and_ordered_by_its_keys() {
        let events = SplashHubAdapter::parse_response(LIVE).expect("parses");
        let ids: Vec<String> = events.iter().map(|e| e.event_id.to_string()).collect();
        assert_eq!(
            ids,
            vec!["459332389", "459417976"],
            "an object has no order; the ids give it one so two runs can be diffed"
        );
    }

    #[test]
    fn an_event_becomes_an_opportunity_with_its_place_and_instants() {
        let events = SplashHubAdapter::parse_response(LIVE).expect("parses");
        let calgary = events
            .iter()
            .find(|e| e.title.contains("Calgary"))
            .expect("present");
        let opp = adapter().normalize(calgary.clone(), "123");

        assert_eq!(opp.id, "evt:splash:459417976");
        assert_eq!(opp.title, "Prep Series Calgary: New Heights");
        assert_eq!(opp.url, "https://community-events.example.com/newheights");
        assert_eq!(opp.city.as_deref(), Some("Calgary"));
        assert_eq!(opp.country_code.as_deref(), Some("Canada"));
        assert_eq!(opp.latitude, Some(51.0405397));
        // The record carries both `start: "08/11/2026 18:30:00"` and
        // `start_timestamp: 1786494600`, and they are six hours apart because
        // the string is *local* wall time in America/Edmonton while the epoch
        // is UTC. The timestamp is the unambiguous one, which is why this
        // adapter reads it and ignores the string. Parsing that string as UTC
        // would put every North American event on the wrong evening, and half
        // of them on the wrong day.
        assert_eq!(
            opp.starts_at.as_deref(),
            Some("2026-08-12T00:30:00Z"),
            "epoch seconds become the UTC instant the rest of the crate stores"
        );
    }

    /// The type that would otherwise fail the whole hub.
    #[test]
    fn an_empty_end_timestamp_is_no_end_rather_than_a_parse_failure() {
        let events = SplashHubAdapter::parse_response(LIVE).expect("parses");
        let calgary = events
            .iter()
            .find(|e| e.title.contains("Calgary"))
            .expect("present");
        let opp = adapter().normalize(calgary.clone(), "123");
        assert!(opp.starts_at.is_some());
        assert_eq!(opp.ends_at, None);

        let london = events
            .iter()
            .find(|e| e.title.contains("London"))
            .expect("present");
        assert!(adapter().normalize(london.clone(), "123").ends_at.is_some());
    }

    /// A real case in this data, and the calendar promotion already refuses it.
    #[test]
    fn an_undated_event_is_kept_visible_without_a_start_it_could_block_a_day_with() {
        let tbd = r#"{"result":{"1":{"event_id":1,"title":"Date to come",
            "domain":"https://example.test/e",
            "date":{"tbd":true,"start_timestamp":"","end_timestamp":""},
            "venue":{"city":"Berlin","country":"Germany"}}}}"#;
        let events = SplashHubAdapter::parse_response(tbd).expect("parses");
        let opp = adapter().normalize(events[0].clone(), "123");
        assert_eq!(opp.starts_at, None);
        assert_eq!(opp.title, "Date to come");
    }

    #[test]
    fn zero_is_not_a_date() {
        assert_eq!(as_epoch(&serde_json::json!(0)), None);
        assert_eq!(as_epoch(&serde_json::json!("")), None);
        assert_eq!(as_epoch(&serde_json::json!("1786494600")), Some(1786494600));
        assert_eq!(as_epoch(&serde_json::json!(null)), None);
    }

    #[test]
    fn a_declared_source_id_is_the_adapter_name_so_the_cursor_is_per_hub() {
        assert_eq!(adapter().name(), "splash_hub");
        assert_eq!(
            adapter().with_source_id("arcteryx-community".into()).name(),
            "arcteryx-community"
        );
    }

    #[test]
    fn a_body_that_is_not_a_hub_response_is_a_parse_error_naming_itself() {
        let error = SplashHubAdapter::parse_response("<html>nope</html>").expect_err("not json");
        assert!(error.to_string().contains("splash hub response"));
    }

    #[test]
    fn an_empty_hub_is_no_events_rather_than_an_error() {
        assert!(SplashHubAdapter::parse_response(r#"{"result":{}}"#)
            .expect("parses")
            .is_empty());
    }
}

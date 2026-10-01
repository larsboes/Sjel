//! Every HTTP call the plan search makes, and nothing else.
//!
//! One function per upstream, each with an explicit timeout, each returning a
//! `Result`. No `.ok()`, no `unwrap_or_default()` on a response path: a
//! capability that could not be reached is reported as unreached, which is the
//! half `flight_when` leaves out when it swallows a calendar failure and every
//! day comes back `Free`.
//!
//! ## Calendar is read at `GET /api/windows` and nowhere else
//!
//! `FeasibleWindow` carries dates, a verdict and the days that would cost a
//! travel day. `GET /api/entries`, which `flight_when` reads, carries entry
//! **titles**, and `crate::windows::day_loads` puts them into `collisions`.
//! An adopted payload is written verbatim into the operator's cloud-synced
//! vault (`projection.rs`), so the reduced surface is the one this path reads.
//!
//! ## The 250 ms pause is this file's, not transit's
//!
//! `capabilities/transit/src/hafas.rs` sets a 15 s client timeout on the search
//! and carries no inter-request pause; the `std::thread::sleep(250)` in that
//! file sits inside the split-ticket `for i / for j` loop. The fan-out cadence
//! is transit's own CLI rule (`capabilities/transit/README.md`, the fan-out
//! section), so a caller that fans out over the HTTP route has to keep it
//! itself. Fares are a scrape behind a spoofed user agent; eight sequential
//! searches per plan search is a new traffic pattern against an undocumented
//! endpoint, and the cadence is the cheapest thing that keeps it polite.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::jobs::JOB_DEADLINE_S;
use crate::plan_search::{
    CandidateEvent, DestinationCandidate, FeasibleWindow, SeasonScore, SoftWeights, Sources,
};
use crate::store::{PlaceKind, PlaceRef};

const CALENDAR_TIMEOUT: Duration = Duration::from_secs(3);
const PLACES_TIMEOUT: Duration = Duration::from_secs(3);
const PRESENCE_TIMEOUT: Duration = Duration::from_secs(2);
const CLIMATE_TIMEOUT: Duration = Duration::from_secs(3);
const SCOUTING_TIMEOUT: Duration = Duration::from_secs(5);
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(5);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Keys per climate request. The producer's own bound (`MAX_CLIMATE_KEYS` in
/// `capabilities/places/src/server.rs`), kept here so a shortlist longer than
/// eight is chunked rather than refused.
const CLIMATE_BATCH_KEYS: usize = 8;

/// Transit's own fan-out cadence, kept by the caller. See the module note.
const FARE_PAUSE: Duration = Duration::from_millis(250);

/// The hour a fare is priced for. A whole day of departures is a second search
/// dimension this route does not have a budget for; mid-morning is when a
/// day-trip-shaped journey is actually taken.
const DEPARTURE_TIME: &str = "T09:00:00";

fn base_url(variable: &str, port: u16) -> String {
    sjel_config::env_var(variable).unwrap_or_else(|_| format!("http://127.0.0.1:{port}"))
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    sjel_http::client(sjel_http::Purpose::new("trips-upstream"), timeout)
        .map_err(|error| format!("client build: {error}"))
}

/// One GET, with the status checked before the body is parsed.
fn get_json(url: &str, timeout: Duration) -> Result<Value, String> {
    let request = sjel_server::InboundAuth::with_loopback_auth(client(timeout)?.get(url), url);
    let response = request.send().map_err(|error| short(&error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("answered {}", status.as_u16()));
    }
    response
        .json()
        .map_err(|error| format!("unreadable reply: {}", short(&error.to_string())))
}

/// A reason short enough to sit inside a `reach` string, with the URL removed.
///
/// A reqwest error prints the whole URL chain, which would put a host and a
/// port into a response body for no diagnostic gain. Splitting on `:` was the
/// first attempt and it cut `...for url (http` mid-scheme; the parenthesised
/// URL is what has to go, not everything after the first colon.
fn short(reason: &str) -> String {
    let mut cleaned = String::with_capacity(reason.len());
    let mut depth = 0usize;
    for character in reason.chars() {
        match character {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => cleaned.push(character),
            _ => {}
        }
    }
    // Removing the parentheses leaves a double space and an orphan colon where
    // the URL was, so the words are re-joined rather than left as
    // "for url : connection refused".
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    words
        .join(" ")
        .replace(" :", ":")
        .trim_end_matches(':')
        .chars()
        .take(80)
        .collect()
}

/// The live implementation of [`Sources`].
pub struct HttpSources {
    started: Instant,
    origin_eva: RefCell<Option<Option<String>>>,
    eva_cache: RefCell<HashMap<String, Option<String>>>,
    retrospectives: HashMap<String, crate::retrospective::DestinationFactor>,
    soft_weights: Option<SoftWeights>,
}

impl HttpSources {
    pub fn new(started: Instant) -> Self {
        let soft_weights = crate::traveler_client::soft_weights();
        Self {
            started,
            origin_eva: RefCell::new(None),
            eva_cache: RefCell::new(HashMap::new()),
            retrospectives: HashMap::new(),
            soft_weights,
        }
    }

    pub fn with_retrospectives(
        mut self,
        retrospectives: HashMap<String, crate::retrospective::DestinationFactor>,
    ) -> Self {
        self.retrospectives = retrospectives;
        self
    }

    pub fn with_soft_weights(mut self, soft_weights: Option<SoftWeights>) -> Self {
        self.soft_weights = soft_weights;
        self
    }

    /// A station code for a place name, resolved through transit's own suggest
    /// surface and cached for the life of one job.
    ///
    /// `GET /api/search` takes EVA codes only, and `places` carries `eva:<code>`
    /// on its station rows only, so a city has to be resolved. This extends a
    /// path that already exists: `capabilities/places/src/backfill.rs` resolves
    /// bare codes through the same surface.
    fn eva_for(&self, name: &str) -> Result<Option<String>, String> {
        if let Some(hit) = self.eva_cache.borrow().get(name) {
            return Ok(hit.clone());
        }
        let url = format!(
            "{}/api/suggest?q={}",
            base_url("SJEL_TRANSIT_URL", 3000),
            urlencode(name)
        );
        let body = get_json(&url, SUGGEST_TIMEOUT)?;
        let eva = body
            .as_array()
            .and_then(|stations| stations.first())
            .and_then(|station| station.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.eva_cache
            .borrow_mut()
            .insert(name.to_string(), eva.clone());
        Ok(eva)
    }

    fn origin_eva(&self, origin: &PlaceRef) -> Result<Option<String>, String> {
        if let Some(cached) = self.origin_eva.borrow().clone() {
            return Ok(cached);
        }
        let declared = origin
            .id
            .strip_prefix("eva:")
            .map(str::to_string)
            .or_else(|| {
                origin
                    .id
                    .chars()
                    .all(|c| c.is_ascii_digit())
                    .then(|| origin.id.clone())
                    .filter(|id| !id.is_empty())
            });
        let eva = match declared {
            Some(code) => Some(code),
            None => self.eva_for(&origin.name)?,
        };
        *self.origin_eva.borrow_mut() = Some(eva.clone());
        Ok(eva)
    }
}

impl Sources for HttpSources {
    fn calendar_windows(
        &self,
        from: &str,
        to: &str,
        min_days: u32,
    ) -> Result<Vec<FeasibleWindow>, String> {
        // `to` is inclusive here and exclusive there, so the request asks for
        // one day more than the caller named.
        let ends_before = crate::windows::day_number(to)
            .map(|day| crate::windows::iso_of_day_number(day + 1))
            .unwrap_or_else(|| to.to_string());
        let url = format!(
            "{}/api/windows?from={from}&to={ends_before}&min_days={min_days}",
            base_url("SJEL_CALENDAR_URL", 8087)
        );
        let body = get_json(&url, CALENDAR_TIMEOUT)?;
        serde_json::from_value(body["windows"].clone())
            .map_err(|error| format!("unexpected windows shape: {error}"))
    }

    fn cities(&self) -> Result<Vec<DestinationCandidate>, String> {
        let url = format!("{}/api/places?kind=city", base_url("SJEL_PLACES_URL", 8093));
        let body = get_json(&url, PLACES_TIMEOUT)?;
        let rows = body["places"]
            .as_array()
            .ok_or_else(|| "unexpected places shape".to_string())?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let id = row["id"].as_str()?.to_string();
                let name = row["name"].as_str()?.to_string();
                Some(DestinationCandidate {
                    place_id: id.clone(),
                    destination: PlaceRef {
                        id,
                        name,
                        kind: PlaceKind::City,
                        address: None,
                        latitude: row["latitude"].as_f64(),
                        longitude: row["longitude"].as_f64(),
                    },
                    sources: vec!["places".into()],
                })
            })
            .collect())
    }

    fn opportunities(&self) -> Result<Vec<CandidateEvent>, String> {
        // No `/api` prefix: scouting serves this route at the root.
        let url = format!(
            "{}/opportunities?limit=500",
            base_url("SJEL_SCOUTING_URL", 8084)
        );
        let body = get_json(&url, SCOUTING_TIMEOUT)?;
        let rows = body["opportunities"]
            .as_array()
            .ok_or_else(|| "unexpected opportunities shape".to_string())?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                // The list is `{opportunity, event_route}` pairs.
                let row = row.get("opportunity").unwrap_or(row);
                Some(CandidateEvent {
                    id: row["id"].as_str()?.to_string(),
                    title: row["title"].as_str().unwrap_or_default().to_string(),
                    starts_at: row["starts_at"].as_str().map(str::to_string),
                    url: row["url"].as_str().unwrap_or_default().to_string(),
                    latitude: row["latitude"].as_f64(),
                    longitude: row["longitude"].as_f64(),
                    distance_km: None,
                })
            })
            .collect())
    }

    /// The batch form, chunked at the producer's own bound.
    ///
    /// The climate surface is optional by construction: any non-200, including
    /// the 404 a deployment without it answers, sets `reach.climate` to the
    /// failure, drops the season factor and re-normalises the rest. The search
    /// is correct with zero climate calls.
    ///
    /// It is chunked because the producer refuses more than
    /// `MAX_CLIMATE_KEYS` = 8 keys per request (`capabilities/places`, handler
    /// `climate`) while this search shortlists up to [`MAX_MAX_CANDIDATES`]
    /// = 20. One oversized call would answer 400 and lose the factor for every
    /// candidate.
    ///
    /// [`MAX_MAX_CANDIDATES`]: crate::plan_search::MAX_MAX_CANDIDATES
    fn climate(
        &self,
        place_ids: &[String],
        month: u32,
    ) -> Result<HashMap<String, SeasonScore>, String> {
        if place_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut found = HashMap::new();
        for chunk in place_ids.chunks(CLIMATE_BATCH_KEYS) {
            let url = format!(
                "{}/api/climate?place_ids={}",
                base_url("SJEL_PLACES_URL", 8093),
                urlencode(&chunk.join(","))
            );
            let body = get_json(&url, CLIMATE_TIMEOUT)?;
            found.extend(parse_climate(&body, month)?);
        }
        Ok(found)
    }

    fn price(
        &self,
        origin: &PlaceRef,
        destination: &DestinationCandidate,
        depart_iso: &str,
    ) -> Result<Option<i64>, String> {
        // Which of the two transit surfaces failed matters to a reader: the
        // station lookup and the fare search fail for different reasons and on
        // different days, and "answered 500" alone names neither.
        let Some(from_eva) = self
            .origin_eva(origin)
            .map_err(|reason| format!("transit station lookup {reason}"))?
        else {
            return Ok(None);
        };
        std::thread::sleep(FARE_PAUSE);
        let Some(to_eva) = self
            .eva_for(&destination.destination.name)
            .map_err(|reason| format!("transit station lookup {reason}"))?
        else {
            return Ok(None);
        };
        if from_eva == to_eva {
            return Ok(None);
        }
        std::thread::sleep(FARE_PAUSE);
        let url = format!(
            "{}/api/search?from={from_eva}&to={to_eva}&time={depart_iso}{DEPARTURE_TIME}",
            base_url("SJEL_TRANSIT_URL", 3000)
        );
        let body = get_json(&url, SEARCH_TIMEOUT)
            .map_err(|reason| format!("transit fare search {reason}"))?;
        let cheapest = body
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|journey| journey["total_price"].as_f64())
            .filter(|price| *price > 0.0)
            .fold(f64::INFINITY, f64::min);
        Ok(cheapest
            .is_finite()
            .then(|| (cheapest * 100.0).round() as i64))
    }

    fn presence(
        &self,
        latitude: f64,
        longitude: f64,
        from: &str,
        to: &str,
    ) -> Result<(u32, u32), String> {
        let url = format!(
            "{}/api/people/presence?latitude={latitude}&longitude={longitude}&from={from}&to={to}",
            base_url("SJEL_PLACES_URL", 8093)
        );
        let body = get_json(&url, PRESENCE_TIMEOUT)?;
        Ok((
            body["known_companions"].as_u64().unwrap_or(0) as u32,
            body["overlap_days"].as_u64().unwrap_or(0) as u32,
        ))
    }

    fn deadline_spent(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(JOB_DEADLINE_S)
    }

    fn observed_at(&self) -> String {
        now_iso()
    }

    fn soft_weights(&self) -> Option<SoftWeights> {
        self.soft_weights
    }

    fn retrospectives(&self) -> HashMap<String, crate::retrospective::DestinationFactor> {
        self.retrospectives.clone()
    }
}

/// Read the climate batch answer.
///
/// One shape, the producer's: `{best_months_rule, attribution, results:[{key,
/// resolved_by, matched_place, months:[{month, best_month, ...}]}]}`
/// (`capabilities/places/src/server.rs`, handler `climate`). `key` is the id
/// this search asked about and the id the rest of the search keys by, so it is
/// preferred over `matched_place.id`, which is the place the producer resolved
/// it to.
///
/// A month the normals do not name as best is not scored zero: "not the best
/// month" is not "unbearable". It is scored low enough to lose to a best month
/// and high enough to stay in the ranking, and the rationale names which month
/// is best.
///
/// `Err` when the body is not that shape at all, rather than an empty map. A
/// 200 nobody can read must not look like a measured answer with no season in
/// it: `reach.climate` would say "ok" while the 0.20-weight factor silently
/// vanished from every candidate. A place the producer has no normals for
/// answers `months: []`, which is an absent measurement and stays absent.
pub fn parse_climate(body: &Value, month: u32) -> Result<HashMap<String, SeasonScore>, String> {
    let rows = body["results"]
        .as_array()
        .ok_or_else(|| "unreadable reply: no results[]".to_string())?;

    let mut found = HashMap::new();
    for row in rows {
        let Some(place_id) = row["key"]
            .as_str()
            .or_else(|| row["matched_place"]["id"].as_str())
        else {
            continue;
        };
        let months: &[Value] = row["months"].as_array().map_or(&[], Vec::as_slice);
        let Some(asked_for) = months
            .iter()
            .find(|entry| entry["month"].as_u64() == Some(u64::from(month)))
        else {
            continue;
        };
        let best_month = months
            .iter()
            .find(|entry| entry["best_month"].as_bool() == Some(true))
            .and_then(|entry| entry["month"].as_u64())
            .map(|best| best as u32);
        let score = if asked_for["best_month"].as_bool() == Some(true) {
            1.0
        } else {
            0.3
        };
        found.insert(
            place_id.to_string(),
            SeasonScore {
                month,
                score,
                best_month,
            },
        );
    }
    Ok(found)
}

/// Percent-encode a query value. Place names carry spaces, commas and umlauts.
fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Now, as an ISO instant, without a date crate: the day number gives the date
/// and the remainder gives the clock.
pub fn now_iso() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let day = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        crate::windows::iso_of_day_number(day + crate::windows::UNIX_EPOCH_DAY),
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_query_value_survives_a_space_and_an_umlaut() {
        assert_eq!(urlencode("Bad Vilbel"), "Bad%20Vilbel");
        assert_eq!(urlencode("Köln"), "K%C3%B6ln");
        assert_eq!(urlencode("a,b"), "a%2Cb");
    }

    /// The shape AND the century. The first version of this function fed a Unix
    /// day count straight into `iso_of_day_number`, whose day 0 is 0000-03-01,
    /// and stamped `0056-11-03` into a live response body.
    #[test]
    fn now_is_an_iso_instant_in_this_century() {
        let now = now_iso();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.ends_with('Z'));
        assert_eq!(&now[4..5], "-");
        assert_eq!(&now[10..11], "T");
        let year: i64 = now[..4].parse().expect("a four-digit year");
        assert!(
            (2025..2100).contains(&year),
            "now_iso answered {now}, which is not a date this process is running on"
        );
        // The epoch shift, checked against a day whose date is known.
        assert_eq!(
            crate::windows::iso_of_day_number(crate::windows::UNIX_EPOCH_DAY),
            "1970-01-01"
        );
    }

    /// The producer's real envelope, read by `key` and scored from
    /// `months[].best_month`. Written against the body
    /// `capabilities/places`' `GET /api/climate` handler builds.
    #[test]
    fn a_climate_batch_is_read_by_the_key_the_search_asked_about() {
        let body = json!({
            "best_months_rule": "...",
            "attribution": "...",
            "results": [
                {
                    "key": "place:lisbon",
                    "resolved_by": "id",
                    "matched_place": { "id": "place:lisbon-registry", "name": "Lisbon" },
                    "reason": null,
                    "months": [
                        { "month": 6, "best_month": true, "in_window": false },
                        { "month": 10, "best_month": true, "in_window": true },
                    ],
                },
                {
                    "key": "place:oslo",
                    "resolved_by": "id",
                    "matched_place": { "id": "place:oslo", "name": "Oslo" },
                    "reason": null,
                    "months": [
                        { "month": 6, "best_month": true, "in_window": false },
                        { "month": 10, "best_month": false, "in_window": true },
                    ],
                },
                // A registered place with no normals yet. An absent
                // measurement stays absent rather than scoring zero.
                { "key": "place:nowhere", "resolved_by": "id", "months": [] },
                // A key the producer could not resolve at all.
                { "key": "place:unknown", "resolved_by": null, "matched_place": null,
                  "reason": "no place with that id", "months": [] },
            ],
        });
        let scored = parse_climate(&body, 10).expect("the producer's shape is readable");
        assert_eq!(scored["place:lisbon"].score, 1.0);
        assert_eq!(scored["place:lisbon"].month, 10);
        assert_eq!(scored["place:lisbon"].best_month, Some(6));
        assert!(scored["place:oslo"].score < scored["place:lisbon"].score);
        assert!(!scored.contains_key("place:nowhere"));
        assert!(!scored.contains_key("place:unknown"));
        assert_eq!(scored.len(), 2);
    }

    /// A 200 in a shape this reader does not know is an error, not an empty
    /// map: `reach.climate` saying "ok" while every season factor silently
    /// vanished is the failure a reader cannot see.
    #[test]
    fn a_climate_answer_it_cannot_read_is_an_error_rather_than_a_silent_blank() {
        let error = parse_climate(&json!({"error": "not found"}), 10)
            .expect_err("a body with no results[] is unreadable");
        assert!(error.contains("unreadable"), "{error}");
        assert!(parse_climate(&json!({"climate": [{"place_id": "p1"}]}), 10).is_err());
    }

    /// A failure reason goes into a response body, so it carries no URL.
    #[test]
    fn a_reason_is_short_and_carries_no_host() {
        let long =
            "error sending request for url (http://127.0.0.1:8093/api/places): connection refused";
        assert_eq!(
            short(long),
            "error sending request for url: connection refused"
        );
        assert!(!short(long).contains("8093"));
    }
}

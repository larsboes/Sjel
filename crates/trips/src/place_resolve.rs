//! A destination's coordinate, resolved once and stored on the plan.
//!
//! A typed destination was a name and nothing else: `place:<slug>` with null
//! latitude and longitude, which `capabilities/places`' travel layer skips
//! ("only refs that carry a coordinate can be drawn"). So an upcoming trip drew
//! nothing on the map, had no anchor for an accommodation search, no distance to
//! a scouting candidate and no climate for its stage. The coordinate is derived
//! state, so it is resolved here rather than asked of every client.
//!
//! ## The ladder is registry first, geocoder second, and that order is the point
//!
//! `GET /api/places?q=<name>` answers from the registry places already holds,
//! which for a city visited before is instant, offline and free of a new
//! upstream call. `POST /api/geocode` reaches Nominatim at one request per
//! second (`MIN_REQUEST_SPACING` in `capabilities/places/src/geocode.rs`) and
//! writes a registry row, so it is the second rung and not the first.
//!
//! The registry rung requires an **exact** name. `search_places` is a `LIKE
//! '%name%'` over name and city, so "London" answers Bedford Row, Greater London
//! and Hounslow; taking the first row would put a trip's pin in a law office.
//! A rung that has to guess is not a rung, so anything short of an exact name
//! falls through to the geocoder, which is asked the whole name.
//!
//! ## Nothing here is fatal to a save
//!
//! `places` is not a declared requirement of this process (`service.toml`), so
//! a save with places down must store the plan. Every failure leaves the
//! coordinate null and says why in the report; the plan itself keeps the
//! destination the operator typed. `POST /api/plans/:id/resolve-places` is the
//! retry, and the same rung is what an operator runs after the fact.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::store::PlaceRef;

const REGISTRY_TIMEOUT: Duration = Duration::from_secs(3);
/// Longer than the registry: this rung waits on Nominatim, which places spaces
/// one second apart, and a 3 s timeout would fail a slow-but-honest answer.
const GEOCODE_TIMEOUT: Duration = Duration::from_secs(8);
/// What one save may spend resolving, in total. A trip has at most four
/// destinations, so the worst case is four geocoder calls at a second apart; the
/// budget is the backstop for a hung upstream rather than the normal path.
const RESOLVE_BUDGET: Duration = Duration::from_secs(6);

/// Where a coordinate came from. Kept in the report only: the plan stores the
/// numbers, not their provenance, because every reader of a coordinate wants the
/// numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The registry already held a place with exactly this name.
    Registry,
    /// The geocoder answered, and places wrote the row it answered with.
    Geocode,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub place_id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unresolved {
    pub place_id: String,
    pub name: String,
    pub reason: String,
}

/// What one run did. A place that already carried a coordinate appears in
/// neither list: the report is about this run, not about the plan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub resolved: Vec<Resolved>,
    pub unresolved: Vec<Unresolved>,
}

impl Report {
    /// place key → coordinate, for the store to stamp. Keyed on the place NAME,
    /// not its id: an imported plan carries `"id": ""` on its origin and every
    /// destination, so an id-keyed map put London's coordinate on Bonn (measured
    /// 2026-11-03). A name is what the lookup was made with and what a place in
    /// one plan is identified by here.
    pub fn coordinates(&self) -> std::collections::HashMap<String, (f64, f64)> {
        self.resolved
            .iter()
            .map(|place| (place_key(&place.name), (place.latitude, place.longitude)))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty() && self.unresolved.is_empty()
    }
}

/// Resolves the origin and every destination that carries no coordinate, in
/// place, and reports what happened to each.
pub fn stamp_plan(origin: &mut PlaceRef, destinations: &mut [PlaceRef]) -> Report {
    let mut pass = Pass::new();
    let mut report = Report::default();
    pass.stamp_one(origin, &mut report);
    pass.stamp_all(destinations, &mut report);
    report
}

/// One save's worth of resolution: one budget, and one answer per place name.
///
/// The answer is kept, not merely the fact that the name was seen. A round trip
/// names the same place twice — origin `Bonn`, destination `Cologne`, destination
/// `Bonn` — and skipping the second Bonn instead of copying the first one's
/// coordinate left half the stages unlocated.
pub struct Pass {
    deadline: Instant,
    /// The places service. A field rather than a call at each lookup so a test can
    /// answer the ladder without an env var: two tests in one binary racing over
    /// `SJEL_PLACES_URL` would be a flake nobody could read.
    base: String,
    /// Place name → coordinate, or `None` when the lookup failed (the reason is in
    /// the report, once, because a failure for a name is one failure).
    looked_up: std::collections::HashMap<String, Option<(f64, f64)>>,
    /// Look a place up again even when the plan already has a coordinate for it.
    /// A coordinate can be wrong — a geocoder moved, a name meant somewhere else,
    /// and an import once put one destination's coordinate on another — and
    /// without this the only repair would be a hand edit of the database.
    refresh: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass {
    pub fn new() -> Self {
        Self::at(crate::upstream::places_base_url())
    }

    pub fn at(base: impl Into<String>) -> Self {
        Self {
            deadline: Instant::now() + RESOLVE_BUDGET,
            base: base.into(),
            looked_up: std::collections::HashMap::new(),
            refresh: false,
        }
    }

    /// The pass that replaces coordinates the plan already holds.
    pub fn refreshing() -> Self {
        Self {
            refresh: true,
            ..Self::new()
        }
    }

    pub fn stamp_all(&mut self, places: &mut [PlaceRef], report: &mut Report) {
        for place in places.iter_mut() {
            self.stamp_one(place, report);
        }
    }

    pub fn stamp_one(&mut self, place: &mut PlaceRef, report: &mut Report) {
        let located = place.latitude.is_some() && place.longitude.is_some();
        if located && !self.refresh {
            return;
        }
        let key = place_key(&place.name);
        if let Some(found) = self.looked_up.get(&key) {
            apply(place, *found);
            return;
        }
        if place.name.trim().is_empty() {
            report.unresolved.push(Unresolved {
                place_id: place.id.clone(),
                name: place.name.clone(),
                reason: "the place has no name to look up".into(),
            });
            self.looked_up.insert(key, None);
            return;
        }
        match coordinate_at(&self.base, &place.name, self.deadline) {
            Ok((latitude, longitude, source)) => {
                self.looked_up.insert(key, Some((latitude, longitude)));
                apply(place, Some((latitude, longitude)));
                report.resolved.push(Resolved {
                    place_id: place.id.clone(),
                    name: place.name.clone(),
                    latitude,
                    longitude,
                    source,
                });
            }
            Err(reason) => {
                self.looked_up.insert(key, None);
                report.unresolved.push(Unresolved {
                    place_id: place.id.clone(),
                    name: place.name.clone(),
                    reason,
                });
            }
        }
    }
}

/// Writes the answer onto a place, or leaves the place as the operator typed it.
fn apply(place: &mut PlaceRef, found: Option<(f64, f64)>) {
    if let Some((latitude, longitude)) = found {
        place.latitude = Some(latitude);
        place.longitude = Some(longitude);
    }
}

/// Two places with the same name are the same place, in one plan. Case and
/// outer whitespace are not part of the name.
pub fn place_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The ladder itself: exact registry name, then the geocoder.
pub fn coordinate_for(name: &str, deadline: Instant) -> Result<(f64, f64, Source), String> {
    coordinate_at(&crate::upstream::places_base_url(), name, deadline)
}

/// The same ladder against a named service, so a test can answer it itself
/// instead of reaching the deployment's places process.
pub fn coordinate_at(
    base: &str,
    name: &str,
    deadline: Instant,
) -> Result<(f64, f64, Source), String> {
    let name = name.trim();
    let mut reasons: Vec<String> = Vec::new();
    match registry_coordinate(base, name, deadline) {
        Ok(Some(found)) => return Ok((found.0, found.1, Source::Registry)),
        Ok(None) => reasons.push("no registry place has exactly this name".into()),
        Err(reason) => reasons.push(format!("places registry {reason}")),
    }
    match geocode_coordinate(base, name, deadline) {
        Ok(Some(found)) => return Ok((found.0, found.1, Source::Geocode)),
        Ok(None) => reasons.push("the geocoder found nothing".into()),
        Err(reason) => reasons.push(reason),
    }
    Err(reasons.join("; "))
}

fn registry_coordinate(
    base: &str,
    name: &str,
    deadline: Instant,
) -> Result<Option<(f64, f64)>, String> {
    let url = format!("{base}/api/places?q={}", crate::upstream::urlencode(name));
    let body = crate::upstream::get_json_within(&url, REGISTRY_TIMEOUT, deadline)?;
    let rows = body["places"]
        .as_array()
        .ok_or_else(|| "unexpected places shape".to_string())?;
    Ok(exact_match(rows, name))
}

fn geocode_coordinate(
    base: &str,
    name: &str,
    deadline: Instant,
) -> Result<Option<(f64, f64)>, String> {
    let url = format!("{base}/api/geocode");
    let body = crate::upstream::post_json_within(
        &url,
        &serde_json::json!({ "query": name }),
        GEOCODE_TIMEOUT,
        deadline,
    )?;
    Ok(coordinate_of(&body["place"]))
}

/// The registry row whose name is exactly `name`, ignoring case and outer
/// whitespace, and which carries both coordinates.
fn exact_match(rows: &[Value], name: &str) -> Option<(f64, f64)> {
    let wanted = name.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            row["name"]
                .as_str()
                .is_some_and(|found| found.trim().to_lowercase() == wanted)
        })
        .find_map(coordinate_of)
}

/// Both coordinates or nothing: a row with one of them cannot be drawn, and
/// `0.0` is a real coordinate in the Gulf of Guinea rather than a missing value.
pub(crate) fn coordinate_of(place: &Value) -> Option<(f64, f64)> {
    Some((
        place.get("latitude")?.as_f64()?,
        place.get("longitude")?.as_f64()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, latitude: Value, longitude: Value) -> Value {
        serde_json::json!({ "name": name, "latitude": latitude, "longitude": longitude })
    }
    /// The measured failure this rung exists to prevent: `GET /api/places?q=London`
    /// answered Bedford Row, Greater London and Hounslow, and a prefix match would
    /// have pinned the trip at a law office in Holborn.
    #[test]
    fn a_fuzzy_registry_answer_is_not_a_match() {
        let rows = vec![
            row("Bedford Row", 51.5199989.into(), (-0.1161213).into()),
            row("Greater London", 51.5074456.into(), (-0.1277653).into()),
            row("Hounslow", 51.4686132.into(), (-0.3613471).into()),
        ];
        assert_eq!(exact_match(&rows, "London"), None);
    }

    #[test]
    fn an_exact_name_matches_whatever_its_case_and_padding() {
        let rows = vec![
            row("Greater London", 51.5074456.into(), (-0.1277653).into()),
            row("Berkeley", 37.8708393.into(), (-122.272863).into()),
        ];
        assert_eq!(
            exact_match(&rows, "  berkeley "),
            Some((37.8708393, -122.272863))
        );
    }

    /// A row with one coordinate is not a place on a map, and a null is not 0.0.
    #[test]
    fn a_half_located_row_is_no_match() {
        let rows = vec![
            row("Berkeley", 37.8708393.into(), Value::Null),
            row("Berkeley", Value::Null, Value::Null),
        ];
        assert_eq!(exact_match(&rows, "Berkeley"), None);
    }

    /// The geocoder answers `{status, cached, place:{…}}`, and `place` is null
    /// when nothing matched — which is a `not_found` status, not an error.
    #[test]
    fn the_geocoder_reply_is_read_from_the_place_object() {
        let found = serde_json::json!({
            "status": "ok",
            "place": { "place_id": "place:1", "name": "Berkeley", "latitude": 37.87, "longitude": -122.27 }
        });
        assert_eq!(coordinate_of(&found["place"]), Some((37.87, -122.27)));
        let missing = serde_json::json!({ "status": "not_found", "place": null });
        assert_eq!(coordinate_of(&missing["place"]), None);
    }

    /// A places service that answers from a script, so the ladder can be driven
    /// end to end without the deployment's process or an env var: two tests in one
    /// binary racing over `SJEL_PLACES_URL` would be a flake nobody could read.
    fn stub(
        routes: Vec<(&'static str, u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let asked = routes.len();
        stub_for(routes, asked)
    }

    fn stub_for(
        routes: Vec<(&'static str, u16, String)>,
        asked: usize,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..asked {
                let (mut stream, _) = listener.accept().expect("accept");
                let request = read_request(&mut stream);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default()
                    .to_string();
                let (status, body) = routes
                    .iter()
                    .find(|(prefix, _, _)| path.starts_with(prefix))
                    .map(|(_, status, body)| (*status, body.clone()))
                    .unwrap_or((404, "{}".into()));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                use std::io::Write;
                let _ = stream.write_all(reply.as_bytes());
                requests.push(path);
            }
            requests
        });
        (base, handle)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => break,
            }
        }
        let head = String::from_utf8_lossy(&head).to_string();
        let length: usize = head
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0);
        if length > 0 {
            let mut body = vec![0u8; length];
            let _ = stream.read_exact(&mut body);
        }
        head
    }

    #[test]
    fn an_exact_registry_place_answers_without_asking_the_geocoder() {
        // One route and one expected request: the ladder must stop at the first rung.
        let (base, server) = stub(vec![(
            "/api/places",
            200,
            serde_json::json!({
                "places": [row("Berkeley", 37.8708393.into(), (-122.272863).into())]
            })
            .to_string(),
        )]);
        let found = coordinate_at(&base, " Berkeley ", Instant::now() + RESOLVE_BUDGET);
        assert_eq!(found, Ok((37.8708393, -122.272863, Source::Registry)));
        let asked = server.join().expect("the stub thread finishes");
        assert_eq!(asked.len(), 1, "the geocoder was asked as well: {asked:?}");
    }

    /// The measured registry answer for "London" holds no place called London, so
    /// the geocoder is asked the whole name — and its answer is the one stored.
    #[test]
    fn a_registry_miss_asks_the_geocoder_the_whole_name() {
        let (base, server) = stub(vec![
            (
                "/api/places",
                200,
                serde_json::json!({
                    "places": [row("Greater London", 51.5074456.into(), (-0.1277653).into())]
                })
                .to_string(),
            ),
            (
                "/api/geocode",
                200,
                serde_json::json!({
                    "status": "ok",
                    "cached": false,
                    "place": { "place_id": "place:2", "name": "London", "latitude": 51.5074, "longitude": -0.1278 }
                })
                .to_string(),
            ),
        ]);
        let found = coordinate_at(&base, "London", Instant::now() + RESOLVE_BUDGET);
        assert_eq!(found, Ok((51.5074, -0.1278, Source::Geocode)));
        let asked = server.join().expect("the stub thread finishes");
        assert!(
            asked[1].starts_with("/api/geocode"),
            "the second rung was not the geocoder: {asked:?}"
        );
    }

    /// Both rungs down is a named reason and not a silent null: this string is what
    /// `POST /api/plans/:id/resolve-places` puts in `unresolved[].reason`.
    #[test]
    fn both_rungs_down_names_both_reasons() {
        let (base, _server) = stub(vec![
            ("/api/places", 503, "{}".into()),
            ("/api/geocode", 502, "{}".into()),
        ]);
        let error = coordinate_at(&base, "Berkeley", Instant::now() + RESOLVE_BUDGET)
            .expect_err("neither rung answered");
        assert!(error.contains("places registry answered 503"), "{error}");
        assert!(error.contains("answered 502"), "{error}");
    }

    /// A round trip names one place twice — origin `Bonn`, destination `Cologne`,
    /// destination `Bonn`. The second Bonn must take the first mention's answer
    /// instead of being skipped, which left half the stages unlocated.
    #[test]
    fn a_repeated_place_takes_the_answer_its_first_mention_got() {
        // Three lookups: Bonn from the registry, Cologne from the registry (no
        // exact match) and then from the geocoder. "bonn " is answered from the pass's
        // own memory.
        let (base, server) = stub_for(
            vec![
                (
                    "/api/places",
                    200,
                    serde_json::json!({
                        "places": [row("Bonn", 50.7352621.into(), 7.1024635.into())]
                    })
                    .to_string(),
                ),
                (
                    "/api/geocode",
                    200,
                    serde_json::json!({
                        "status": "ok",
                        "place": { "name": "Cologne", "latitude": 50.9375, "longitude": 6.9603 }
                    })
                    .to_string(),
                ),
            ],
            3,
        );
        let mut pass = Pass::at(&base);
        let mut report = Report::default();
        let mut origin = unlocated("Bonn");
        pass.stamp_one(&mut origin, &mut report);
        let mut destinations = [unlocated("Cologne"), unlocated("bonn ")];
        for destination in destinations.iter_mut() {
            pass.stamp_one(destination, &mut report);
        }

        assert_eq!(
            (destinations[1].latitude, destinations[1].longitude),
            (Some(50.7352621), Some(7.1024635)),
            "the repeated Bonn was skipped instead of taking the first answer"
        );
        assert_eq!(destinations[0].latitude, Some(50.9375));
        assert_eq!(
            report.resolved.len(),
            2,
            "one report entry per place looked up, not per mention"
        );
        assert_eq!(
            server.join().expect("the stub thread finishes").len(),
            3,
            "three mentions, three lookups: Bonn once, Cologne twice (registry, then geocoder)"
        );
    }

    fn unlocated(name: &str) -> PlaceRef {
        PlaceRef {
            id: format!("place:{name}"),
            name: name.into(),
            kind: crate::store::PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    #[test]
    fn a_place_that_already_has_a_coordinate_is_not_looked_up() {
        let mut origin = PlaceRef {
            id: "place:cgn".into(),
            name: "Cologne Bonn Airport (CGN)".into(),
            kind: crate::store::PlaceKind::City,
            address: None,
            latitude: Some(50.86),
            longitude: Some(7.14),
        };
        let mut destinations = Vec::new();
        let report = stamp_plan(&mut origin, &mut destinations);
        assert!(
            report.is_empty(),
            "the report is about this run, not the plan"
        );
    }
}

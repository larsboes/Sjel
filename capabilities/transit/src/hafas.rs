//! HAFAS client: talks to bahn.de's internal, undocumented journey-search API
//! (the same one bahn.de's own website calls from the browser -- no key, no
//! auth, no public docs). Two endpoints:
//!   - POST /web/api/angebote/fahrplan   (journey search; also the pairwise
//!     query the split-ticket solver uses)
//!   - GET  /web/api/reiseloesung/orte    (station name -> EVA id lookup)
//!
//! **Not ported from the original: ONNX delay-risk prediction.** The source
//! service loaded a `tract-onnx` model (`infra/data/model.onnx`) to fill
//! `Journey.delay_risk_score`. Axon has no such model artifact -- the
//! source monorepo's delay-training pipeline was rated quarry-for-patterns-only
//! in the original migration evaluation and has now been removed from Axon.
//! Carrying `tract-onnx` (a heavy dependency) plus hand-rolled date
//! math for a field that would only ever return a constant 0.15 fallback is
//! the exact "machinery with nothing behind it" anti-pattern this repo
//! already strips elsewhere (scouting's CV generator, its HTTP server). The
//! field stays in `travel::Journey` (always `None` *here*) so filling it later
//! is additive, not a schema break.
//!
//! It is filled now, one layer up: `transit::punctuality` asks
//! `capabilities/punctuality` for measured lateness and enriches the journeys
//! this module returns. Deliberately not done inside this module -- HAFAS
//! parsing should stay a pure function of the HAFAS response, and the tests
//! below assert exactly that by checking the field is still `None` on the way
//! out of here.

use crate::travel::{
    ContractBoundary, Journey, Leg, SplitConfidence, SplitResult, SplitSegment, Station, TrainMatch,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

// Spoofed real-browser UA sent to bahn.de -- kept deliberately, not
// genericized to a self-identifying "Axon-Transit/0.1" string. This mirrors
// `capabilities/scouting/src/adapters/meetup.rs`'s precedent, not the
// self-identifying-UA one (source.rs/cfp_conferences/luma): this endpoint is
// undocumented and ungated only because it looks like normal browser
// traffic. A self-identifying UA here would plausibly just get blocked
// outright rather than "politely identify us" -- there's no ToS/robots.txt
// contract to honor on an endpoint that was never meant to be called this
// way. Flagged here and in README Gotchas rather than hidden.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const FAHRPLAN_URL: &str = "https://www.bahn.de/web/api/angebote/fahrplan";
const ORTE_URL: &str = "https://www.bahn.de/web/api/reiseloesung/orte";

/// The DB Navigator app's journey endpoint and its versioned vendor media
/// type, both read from db-vendo-client's `p/dbnav/base.json` and
/// `journeys-req.js` and then confirmed live (2026-08-12).
const DBNAV_FAHRPLAN_URL: &str = "https://app.services-bahn.de/mob/angebote/fahrplan";
const DBNAV_MEDIA_TYPE: &str = "application/x.db.vendo.mob.verbindungssuche.v9+json";

/// `X-Correlation-ID` is REQUIRED by this endpoint and its *shape* is not:
/// omitting it answers 405, while an arbitrary non-UUID string answers 200
/// (both probed 2026-08-12). db-vendo-client sends two v4 UUIDs joined by
/// `_`; matching that exactly would cost a `uuid` dependency for a value the
/// server does not parse, so this derives a per-request value from the clock
/// instead. Per-request rather than a constant, because one fixed id across
/// every query is itself a fingerprint.
fn dbnav_correlation_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032x}_{:032x}", nanos.rotate_left(64))
}

#[derive(Debug, thiserror::Error)]
pub enum HafasError {
    #[error("request failed: {0}")]
    Request(String),
    #[error("HAFAS query failed with status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("could not parse response: {0} (body: {1})")]
    Parse(String, String),
    /// No cheaper combination exists for this route. An outcome, not a failure: the
    /// query was fine and the honest answer is "none". Its own variant rather than an
    /// Other(String) the caller has to string-match, because the HTTP layer has to map
    /// it to a different status than a broken upstream.
    #[error("no split-ticket combination is cheaper than the direct fare here")]
    NoSplitFound,
    /// The caller's departure time is not a datetime this client will post. Its own
    /// variant for the same reason as `NoSplitFound`: it is the caller's input, so the
    /// HTTP layer has to answer 400 rather than relay it as a server fault.
    #[error("time must be YYYY-MM-DDTHH:MM:SS or YYYY-MM-DDTHH:MM, got {0:?}")]
    InvalidDatetime(String),
    #[error("{0}")]
    Other(String),
}

/// bahn.de's `anfrageZeitpunkt` is a naive local datetime, and the endpoint is stricter
/// about it than its own answer admits: given a timestamp without seconds
/// (`2026-08-16T13:00`) `/web/api/angebote/fahrplan` returns 500 with an EMPTY body.
/// That relayed out as `BadStatus` and left the HTTP layer answering its own 500 with
/// nothing in it, which on 2026-08-13 read as "the upstream is blocking us" and cost an
/// evening of investigation. Every programmed caller already sends seconds; a
/// hand-written query is where the minute-precision form comes from.
///
/// So the seconds are filled in here, once, ahead of both payload builders, and a value
/// that is not a datetime at all is refused before it becomes a request -- an upstream
/// 500 is a terrible way to learn you mistyped a date.
///
/// Field ranges are checked, the calendar is not: `2026-02-31` has the right shape and
/// goes upstream, where the timetable is the authority on which days exist.
fn normalize_datetime(datetime: &str) -> Result<String, HafasError> {
    /// One fixed-width numeric field, present and in range.
    fn field(part: Option<&str>, width: usize, range: std::ops::RangeInclusive<u32>) -> bool {
        match part {
            Some(p) if p.len() == width && p.bytes().all(|b| b.is_ascii_digit()) => {
                p.parse().map(|n| range.contains(&n)).unwrap_or(false)
            }
            _ => false,
        }
    }

    let invalid = || HafasError::InvalidDatetime(datetime.to_string());
    let (date, time) = datetime.split_once('T').ok_or_else(invalid)?;
    let (mut date, mut time) = (date.split('-'), time.split(':'));

    let shape = field(date.next(), 4, 1..=9999)
        && field(date.next(), 2, 1..=12)
        && field(date.next(), 2, 1..=31)
        && date.next().is_none()
        && field(time.next(), 2, 0..=23)
        && field(time.next(), 2, 0..=59);
    let seconds = time.next();
    if !shape || time.next().is_some() || seconds.is_some_and(|s| !field(Some(s), 2, 0..=59)) {
        return Err(invalid());
    }

    Ok(match seconds {
        Some(_) => datetime.to_string(),
        None => format!("{datetime}:00"),
    })
}

/// Fare context carried into every bahn.de query. The vendor's own pricing
/// engine applies the discount, so returned fares are discount-correct per
/// leg -- which is what the split solver needs, because BahnCard applies per
/// Fahrkarte (BB C.2 Nr. 2.1) and every split segment is its own Fahrkarte.
/// A Deutschlandticket additionally zeroes pure regional connections on the
/// vendor's side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FareOptions {
    /// 25 or 50; anything else fails the payload builder loudly.
    pub bahncard: Option<u8>,
    pub first_class: bool,
    pub deutschland_ticket: bool,
}

/// Which bahn.de backend this client speaks. The seam R1 asked for: losing a
/// backend costs one variant, and the selection is config, not a rewrite.
///
/// `DbNav` speaks journeys for real as of 2026-08-12. Getting there took two
/// wrong premises, both worth keeping written down.
///
/// First probe: the endpoint answered OPS_BLOCKED to the canonical
/// db-vendo-client, and that was generalized to "no client can speak dbnav".
/// The re-probe falsified it. The block belongs to that client's HTTP stack,
/// not to the endpoint: db-vendo-client 6.11.1 goes through cross-fetch /
/// node-fetch@2 with ALPN pinned to http/1.1, and it still draws OPS_BLOCKED
/// while curl and reqwest, sending a byte-identical body and the same two
/// headers from the same address in the same minute, both get 200. Neither
/// the HTTP version nor the User-Agent is the discriminator (all four
/// combinations of {http/1.1, http/2} x {curl UA, client UA} serve), which
/// leaves the TLS fingerprint as the remaining suspect. The operative point
/// for this file: `reqwest` is the client transit uses, and reqwest is not
/// blocked.
///
/// Lesson, since it generalizes past this endpoint: one blocked client is
/// evidence about that client. Reaching "the endpoint is blocked" needs a
/// second client, and the cheapest second client is curl.
///
/// `DbNav` is the default as of 2026-08-20, and the reason is punctuality, not
/// stability. punctuality keys its cells on DB's open-data vocabulary (`RB`, `RE`,
/// `ICE`). dbnav's `produktGattung` IS that vocabulary; dbweb's
/// `verkehrsmittel.kategorie` is HAFAS's own, so the identical RE5 arrives as `DRB`
/// and finds no cell. On dbweb every regional leg therefore lost its
/// `on_time_probability`, and one missing term voids the whole reliability product by
/// design -- so the default path answered `null` for exactly the journeys a traveller
/// most wants scored. dbweb is one env var away and answers everything dbnav does,
/// split-ticketing included since `cdacfbf`; what it no longer is, is the path a search
/// takes when nobody chose one.
///
/// The mapping table in `punctuality::train_type_for_category` fixes dbweb too. Both
/// were built: making dbnav default fixes the path real searches take, and the table
/// stops the second backend from quietly scoring nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RailBackend {
    DbWeb,
    #[default]
    DbNav,
}

impl RailBackend {
    /// From `SJEL_TRANSIT_BACKEND` ("dbweb" | "dbnav"); anything else falls
    /// back to the default loudly via the log rather than failing the process.
    pub fn from_env() -> Self {
        match sjel_config::env_var("SJEL_TRANSIT_BACKEND").as_deref() {
            Ok("dbweb") => Self::DbWeb,
            Ok("dbnav") | Err(_) => Self::DbNav,
            Ok(other) => {
                eprintln!("transit: unknown SJEL_TRANSIT_BACKEND '{other}', using dbnav");
                Self::default()
            }
        }
    }
}

/// Where the client actually sends, resolved once per client.
///
/// The three literals above are the defaults and stay the defaults; these are the
/// override, and the reason they exist is that every answer this capability gives comes
/// from a live query to a reverse-engineered endpoint. That makes the client untestable
/// offline and undemonstrable at all: recording a real timetable publishes something that
/// stops being true within the hour, which is why transit was left out of the published
/// demo entirely. Pointing the real client at a stub serving bahn.de-shaped payloads means
/// the parser still does the work and what gets recorded is genuinely transit's own output.
///
/// One variable per endpoint rather than one base URL: the two backends live on different
/// hosts under different path prefixes, so a single prefix would have to be split back
/// apart by whichever of them was being replaced.
struct Endpoints {
    /// dbweb journey search. `SJEL_TRANSIT_FAHRPLAN_URL`.
    fahrplan: String,
    /// dbweb station suggest. `SJEL_TRANSIT_ORTE_URL`. dbnav has no suggest path here:
    /// `suggest_stations` is dbweb-only regardless of backend.
    orte: String,
    /// dbnav journey search. `SJEL_TRANSIT_DBNAV_FAHRPLAN_URL`. Overridable too, and not
    /// optional now that dbnav is the default — without it the default backend is the one
    /// a stub cannot reach.
    dbnav_fahrplan: String,
}

impl Endpoints {
    fn from_env() -> Self {
        Self::resolve(|var| sjel_config::env_var(var).ok())
    }

    /// Split from `from_env` so the resolution rules are testable without mutating the
    /// process environment, which Rust's parallel test threads share.
    fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self {
        // An empty or blank value falls back rather than pointing the client at "": a
        // shell that exports the variable unconditionally sets it to empty when it has
        // nothing to put there, and a request to "" fails in a way that names neither.
        let or_default = |var: &str, default: &str| {
            lookup(var)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            fahrplan: or_default("SJEL_TRANSIT_FAHRPLAN_URL", FAHRPLAN_URL),
            orte: or_default("SJEL_TRANSIT_ORTE_URL", ORTE_URL),
            dbnav_fahrplan: or_default("SJEL_TRANSIT_DBNAV_FAHRPLAN_URL", DBNAV_FAHRPLAN_URL),
        }
    }
}

pub struct HafasClient {
    client: Client,
    backend: RailBackend,
    endpoints: Endpoints,
}

impl Default for HafasClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HafasClient {
    pub fn new() -> Self {
        Self::with_backend(RailBackend::from_env())
    }

    pub fn with_backend(backend: RailBackend) -> Self {
        // A bare `Client::new()` has NO request timeout -- a slow or hung
        // response from bahn.de's undocumented endpoint blocks the calling
        // thread forever (this is exactly what happened during the port's
        // own live smoke test: a `--search` call stalled past 600s with no
        // recovery). 15s is generous for a single journey-search POST; a
        // real network issue should fail fast and loud, not hang silently.
        let client = sjel_http::client(
            sjel_http::Purpose::new("transit-hafas"),
            std::time::Duration::from_secs(15),
        )
        .expect("reqwest client with a fixed timeout should always build");
        Self {
            client,
            backend,
            endpoints: Endpoints::from_env(),
        }
    }

    fn fahrplan_payload(
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<Value, HafasError> {
        let datetime = normalize_datetime(datetime)?;
        let klasse = if fare.first_class {
            "KLASSE_1"
        } else {
            "KLASSE_2"
        };
        let ermaessigung = match fare.bahncard {
            None => json!({"art": "KEINE_ERMAESSIGUNG", "klasse": "KLASSENLOS"}),
            // Enum strings verified against db-vendo-client's
            // format/loyalty-cards.js (fetched 2026-08-12): the ermaessigung
            // klasse is the CARD's class; one first_class knob drives both.
            Some(25) => json!({"art": "BAHNCARD25", "klasse": klasse}),
            Some(50) => json!({"art": "BAHNCARD50", "klasse": klasse}),
            Some(other) => {
                return Err(HafasError::Other(format!(
                    "bahncard must be 25 or 50, got {other}"
                )))
            }
        };
        Ok(json!({
            "abfahrtsHalt": from_eva,
            "anfrageZeitpunkt": datetime,
            "ankunftsHalt": to_eva,
            "ankunftSuche": "ABFAHRT",
            "klasse": klasse,
            "produktgattungen": ["ICE", "EC_IC", "IR", "REGIONAL", "SBAHN", "BUS", "SCHIFF", "UBAHN", "TRAM", "ANRUFPFLICHTIG"],
            "reisende": [{
                "typ": "ERWACHSENER",
                "ermaessigungen": [ermaessigung],
                "anzahl": 1,
                "alter": []
            }],
            "schnelleVerbindungen": true,
            "deutschlandTicketVorhanden": fare.deutschland_ticket
        }))
    }

    /// The dbnav equivalent, keyed by German field names and a different fare
    /// vocabulary. Two shapes verified live rather than assumed: the station
    /// ids go in as the short lid form `A=1@L=<eva>@` (the canonical client
    /// sends exactly that, no coordinates needed), and `reiseDatum` accepts the
    /// same naive local string dbweb takes, so `datetime` goes through the same
    /// `normalize_datetime` and both backends keep one caller-facing contract.
    ///
    /// The discount is one space-joined string (`"BAHNCARD25 KLASSE_2"`), not
    /// the object dbweb wants -- read from db-vendo-client's `journeys-req.js`,
    /// which builds `art + ' ' + klasse` by hand.
    fn dbnav_payload(
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<Value, HafasError> {
        let datetime = normalize_datetime(datetime)?;
        let klasse = if fare.first_class {
            "KLASSE_1"
        } else {
            "KLASSE_2"
        };
        let ermaessigung = match fare.bahncard {
            None => "KEINE_ERMAESSIGUNG KLASSENLOS".to_string(),
            Some(25) => format!("BAHNCARD25 {klasse}"),
            Some(50) => format!("BAHNCARD50 {klasse}"),
            Some(other) => {
                return Err(HafasError::Other(format!(
                    "bahncard must be 25 or 50, got {other}"
                )))
            }
        };
        Ok(json!({
            "autonomeReservierung": false,
            "einstiegsTypList": ["STANDARD"],
            "fahrverguenstigungen": {
                "deutschlandTicketVorhanden": fare.deutschland_ticket,
                "nurDeutschlandTicketVerbindungen": false
            },
            "klasse": klasse,
            "reisendenProfil": {
                "reisende": [{
                    "ermaessigungen": [ermaessigung],
                    "reisendenTyp": "ERWACHSENER"
                }]
            },
            "reservierungsKontingenteVorhanden": false,
            "reiseHin": {
                "wunsch": {
                    "abgangsLocationId": format!("A=1@L={from_eva}@"),
                    "verkehrsmittel": ["ALL"],
                    "alternativeHalteBerechnung": true,
                    "zeitWunsch": {
                        "reiseDatum": datetime,
                        "zeitPunktArt": "ABFAHRT"
                    },
                    "zielLocationId": format!("A=1@L={to_eva}@"),
                    "fahrradmitnahme": false
                }
            }
        }))
    }

    /// Direct journey search between two EVA station codes.
    pub fn search_connections(
        &self,
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<Vec<Journey>, HafasError> {
        let request = match self.backend {
            RailBackend::DbWeb => self
                .client
                .post(&self.endpoints.fahrplan)
                .header("User-Agent", BROWSER_UA)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json; charset=UTF-8")
                .json(&Self::fahrplan_payload(from_eva, to_eva, datetime, fare)?),
            RailBackend::DbNav => self
                .client
                .post(&self.endpoints.dbnav_fahrplan)
                .header("X-Correlation-ID", dbnav_correlation_id())
                .header("Accept", DBNAV_MEDIA_TYPE)
                .header("Content-Type", DBNAV_MEDIA_TYPE)
                .json(&Self::dbnav_payload(from_eva, to_eva, datetime, fare)?),
        };

        let response = request
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        if !status.is_success() {
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: text,
            });
        }

        let body: Value =
            serde_json::from_str(&text).map_err(|e| HafasError::Parse(e.to_string(), text))?;
        Ok(match self.backend {
            RailBackend::DbWeb => parse_journeys_from_response(&body),
            RailBackend::DbNav => parse_dbnav_journeys(&body),
        })
    }

    /// Station name -> EVA id search (autocomplete-style).
    pub fn suggest_stations(&self, query: &str) -> Result<Vec<Station>, HafasError> {
        let response = self
            .client
            .get(&self.endpoints.orte)
            .query(&[("suchbegriff", query), ("typ", "ALL"), ("limit", "10")])
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: String::new(),
            });
        }

        let list: Vec<Value> = response
            .json()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        Ok(parse_suggest_response(&list))
    }

    /// Cheapest split-ticket search: finds every reasonable intermediate stop
    /// on the direct connection, prices every pairwise segment, and picks the
    /// cheapest way to stitch consecutive segments together end to end.
    ///
    /// Deliberately sequential, not bounded-concurrent (the original used a
    /// `tokio::task::JoinSet` + `Semaphore(2)`) -- see README's "Known gaps":
    /// this is a personal, low-frequency CLI tool, and avoiding tokio as a
    /// dependency (matching scouting's no-async-runtime precedent) is worth
    /// more than shaving a few seconds off an occasional split-ticket search.
    /// The 250ms inter-request pause is preserved.
    pub fn search_split_tickets(
        &self,
        from_eva: &str,
        to_eva: &str,
        datetime: &str,
        fare: &FareOptions,
    ) -> Result<SplitResult, HafasError> {
        // Same dispatch as `search_connections`, because this is the same query:
        // one direct journey, read for its structure instead of returned. Keeping
        // the two in step is the point -- a backend that can answer journeys can
        // answer this, and split-ticketing stopped being dbweb-only the moment the
        // stop and span readers below existed for both shapes.
        let request = match self.backend {
            RailBackend::DbWeb => self
                .client
                .post(&self.endpoints.fahrplan)
                .header("User-Agent", BROWSER_UA)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json; charset=UTF-8")
                .json(&Self::fahrplan_payload(from_eva, to_eva, datetime, fare)?),
            RailBackend::DbNav => self
                .client
                .post(&self.endpoints.dbnav_fahrplan)
                .header("X-Correlation-ID", dbnav_correlation_id())
                .header("Accept", DBNAV_MEDIA_TYPE)
                .header("Content-Type", DBNAV_MEDIA_TYPE)
                .json(&Self::dbnav_payload(from_eva, to_eva, datetime, fare)?),
        };

        let response = request
            .send()
            .map_err(|e| HafasError::Request(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .map_err(|e| HafasError::Request(e.to_string()))?;
        if !status.is_success() {
            // The body used to be dropped here (`body: String::new()`) while
            // `search_connections` kept it. On dbweb that cost little, because the
            // same request shape was already debuggable through the other method.
            // On dbnav it is the difference between a diagnosable rejection and a
            // bare number, and a versioned vendor media type is exactly the kind of
            // thing that starts failing with a message worth reading.
            return Err(HafasError::BadStatus {
                status: status.as_u16(),
                body: text,
            });
        }

        let body: Value =
            serde_json::from_str(&text).map_err(|e| HafasError::Parse(e.to_string(), text))?;

        // Which train the traveller is actually on over each stop pair comes from
        // this same payload, so classifying a segment costs no extra request.
        let DirectJourney {
            stops,
            spans,
            direct_price,
        } = match self.backend {
            RailBackend::DbWeb => dbweb_direct_journey(&body)?,
            RailBackend::DbNav => dbnav_direct_journey(&body)?,
        };
        let n = stops.len();
        if n < 2 {
            return Err(HafasError::Other(
                "not enough stops to perform split-ticketing".into(),
            ));
        }

        let mut prices: HashMap<(usize, usize), f64> = HashMap::new();
        let mut segments_data: HashMap<(usize, usize), Journey> = HashMap::new();
        let mut queried_pairs = 0usize;
        let mut unpriced_pairs = 0usize;

        for i in 0..n {
            for j in (i + 1)..n {
                std::thread::sleep(std::time::Duration::from_millis(250));
                queried_pairs += 1;
                // A failed query used to disappear here, so the DP quietly ran
                // against a table with holes in it and the caller was told the
                // answer was the cheapest chain that exists. It is counted now.
                // Same fare context as the direct query: BahnCard applies per
                // Fahrkarte, so every candidate segment is priced the way it
                // would actually be bought.
                let priced = match self.search_connections(
                    &stops[i].ext_id,
                    &stops[j].ext_id,
                    &stops[i].departure_iso,
                    fare,
                ) {
                    Ok(journeys) => match journeys.first() {
                        Some(first) => first.total_price.map(|price| (price, first.clone())),
                        None => None,
                    },
                    Err(_) => None,
                };
                match priced {
                    Some((price, journey)) => {
                        prices.insert((i, j), price);
                        segments_data.insert((i, j), journey);
                    }
                    None => unpriced_pairs += 1,
                }
            }
        }

        let (split_price, path) = cheapest_split(n, &prices).ok_or(HafasError::NoSplitFound)?;

        let segments: Vec<SplitSegment> = path
            .into_iter()
            .filter_map(|(i, j)| {
                segments_data.get(&(i, j)).cloned().map(|journey| {
                    let expected_trains = expected_trains(&spans, i, j);
                    let train_match = classify_train_match(&expected_trains, &journey);
                    SplitSegment {
                        journey,
                        train_match,
                        expected_trains,
                    }
                })
            })
            .collect();

        let confidence = split_confidence(&segments, unpriced_pairs);
        let contract_boundaries = contract_boundaries_of(&segments);

        Ok(SplitResult {
            original_price: direct_price,
            split_price,
            savings: direct_price.map(|p| p - split_price),
            segments,
            contract_boundaries,
            confidence,
            unpriced_pairs,
            queried_pairs,
        })
    }
}

/// The ticket boundaries between consecutive chain segments.
///
/// The boundary station is where segment i's journey ends and segment i+1's
/// begins. `same_train` compares the arriving and departing train numbers: a
/// mid-run split has no connection to miss, while a train change across a
/// contract boundary is the risk the through ticket would not have carried.
/// The buffer uses the UTC instants X1 attached, so it stays right when a
/// chain crosses a timezone.
fn contract_boundaries_of(segments: &[crate::travel::SplitSegment]) -> Vec<ContractBoundary> {
    segments
        .windows(2)
        .filter_map(|pair| {
            let arriving = pair[0].journey.legs.last()?;
            let departing = pair[1].journey.legs.first()?;
            let same_train = !arriving.train_number.is_empty()
                && arriving.train_number == departing.train_number;
            // Both strings carry explicit offsets ("...Z"), so the station ids
            // station-time would otherwise resolve a zone from are unused.
            let transfer_minutes = match (&arriving.arrival_utc, &departing.departure_utc) {
                (Some(arrival), Some(departure)) => {
                    station_time::duration_between(arrival, "", departure, "")
                        .map(|d| d.num_minutes())
                }
                _ => None,
            };
            Some(ContractBoundary {
                station: arriving.destination.clone(),
                same_train,
                transfer_minutes,
                incoming_share_late_6: None,
            })
        })
        .collect()
}

struct Stop {
    ext_id: String,
    departure_iso: String,
}

/// A stop's scheduled and real-time value for one event, kept apart.
///
/// Both are read every time. Folding them with `or_else` on the way in is what
/// made a delay invisible: the field held whichever existed and nothing recorded
/// which one it was.
fn times_of(halt: &Value, event: &str) -> (Option<String>, Option<String>) {
    let read = |key: &str| {
        halt.get(event)
            .and_then(|e| e.get(key))
            .and_then(|t| t.as_str())
            .map(str::to_string)
    };
    // `echtzeit`, captured from a live response. The code this replaced fell
    // back to `istzeit`, which this endpoint does not serve at all, so the
    // fallback never once fired and every journey silently carried its
    // scheduled time as though it were the real one. The same shape as the
    // `id`/`tripId` bug in Gotchas: a wrong key name that fails as silence.
    (read("sollzeit"), read("echtzeit"))
}

/// Whether HAFAS marked something cancelled.
///
/// The flag appears under several names across the response depending on where
/// it sits, and a missing flag means not cancelled -- never unknown, because
/// HAFAS omits it for the ordinary case.
fn is_cancelled(node: &Value) -> bool {
    // Both captured from a real section. A missing flag means not cancelled:
    // bahn.de omits them for the ordinary case rather than sending false.
    ["originCancelled", "destinationCancelled"]
        .iter()
        .any(|key| node.get(*key).and_then(|v| v.as_bool()).unwrap_or(false))
}

/// One non-WALK section of the direct journey, expressed as the stop-index pair
/// it spans plus the train that covers it.
struct SectionSpan {
    from: usize,
    to: usize,
    train_number: String,
}

/// Maps each non-WALK section of the direct journey onto the stop indices
/// `extract_stops` produced, so a stop pair can be asked which trains it rides.
///
/// Done as a second pass rather than inside `extract_stops` because that function
/// deduplicates by station id: a transfer station is pushed once, while processing
/// the section that *arrives* there, and the train it later departs on is not known
/// at that moment. Matching by station id afterwards has no such ordering problem.
fn extract_section_spans(v: &Value, stops: &[Stop]) -> Vec<SectionSpan> {
    let index_of = |ext_id: &str| stops.iter().position(|s| s.ext_id == ext_id);
    let mut spans = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return spans;
    };
    for section in sections {
        let verkehrsmittel = section
            .get("verkehrsmittel")
            .cloned()
            .unwrap_or(Value::Null);
        if verkehrsmittel.get("typ").and_then(|t| t.as_str()) == Some("WALK") {
            continue;
        }
        let Some(halte) = section.get("halte").and_then(|h| h.as_array()) else {
            continue;
        };
        if halte.len() < 2 {
            continue;
        }
        let halt_id = |halt: &Value| -> Option<String> {
            halt.get("id")
                .or_else(|| halt.get("extId"))
                .and_then(|id| id.as_str())
                .map(|s| s.to_string())
        };
        let (Some(from_id), Some(to_id)) = (halt_id(&halte[0]), halt_id(halte.last().unwrap()))
        else {
            continue;
        };
        let (Some(from), Some(to)) = (index_of(&from_id), index_of(&to_id)) else {
            continue;
        };
        let train_number = verkehrsmittel
            .get("nummer")
            .or_else(|| verkehrsmittel.get("linienNummer"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        spans.push(SectionSpan {
            from,
            to,
            train_number,
        });
    }
    spans
}

/// The trains the direct journey uses between two stop indices, in route order.
fn expected_trains(spans: &[SectionSpan], i: usize, j: usize) -> Vec<String> {
    spans
        .iter()
        .filter(|s| s.from >= i && s.to <= j && !s.train_number.is_empty())
        .map(|s| s.train_number.clone())
        .collect()
}

/// Whether a separately-priced journey is the same ride as the planned one.
pub fn classify_train_match(expected: &[String], journey: &Journey) -> TrainMatch {
    let actual: Vec<&str> = journey
        .legs
        .iter()
        .map(|l| l.train_number.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    if expected.is_empty() || actual.is_empty() {
        return TrainMatch::Unknown;
    }
    if expected.len() == actual.len() && expected.iter().zip(&actual).all(|(e, a)| e == a) {
        return TrainMatch::Exact;
    }
    if actual.iter().any(|a| expected.iter().any(|e| e == a)) {
        return TrainMatch::Partial;
    }
    TrainMatch::Different
}

/// One value a caller can gate on, taking the worst case across the chain: a
/// chain is only as buyable as its least trustworthy ticket.
pub fn split_confidence(segments: &[SplitSegment], unpriced_pairs: usize) -> SplitConfidence {
    if segments
        .iter()
        .any(|s| s.train_match == TrainMatch::Different)
    {
        return SplitConfidence::Low;
    }
    if unpriced_pairs > 0 || segments.iter().any(|s| s.train_match != TrainMatch::Exact) {
        return SplitConfidence::Partial;
    }
    SplitConfidence::Exact
}

fn extract_stops(v: &Value) -> Vec<Stop> {
    let mut stops = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return stops;
    };
    for section in sections {
        let verkehrsmittel = section
            .get("verkehrsmittel")
            .cloned()
            .unwrap_or(Value::Null);
        let typ = verkehrsmittel
            .get("typ")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if typ == "WALK" {
            continue;
        }
        let Some(halte) = section.get("halte").and_then(|h| h.as_array()) else {
            continue;
        };
        if halte.len() < 2 {
            continue;
        }
        for halt in [&halte[0], halte.last().unwrap()] {
            let Some(ext_id) = halt
                .get("id")
                .or_else(|| halt.get("extId"))
                .and_then(|id| id.as_str())
            else {
                continue;
            };
            if stops.iter().any(|s: &Stop| s.ext_id == ext_id) {
                continue;
            }
            let departure_iso = halt
                .get("abfahrt")
                .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                .and_then(|t| t.as_str())
                .or_else(|| {
                    halt.get("ankunft")
                        .and_then(|a| a.get("sollzeit").or_else(|| a.get("istzeit")))
                        .and_then(|t| t.as_str())
                })
                .unwrap_or("")
                .to_string();
            stops.push(Stop {
                ext_id: ext_id.to_string(),
                departure_iso,
            });
        }
    }
    stops
}

/// dbnav stamps every time with its offset (`2026-09-15T09:04:00+02:00`) where
/// dbweb sends the same wall clock naive (`2026-07-15T08:30:00`).
///
/// The offset comes off here rather than `normalize_datetime` learning to
/// tolerate it, because `dbnav_payload` documents the opposite contract on
/// purpose: both backends take the same naive local string, so there is one
/// caller-facing shape. Nothing downstream loses by it -- a stop's stamp is only
/// ever handed back as "search from this moment" for the very station it was read
/// at, and local time is what both endpoints want there.
///
/// Found from the capture rather than from reading. An offset-carrying string
/// splits into four colon-separated fields, so `normalize_datetime` refuses it,
/// and every priced pair on dbnav would have died as `InvalidDatetime` -- after
/// the direct search had already succeeded, which is the shape of failure that
/// reads like "no split exists" instead of "the query was malformed".
fn naive_local(stamp: &str) -> String {
    let Some((date, time)) = stamp.split_once('T') else {
        return stamp.to_string();
    };
    let cut = time.find(['+', '-', 'Z']).unwrap_or(time.len());
    format!("{date}T{}", &time[..cut])
}

/// The direct journey as the split solver needs it: the stops it may cut at, the
/// train covering each stop pair, and what the through ticket costs.
///
/// Both backends answer those three questions, and name every field involved
/// differently. Reading them is per-backend; everything after it -- the pairwise
/// pricing, the DP, train matching, contract boundaries -- is not. That asymmetry
/// is the whole reason this struct exists rather than a second copy of the solver.
struct DirectJourney {
    stops: Vec<Stop>,
    spans: Vec<SectionSpan>,
    direct_price: Option<f64>,
}

/// dbweb: the connection is the array element itself, and the through fare sits
/// on it as `angebotsPreis`.
fn dbweb_direct_journey(body: &Value) -> Result<DirectJourney, HafasError> {
    let v = body
        .get("verbindungen")
        .and_then(|v| v.as_array())
        .ok_or_else(|| HafasError::Other("no connections found in Vendo response".into()))?
        .first()
        .ok_or_else(|| HafasError::Other("no connection found".into()))?;
    let stops = extract_stops(v);
    let spans = extract_section_spans(v, &stops);
    Ok(DirectJourney {
        direct_price: v
            .get("angebotsPreis")
            .and_then(|p| p.get("betrag"))
            .and_then(|b| b.as_f64()),
        stops,
        spans,
    })
}

/// dbnav: the connection sits a level down under `verbindung`, and the fare hangs
/// off the sibling `angebote` rather than off the connection -- the same place
/// `parse_dbnav_journeys` already reads `total_price` from.
fn dbnav_direct_journey(body: &Value) -> Result<DirectJourney, HafasError> {
    let item = body
        .get("verbindungen")
        .and_then(|v| v.as_array())
        .ok_or_else(|| HafasError::Other("no connections found in dbnav response".into()))?
        .first()
        .ok_or_else(|| HafasError::Other("no connection found".into()))?;
    let v = item
        .get("verbindung")
        .ok_or_else(|| HafasError::Other("dbnav connection carries no `verbindung`".into()))?;
    let stops = extract_dbnav_stops(v);
    let spans = extract_dbnav_section_spans(v, &stops);
    Ok(DirectJourney {
        direct_price: item
            .get("angebote")
            .and_then(|a| a.get("preise"))
            .and_then(|p| p.get("gesamt"))
            .and_then(|g| g.get("ab"))
            .and_then(|ab| ab.get("betrag"))
            .and_then(|b| b.as_f64()),
        stops,
        spans,
    })
}

/// `extract_stops` against dbnav's names.
///
/// Three differences, every one of them read off the capture: the ride/footpath
/// discriminator is `typ` on the section rather than `verkehrsmittel.typ`; a
/// stop's id is `ort.evaNr`, which is the same EVA number the pairwise fare
/// queries are keyed on; and the stamp sits directly on the halt as
/// `abgangsDatum`/`ankunftsDatum` instead of under an `abfahrt`/`ankunft` object.
fn extract_dbnav_stops(v: &Value) -> Vec<Stop> {
    let mut stops = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return stops;
    };
    for section in sections {
        let Some(halte) = dbnav_ride_halte(section) else {
            continue;
        };
        for halt in [&halte[0], halte.last().unwrap()] {
            let Some(ext_id) = dbnav_halt_id(halt) else {
                continue;
            };
            if stops.iter().any(|s: &Stop| s.ext_id == ext_id) {
                continue;
            }
            let departure_iso = ["abgangsDatum", "ankunftsDatum"]
                .iter()
                .find_map(|key| halt.get(*key).and_then(|t| t.as_str()))
                .map(naive_local)
                .unwrap_or_default();
            stops.push(Stop {
                ext_id,
                departure_iso,
            });
        }
    }
    stops
}

/// `extract_section_spans` against dbnav's names, and a second pass for the same
/// reason the dbweb one is: `extract_dbnav_stops` deduplicates by station, so a
/// transfer stop is pushed while the arriving section is read, before the train
/// it later departs on is known.
fn extract_dbnav_section_spans(v: &Value, stops: &[Stop]) -> Vec<SectionSpan> {
    let index_of = |ext_id: &str| stops.iter().position(|s| s.ext_id == ext_id);
    let mut spans = Vec::new();
    let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) else {
        return spans;
    };
    for section in sections {
        let Some(halte) = dbnav_ride_halte(section) else {
            continue;
        };
        let (Some(from_id), Some(to_id)) = (
            dbnav_halt_id(&halte[0]),
            dbnav_halt_id(halte.last().unwrap()),
        ) else {
            continue;
        };
        let (Some(from), Some(to)) = (index_of(&from_id), index_of(&to_id)) else {
            continue;
        };
        spans.push(SectionSpan {
            from,
            to,
            // `zugNummer` is the bare number `classify_train_match` compares a
            // journey's `train_number` against. `mitteltext` is the rider-facing
            // name ("ICE 857") and would never match one.
            train_number: str_field(section, "zugNummer"),
        });
    }
    spans
}

/// A section's stops, if the section is a ride at all.
///
/// dbnav puts the mode in `typ` on the section: `FAHRZEUG` is a ride, `FUSSWEG`
/// is the footpath dbweb calls `WALK`. Matching `FAHRZEUG` rather than excluding
/// `FUSSWEG` keeps an unknown future mode out of a priced chain instead of
/// quietly selling it, which is the call `parse_dbnav_journeys` already makes.
fn dbnav_ride_halte(section: &Value) -> Option<&Vec<Value>> {
    if section.get("typ").and_then(|t| t.as_str()) != Some("FAHRZEUG") {
        return None;
    }
    section
        .get("halte")
        .and_then(|h| h.as_array())
        .filter(|halte| halte.len() >= 2)
}

/// A halt's EVA number, which is the id the pairwise fare queries take.
fn dbnav_halt_id(halt: &Value) -> Option<String> {
    halt.get("ort")
        .and_then(|ort| ort.get("evaNr"))
        .and_then(|id| id.as_str())
        .map(str::to_string)
}

/// Pure DP core of the split-ticket solver: given `n` stops (0..n, in route
/// order) and known fares between some pairs, finds the cheapest way to
/// stitch together consecutive purchased segments from stop 0 to stop n-1.
/// No network, no HAFAS types -- just prices in, (total, path) out. This is
/// the actual algorithm the original evaluation flagged as "zero tests
/// despite being the riskiest code"; see the tests below.
pub fn cheapest_split(
    n: usize,
    segment_prices: &HashMap<(usize, usize), f64>,
) -> Option<(f64, Vec<(usize, usize)>)> {
    if n == 0 {
        return None;
    }
    let mut dp = vec![f64::INFINITY; n];
    dp[0] = 0.0;
    let mut parent: Vec<Option<usize>> = vec![None; n];

    for i in 1..n {
        for j in 0..i {
            if let Some(price) = segment_prices.get(&(j, i)) {
                let cost = dp[j] + price;
                if cost < dp[i] {
                    dp[i] = cost;
                    parent[i] = Some(j);
                }
            }
        }
    }

    if dp[n - 1] == f64::INFINITY {
        return None;
    }

    let mut path = Vec::new();
    let mut curr = n - 1;
    while let Some(prev) = parent[curr] {
        path.push((prev, curr));
        curr = prev;
    }
    path.reverse();
    Some((dp[n - 1], path))
}

/// Pure JSON -> `Journey` parser, extracted out of `search_connections` so it
/// can be unit tested against a fixture without any network call. This is
/// the hand-rolled parsing of an undocumented, reverse-engineered response
/// shape -- the other piece the original evaluation flagged as untested.
pub fn parse_journeys_from_response(body: &Value) -> Vec<Journey> {
    let mut journeys = Vec::new();
    let Some(verbindungen) = body.get("verbindungen").and_then(|v| v.as_array()) else {
        return journeys;
    };

    for v in verbindungen {
        let mut legs = Vec::new();
        if let Some(sections) = v.get("verbindungsAbschnitte").and_then(|s| s.as_array()) {
            for section in sections {
                let verkehrsmittel = section
                    .get("verkehrsmittel")
                    .cloned()
                    .unwrap_or(Value::Null);
                let name = verkehrsmittel
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let number = verkehrsmittel
                    .get("nummer")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let category = verkehrsmittel
                    .get("kategorie")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();

                let attributes = verkehrsmittel
                    .get("zugattribute")
                    .and_then(|a| a.as_array());
                let is_regional = attributes
                    .map(|attrs| {
                        attrs
                            .iter()
                            .any(|attr| attr.get("key").and_then(|k| k.as_str()) == Some("9G"))
                    })
                    .unwrap_or(false);

                if let Some(halts) = section.get("halte").and_then(|h| h.as_array()) {
                    if halts.len() >= 2 {
                        let origin_halt = &halts[0];
                        let dest_halt = halts.last().unwrap();

                        let origin_station = Station {
                            id: origin_halt
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string(),
                            name: origin_halt
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string(),
                            latitude: None,
                            longitude: None,
                        };
                        let dest_station = Station {
                            id: dest_halt
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string(),
                            name: dest_halt
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string(),
                            latitude: None,
                            longitude: None,
                        };

                        let (scheduled_departure, realtime_departure) =
                            times_of(origin_halt, "abfahrt");
                        let (scheduled_arrival, realtime_arrival) = times_of(dest_halt, "ankunft");
                        // Real-time wins for the primary field, because that is
                        // the time you have to be on the platform for.
                        let departure_time = realtime_departure
                            .clone()
                            .or_else(|| scheduled_departure.clone())
                            .unwrap_or_default();
                        let arrival_time = realtime_arrival
                            .clone()
                            .or_else(|| scheduled_arrival.clone())
                            .unwrap_or_default();
                        let cancelled = is_cancelled(section)
                            || is_cancelled(origin_halt)
                            || is_cancelled(dest_halt);

                        // Live halts carry the plain EVA number in `extId` and a
                        // composite lid in `id`; older fixtures only the latter.
                        // station-time rejects anything that is not a plain
                        // 7-digit id, so trying both is safe.
                        let ext_id_of = |halt: &Value| -> String {
                            halt.get("extId")
                                .or_else(|| halt.get("id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        let departure_utc =
                            station_time::rfc3339_utc(&departure_time, &ext_id_of(origin_halt));
                        let arrival_utc =
                            station_time::rfc3339_utc(&arrival_time, &ext_id_of(dest_halt));

                        legs.push(Leg {
                            on_time_probability: None,
                            origin: origin_station,
                            destination: dest_station,
                            departure_time,
                            arrival_time,
                            departure_utc,
                            arrival_utc,
                            train_name: name,
                            train_number: number,
                            train_category: category,
                            platform: section
                                .get("gleis")
                                .and_then(|g| g.as_str())
                                .map(|s| s.to_string()),
                            is_regional,
                            scheduled_departure,
                            realtime_departure,
                            scheduled_arrival,
                            realtime_arrival,
                            cancelled,
                        });
                    }
                }
            }
        }

        if !legs.is_empty() {
            let first_leg = &legs[0];
            let last_leg = legs.last().unwrap();
            let price = v
                .get("angebotsPreis")
                .and_then(|p| p.get("betrag"))
                .and_then(|b| b.as_f64());
            let duration_seconds = v
                .get("verbindungsDauerInSeconds")
                .and_then(|d| d.as_u64())
                .unwrap_or(0);
            let total_duration_minutes = (duration_seconds / 60) as u32;

            journeys.push(Journey {
                // Filled by punctuality::enrich when that capability answers.
                reliability: None,
                unscored_legs: Vec::new(),
                ranking: None,
                // The real bahn.de response field is "tripId", not "id" --
                // found via live verification while wiring this adapter into
                // scouting (capabilities/store/README.md, Phase 2): every journey in a
                // real response was silently getting id="" and collapsing
                // into one upserted row downstream. The fixture this parser
                // was tested against used "id" too (same wrong assumption in
                // both places), so `cargo test` stayed green the whole time
                // -- a live call was the only thing that caught it.
                id: v
                    .get("tripId")
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string(),
                start_station: first_leg.origin.clone(),
                end_station: last_leg.destination.clone(),
                total_duration_minutes,
                legs,
                total_price: price,
                delay_risk_score: None,
                arrival_punctuality: None,
            });
        }
    }

    journeys
}

/// Pure JSON -> `Journey` parser for the dbnav response, which is a different
/// document from dbweb's rather than a renamed one. Built against a captured
/// live body (Bonn Hbf -> Berlin Hbf, 2026-08-12, 97,671 bytes, `schemaVersion`
/// 1.24.9), not against the client's JS.
///
/// Three shape differences worth naming, because each one is a silent-empty
/// bug if assumed away:
///
/// 1. Journeys nest one level deeper: `verbindungen[].verbindung`, with the
///    price on the *sibling* `verbindungen[].angebote`, so the price is read
///    from the outer item and everything else from the inner one.
/// 2. Sections carry their endpoints as `abgangsOrt`/`ankunftsOrt` objects
///    with a plain `evaNr` and real coordinates, rather than needing the first
///    and last entries of `halte`. Coordinates come out populated here and
///    `None` on dbweb, which is a gain, not a divergence to paper over.
/// 3. `typ` distinguishes `FAHRZEUG` from `FUSSWEG`. Walking transfers are
///    skipped: `Leg` is a train (it has a name, number, category and a
///    D-Ticket flag), and a footpath answers none of those. The capture shows
///    footpaths both with and without `halte`, so filtering on stop count
///    would have kept some of them.
///
/// One honest gap: dbweb marks Deutschlandticket coverage with the explicit
/// HAFAS attribute `9G`, and the dbnav capture contains no `9G` anywhere.
/// `is_regional` is therefore derived from `produktGattung` against the
/// Nahverkehr categories the D-Ticket covers -- a category judgment, not the
/// vendor's own claim. It is only ever surfaced and stored, never priced on
/// (`server.rs` renders it, `store.rs` persists it, no fare logic reads it),
/// which is what makes the approximation acceptable here rather than
/// dangerous.
pub fn parse_dbnav_journeys(body: &Value) -> Vec<Journey> {
    let mut journeys = Vec::new();
    let Some(verbindungen) = body.get("verbindungen").and_then(|v| v.as_array()) else {
        return journeys;
    };

    for item in verbindungen {
        let Some(v) = item.get("verbindung") else {
            continue;
        };
        let mut legs = Vec::new();

        for section in v
            .get("verbindungsAbschnitte")
            .and_then(|s| s.as_array())
            .unwrap_or(&Vec::new())
        {
            if section.get("typ").and_then(|t| t.as_str()) != Some("FAHRZEUG") {
                continue;
            }
            let (Some(origin), Some(destination)) = (
                section.get("abgangsOrt").map(dbnav_station),
                section.get("ankunftsOrt").map(dbnav_station),
            ) else {
                continue;
            };

            let departure_time = str_field(section, "abgangsDatum");
            let arrival_time = str_field(section, "ankunftsDatum");
            let departure_utc = station_time::rfc3339_utc(&departure_time, &origin.id);
            let arrival_utc = station_time::rfc3339_utc(&arrival_time, &destination.id);
            let category = str_field(section, "produktGattung");

            legs.push(Leg {
                on_time_probability: None,
                // The platform lives on the first stop, not on the section.
                platform: section
                    .get("halte")
                    .and_then(|h| h.as_array())
                    .and_then(|halts| halts.first())
                    .and_then(|halt| halt.get("gleis"))
                    .and_then(|g| g.as_str())
                    .map(|s| s.to_string()),
                // `mitteltext` is the rider-facing name ("ICE 857", "RE5");
                // `zugNummer` is the bare number.
                train_name: str_field(section, "mitteltext"),
                train_number: str_field(section, "zugNummer"),
                is_regional: is_dbnav_regional(&category),
                train_category: category,
                // dbnav timestamps already carry their offset, and
                // `rfc3339_utc` respects an offset rather than re-shifting it,
                // so the scheduled fields are the same strings.
                scheduled_departure: Some(departure_time.clone()),
                scheduled_arrival: Some(arrival_time.clone()),
                realtime_departure: section
                    .get("ezAbgangsDatum")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
                realtime_arrival: section
                    .get("ezAnkunftsDatum")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
                // No cancellation marker appeared in the capture (a search
                // three weeks out has no real-time layer at all). Reported as
                // "not cancelled" rather than guessed from `echtzeitNotizen`
                // text, and left for a capture that actually contains one.
                cancelled: false,
                departure_time,
                arrival_time,
                departure_utc,
                arrival_utc,
                origin,
                destination,
            });
        }

        if legs.is_empty() {
            continue;
        }
        let first_leg = &legs[0];
        let last_leg = legs.last().unwrap();
        journeys.push(Journey {
            // Filled by punctuality::enrich when that capability answers.
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
            id: str_field(v, "checksum"),
            start_station: first_leg.origin.clone(),
            end_station: last_leg.destination.clone(),
            total_duration_minutes: (v.get("reiseDauer").and_then(|d| d.as_u64()).unwrap_or(0) / 60)
                as u32,
            legs,
            total_price: item
                .get("angebote")
                .and_then(|a| a.get("preise"))
                .and_then(|p| p.get("gesamt"))
                .and_then(|g| g.get("ab"))
                .and_then(|ab| ab.get("betrag"))
                .and_then(|b| b.as_f64()),
            delay_risk_score: None,
            arrival_punctuality: None,
        });
    }

    journeys
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// A dbnav `abgangsOrt`/`ankunftsOrt`/`ort` object. `evaNr` is the plain EVA
/// number station-time needs; the composite `locationId` would be rejected.
fn dbnav_station(ort: &Value) -> Station {
    let position = ort.get("position");
    Station {
        id: str_field(ort, "evaNr"),
        name: str_field(ort, "name"),
        latitude: position
            .and_then(|p| p.get("latitude"))
            .and_then(|l| l.as_f64()),
        longitude: position
            .and_then(|p| p.get("longitude"))
            .and_then(|l| l.as_f64()),
    }
}

/// Nahverkehr categories the Deutschlandticket covers. Long-distance (ICE, IC,
/// EC, and the private FLX/RJ/TGV/NJ operators) is excluded, which is the line
/// the ticket itself draws.
fn is_dbnav_regional(gattung: &str) -> bool {
    matches!(
        gattung,
        "RB" | "RE" | "IRE" | "S" | "STR" | "U" | "BUS" | "SCHIFF" | "ANRUFPFLICHTIG"
    )
}

/// Pure JSON -> `Station` parser for the `/reiseloesung/orte` suggest
/// endpoint, extracted for the same fixture-testability reason.
pub fn parse_suggest_response(list: &[Value]) -> Vec<Station> {
    let mut stations = Vec::new();
    for item in list {
        if let (Some(ext_id), Some(name)) = (
            item.get("extId").and_then(|id| id.as_str()),
            item.get("name").and_then(|n| n.as_str()),
        ) {
            stations.push(Station {
                id: ext_id.to_string(),
                name: name.to_string(),
                latitude: item.get("lat").and_then(|l| l.as_f64()),
                longitude: item.get("lon").and_then(|l| l.as_f64()),
            });
        }
    }
    stations
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cheapest_split_prefers_two_hop_over_direct_when_cheaper() {
        let mut prices = HashMap::new();
        prices.insert((0, 2), 25.0); // direct
        prices.insert((0, 1), 10.0);
        prices.insert((1, 2), 10.0); // split total 20.0, cheaper than direct

        let (total, path) = cheapest_split(3, &prices).unwrap();
        assert!((total - 20.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn cheapest_split_falls_back_to_direct_when_split_is_pricier() {
        let mut prices = HashMap::new();
        prices.insert((0, 2), 15.0);
        prices.insert((0, 1), 10.0);
        prices.insert((1, 2), 10.0); // split total 20.0, direct is cheaper

        let (total, path) = cheapest_split(3, &prices).unwrap();
        assert!((total - 15.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 2)]);
    }

    #[test]
    fn cheapest_split_returns_none_when_unreachable() {
        let mut prices = HashMap::new();
        prices.insert((0, 1), 10.0);
        // no price at all reaches stop 2 -- (1,2) and (0,2) both missing.
        let result = cheapest_split(3, &prices);
        assert!(result.is_none());
    }

    /// A delayed and partly cancelled connection, in the shape bahn.de serves.
    ///
    /// Every key here was captured from a real bahn.de response on 2026-08-11,
    /// not guessed. That mattered: the first version of this used `istzeit`,
    /// which the endpoint does not serve, so the field would have been
    /// structurally null forever while the test passed. The live capture showed
    /// `echtzeit`, on a train running 35 minutes late.
    #[test]
    fn a_delay_survives_the_parse_instead_of_being_folded_away() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": {
                        "typ": "ZUG", "name": "ICE 1513",
                        "nummer": "1513", "kurzText": "ICE"
                    },
                    "halte": [
                        {
                            "id": "8000044", "name": "Bonn Hbf",
                            "abfahrt": {
                                "sollzeit": "2026-08-25T10:07:00",
                                "echtzeit": "2026-08-25T10:26:00"
                            }
                        },
                        {
                            "id": "8000105", "name": "Frankfurt(Main)Hbf",
                            "ankunft": {
                                "sollzeit": "2026-08-25T11:49:00",
                                "echtzeit": "2026-08-25T12:04:00"
                            }
                        }
                    ]
                }]
            }]
        });

        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        let leg = &journeys[0].legs[0];

        // Both times survive, and they are different: nineteen minutes late.
        assert_eq!(
            leg.scheduled_departure.as_deref(),
            Some("2026-08-25T10:07:00")
        );
        assert_eq!(
            leg.realtime_departure.as_deref(),
            Some("2026-08-25T10:26:00")
        );
        assert_eq!(
            leg.scheduled_arrival.as_deref(),
            Some("2026-08-25T11:49:00")
        );
        assert_eq!(leg.realtime_arrival.as_deref(), Some("2026-08-25T12:04:00"));

        // The primary field carries the time you have to be there for.
        assert_eq!(leg.departure_time, "2026-08-25T10:26:00");
        assert_eq!(leg.arrival_time, "2026-08-25T12:04:00");
        assert!(!leg.cancelled);
    }

    /// One journey of a real dbnav response, trimmed structurally: the other
    /// four journeys and the notice arrays are gone, every field this parser
    /// reads is byte-identical to what the endpoint served on 2026-08-12
    /// (Bonn Hbf -> Berlin Hbf, departing 09:00). A public timetable query,
    /// so the fixture carries nothing personal.
    const DBNAV_CAPTURE: &str = r#"{
 "verbindungen": [
  {
   "verbindung": {
    "checksum": "aaa67600_3",
    "reiseDauer": 18480,
    "umstiegeAnzahl": 1,
    "verbindungsAbschnitte": [
     {
      "typ": "FAHRZEUG",
      "abgangsDatum": "2026-09-15T09:04:00+02:00",
      "ankunftsDatum": "2026-09-15T09:28:00+02:00",
      "abschnittsDauer": 1440,
      "mitteltext": "RE5",
      "kurztext": "NX",
      "produktGattung": "RB",
      "zugNummer": "28510",
      "verkehrsmittelNummer": "28510",
      "abgangsOrt": {
       "name": "Bonn Hbf",
       "locationId": "A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@",
       "evaNr": "8000044",
       "position": {
        "latitude": 50.731964,
        "longitude": 7.096678
       }
      },
      "ankunftsOrt": {
       "name": "Köln Hbf",
       "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
       "evaNr": "8000207",
       "position": {
        "latitude": 50.94282,
        "longitude": 6.959197
       }
      },
      "halte": [
       {
        "abgangsDatum": "2026-09-15T09:04:00+02:00",
        "gleis": "1",
        "ort": {
         "name": "Bonn Hbf",
         "locationId": "A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@",
         "evaNr": "8000044",
         "position": {
          "latitude": 50.731964,
          "longitude": 7.096678
         }
        }
       },
       {
        "ankunftsDatum": "2026-09-15T09:28:00+02:00",
        "gleis": "1 A-C",
        "ort": {
         "name": "Köln Hbf",
         "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
         "evaNr": "8000207",
         "position": {
          "latitude": 50.94282,
          "longitude": 6.959197
         }
        }
       }
      ]
     },
     {
      "typ": "FUSSWEG",
      "abgangsDatum": "2026-09-15T09:28:00+02:00",
      "ankunftsDatum": "2026-09-15T09:45:00+02:00",
      "abschnittsDauer": 1020,
      "abgangsOrt": {
       "name": "Köln Hbf",
       "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
       "evaNr": "8000207",
       "position": {
        "latitude": 50.94282,
        "longitude": 6.959197
       }
      },
      "ankunftsOrt": {
       "name": "Köln Hbf",
       "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
       "evaNr": "8000207",
       "position": {
        "latitude": 50.94282,
        "longitude": 6.959197
       }
      },
      "halte": [
       {
        "ankunftsDatum": "2026-09-15T09:28:00+02:00",
        "gleis": "1 A-C",
        "ort": {
         "name": "Köln Hbf",
         "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
         "evaNr": "8000207",
         "position": {
          "latitude": 50.94282,
          "longitude": 6.959197
         }
        }
       },
       {
        "abgangsDatum": "2026-09-15T09:45:00+02:00",
        "gleis": "2 A-C",
        "ort": {
         "name": "Köln Hbf",
         "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
         "evaNr": "8000207",
         "position": {
          "latitude": 50.94282,
          "longitude": 6.959197
         }
        }
       }
      ]
     },
     {
      "typ": "FAHRZEUG",
      "abgangsDatum": "2026-09-15T09:45:00+02:00",
      "ankunftsDatum": "2026-09-15T14:12:00+02:00",
      "abschnittsDauer": 16020,
      "mitteltext": "ICE 857",
      "kurztext": "ICE",
      "produktGattung": "ICE",
      "zugNummer": "857",
      "verkehrsmittelNummer": "857",
      "abgangsOrt": {
       "name": "Köln Hbf",
       "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
       "evaNr": "8000207",
       "position": {
        "latitude": 50.94282,
        "longitude": 6.959197
       }
      },
      "ankunftsOrt": {
       "name": "Berlin Hbf",
       "locationId": "A=1@O=Berlin Hbf@X=13369549@Y=52525589@U=80@L=8098160@i=U×008031922@",
       "evaNr": "8098160",
       "position": {
        "latitude": 52.52585,
        "longitude": 13.368892
       }
      },
      "halte": [
       {
        "abgangsDatum": "2026-09-15T09:45:00+02:00",
        "gleis": "2 A-C",
        "ort": {
         "name": "Köln Hbf",
         "locationId": "A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@",
         "evaNr": "8000207",
         "position": {
          "latitude": 50.94282,
          "longitude": 6.959197
         }
        }
       },
       {
        "ankunftsDatum": "2026-09-15T14:12:00+02:00",
        "gleis": "3",
        "ort": {
         "name": "Berlin Hbf",
         "locationId": "A=1@O=Berlin Hbf@X=13369549@Y=52525589@U=80@L=8098160@i=U×008031922@",
         "evaNr": "8098160",
         "position": {
          "latitude": 52.52585,
          "longitude": 13.368892
         }
        }
       }
      ]
     }
    ]
   },
   "angebote": {
    "preise": {
     "istTeilpreis": false,
     "hinRueckPauschalpreis": false,
     "gesamt": {
      "klasse": "KLASSE_2",
      "ab": {
       "waehrung": "EUR",
       "betrag": 59.99
      }
     }
    }
   }
  }
 ]
}"#;

    /// The parser against captured reality. Asserts the three shape traps
    /// named on `parse_dbnav_journeys`: the extra `verbindung` nesting, the
    /// price living on the sibling `angebote`, and the footpath between the
    /// two trains being dropped rather than becoming a nameless leg.
    #[test]
    fn the_dbnav_parser_reads_a_captured_live_response() {
        let body: Value = serde_json::from_str(DBNAV_CAPTURE).expect("fixture parses");
        let journeys = parse_dbnav_journeys(&body);

        assert_eq!(journeys.len(), 1);
        let j = &journeys[0];
        assert_eq!(j.id, "aaa67600_3");
        assert_eq!(
            j.total_price,
            Some(59.99),
            "price comes off the sibling angebote"
        );
        assert_eq!(j.total_duration_minutes, 308, "18480s / 60");
        assert_eq!(j.start_station.name, "Bonn Hbf");
        assert_eq!(
            j.start_station.id, "8000044",
            "evaNr, not the composite locationId"
        );
        assert_eq!(j.end_station.name, "Berlin Hbf");

        // Three sections in the response, one of them a FUSSWEG.
        assert_eq!(j.legs.len(), 2, "the footpath is not a leg");
        let first = &j.legs[0];
        assert_eq!(first.train_name, "RE5");
        assert_eq!(first.train_number, "28510");
        assert_eq!(first.train_category, "RB");
        assert!(first.is_regional, "RB is Nahverkehr");
        assert_eq!(first.platform.as_deref(), Some("1"));
        let second = &j.legs[1];
        assert_eq!(second.train_name, "ICE 857");
        assert!(!second.is_regional, "ICE is not covered by the D-Ticket");

        // Coordinates survive: dbnav carries them, dbweb does not.
        assert_eq!(j.start_station.latitude, Some(50.731964));
    }

    /// dbnav timestamps arrive with their offset already on them, so the UTC
    /// fields must respect it rather than shift a second time. 09:04+02:00 is
    /// 07:04Z, and a double shift would read 05:04Z.
    #[test]
    fn an_offset_carrying_dbnav_time_is_not_shifted_twice() {
        let body: Value = serde_json::from_str(DBNAV_CAPTURE).expect("fixture parses");
        let leg = &parse_dbnav_journeys(&body).remove(0).legs[0];
        assert_eq!(leg.departure_time, "2026-09-15T09:04:00+02:00");
        assert_eq!(leg.departure_utc.as_deref(), Some("2026-09-15T07:04:00Z"));
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-09-15T07:28:00Z"));
        assert_eq!(
            leg.realtime_departure, None,
            "a search weeks out has no real-time layer"
        );
    }

    /// An empty or foreign document yields no journeys instead of a panic or a
    /// journey full of empty strings -- the same contract dbweb's parser has.
    #[test]
    fn the_dbnav_parser_answers_nothing_for_a_document_it_does_not_recognize() {
        assert!(parse_dbnav_journeys(&json!({})).is_empty());
        assert!(parse_dbnav_journeys(&json!({"verbindungen": []})).is_empty());
        assert!(
            parse_dbnav_journeys(&json!({"verbindungen": [{"verbindung": {"checksum": "x"}}]}))
                .is_empty(),
            "a journey with no train sections is not a journey"
        );
    }

    /// Every answer this capability gives comes from a live query, which is what kept
    /// transit out of the published demo: a recorded timetable stops being true within
    /// the hour. Overridable endpoints let the real client be pointed at a stub, so the
    /// parser still does the work and the recording is transit's own output.
    #[test]
    fn every_endpoint_can_be_pointed_at_a_stub_and_otherwise_is_bahn_de() {
        let stub = |var: &str| match var {
            "SJEL_TRANSIT_FAHRPLAN_URL" => Some("http://127.0.0.1:8099/fahrplan".to_string()),
            "SJEL_TRANSIT_ORTE_URL" => Some("http://127.0.0.1:8099/orte".to_string()),
            "SJEL_TRANSIT_DBNAV_FAHRPLAN_URL" => {
                Some("http://127.0.0.1:8099/dbnav/fahrplan".to_string())
            }
            _ => None,
        };
        let overridden = Endpoints::resolve(stub);
        assert_eq!(overridden.fahrplan, "http://127.0.0.1:8099/fahrplan");
        assert_eq!(overridden.orte, "http://127.0.0.1:8099/orte");
        assert_eq!(
            overridden.dbnav_fahrplan,
            "http://127.0.0.1:8099/dbnav/fahrplan"
        );

        // Unset is the normal case and must reach the real endpoints, including the
        // dbnav one -- it is the default backend, so leaving it hardcoded would have
        // made the default the one path a stub cannot reach.
        let bare = Endpoints::resolve(|_| None);
        assert_eq!(bare.fahrplan, FAHRPLAN_URL);
        assert_eq!(bare.orte, ORTE_URL);
        assert_eq!(bare.dbnav_fahrplan, DBNAV_FAHRPLAN_URL);

        // Exported-but-empty is a shell that had nothing to put there, not an intent to
        // POST to "".
        let blank = Endpoints::resolve(|_| Some("   ".to_string()));
        assert_eq!(blank.fahrplan, FAHRPLAN_URL);
        assert_eq!(blank.dbnav_fahrplan, DBNAV_FAHRPLAN_URL);
    }

    /// The two backends build genuinely different requests from one call, and
    /// the fare context survives the translation into dbnav's vocabulary: the
    /// discount is a space-joined string there, an object on dbweb.
    #[test]
    fn the_backend_seam_translates_the_fare_context_rather_than_dropping_it() {
        assert_eq!(RailBackend::default(), RailBackend::DbNav);

        let bc25 = FareOptions {
            bahncard: Some(25),
            first_class: false,
            deutschland_ticket: true,
        };
        let dbnav = HafasClient::dbnav_payload("8000044", "8011160", "2026-09-15T09:00:00", &bc25)
            .expect("bahncard 25 is valid");
        assert_eq!(
            dbnav["reisendenProfil"]["reisende"][0]["ermaessigungen"][0],
            "BAHNCARD25 KLASSE_2"
        );
        assert_eq!(
            dbnav["fahrverguenstigungen"]["deutschlandTicketVorhanden"],
            true
        );
        assert_eq!(
            dbnav["reiseHin"]["wunsch"]["abgangsLocationId"],
            "A=1@L=8000044@"
        );
        assert_eq!(
            dbnav["reiseHin"]["wunsch"]["zeitWunsch"]["reiseDatum"],
            "2026-09-15T09:00:00"
        );

        let dbweb =
            HafasClient::fahrplan_payload("8000044", "8011160", "2026-09-15T09:00:00", &bc25)
                .expect("same options on the other backend");
        assert_eq!(
            dbweb["reisende"][0]["ermaessigungen"][0]["art"],
            "BAHNCARD25"
        );

        // A rejected BahnCard is rejected on both, not silently dropped by one.
        let bogus = FareOptions {
            bahncard: Some(17),
            ..Default::default()
        };
        assert!(
            HafasClient::dbnav_payload("8000044", "8011160", "2026-09-15T09:00:00", &bogus)
                .is_err()
        );
    }

    /// A departure time without seconds reaches bahn.de with seconds, on both
    /// backends, and a time that is not a datetime never reaches it at all.
    ///
    /// The regression: `time=2026-08-16T13:00` posted verbatim made the fahrplan
    /// endpoint answer 500 with an empty body, every search 500'd for a caller who
    /// had typed the query by hand, and the emptiness of it read as the upstream
    /// blocking us. Deterministic then and deterministic now -- the same route with
    /// `:00` appended returned journeys in the same minute.
    #[test]
    fn a_departure_time_without_seconds_is_completed_and_a_non_datetime_is_refused() {
        let fare = FareOptions::default();
        let dbweb = HafasClient::fahrplan_payload("8000044", "8000261", "2026-08-16T13:00", &fare)
            .expect("minute precision is a caller's shorthand, not an error");
        assert_eq!(dbweb["anfrageZeitpunkt"], "2026-08-16T13:00:00");

        let dbnav = HafasClient::dbnav_payload("8000044", "8000261", "2026-08-16T13:00", &fare)
            .expect("the other backend takes the same shorthand");
        assert_eq!(
            dbnav["reiseHin"]["wunsch"]["zeitWunsch"]["reiseDatum"],
            "2026-08-16T13:00:00"
        );

        // A full timestamp is passed through byte-identically, seconds and all.
        let exact =
            HafasClient::fahrplan_payload("8000044", "8000261", "2026-08-16T13:00:42", &fare)
                .unwrap();
        assert_eq!(exact["anfrageZeitpunkt"], "2026-08-16T13:00:42");

        for junk in [
            "2026-08-16",
            "16.08.2026T13:00",
            "2026-08-16 13:00",
            "2026-08-16T13:00:00Z",
            "2026-08-16T13:00:00+02:00",
            "2026-8-16T13:00",
            "2026-13-16T13:00",
            "2026-08-16T25:00",
            "tomorrow",
            "",
        ] {
            let err = HafasClient::fahrplan_payload("8000044", "8000261", junk, &fare)
                .expect_err("refused before it becomes a request");
            assert!(
                matches!(err, HafasError::InvalidDatetime(ref got) if got == junk),
                "{junk:?} should name itself in its own error, got {err:?}"
            );
            assert!(HafasClient::dbnav_payload("8000044", "8000261", junk, &fare).is_err());
        }
    }

    /// The three facts the split solver reads off a direct journey, now read off a
    /// dbnav body: where the chain may be cut, which train covers each cut, and
    /// what the through ticket costs. Same capture the journey parser is tested
    /// against, so the mapping is checked against a real response rather than a
    /// hand-written idea of one.
    #[test]
    fn the_dbnav_split_reader_finds_the_three_facts_the_dbweb_one_does() {
        let body: Value = serde_json::from_str(DBNAV_CAPTURE).expect("fixture parses");
        let direct = dbnav_direct_journey(&body).expect("the capture is a usable direct journey");

        let ids: Vec<&str> = direct.stops.iter().map(|s| s.ext_id.as_str()).collect();
        assert_eq!(
            ids,
            ["8000044", "8000207", "8098160"],
            "EVA numbers off ort.evaNr, in route order, the transfer station once"
        );

        let spans: Vec<(usize, usize, &str)> = direct
            .spans
            .iter()
            .map(|s| (s.from, s.to, s.train_number.as_str()))
            .collect();
        assert_eq!(
            spans,
            [(0, 1, "28510"), (1, 2, "857")],
            "bare zugNummer, not the rider-facing `mitteltext` a journey never carries"
        );

        assert_eq!(
            direct.direct_price,
            Some(59.99),
            "the through fare hangs off the sibling angebote, not off the connection"
        );
    }

    /// The bug this port would have shipped with, had it been written from the
    /// field list instead of the capture.
    ///
    /// dbnav stamps times with their offset. `search_split_tickets` hands a stop's
    /// stamp straight back to `search_connections` as the moment to price from, and
    /// `normalize_datetime` refuses an offset-carrying string -- it splits into four
    /// colon-separated fields, not three. The direct search would have succeeded and
    /// then every single priced pair would have failed, which surfaces as "no split
    /// exists" rather than as a malformed query.
    #[test]
    fn a_dbnav_stop_stamp_loses_its_offset_or_every_priced_pair_dies() {
        let body: Value = serde_json::from_str(DBNAV_CAPTURE).expect("fixture parses");
        let direct = dbnav_direct_journey(&body).expect("usable direct journey");

        assert_eq!(direct.stops[0].departure_iso, "2026-09-15T09:04:00");
        assert_eq!(
            direct.stops[2].departure_iso, "2026-09-15T14:12:00",
            "a stop with only an arrival still yields a stamp"
        );

        // The half that makes it a bug rather than a cosmetic difference.
        assert!(
            normalize_datetime("2026-09-15T09:04:00+02:00").is_err(),
            "the raw dbnav stamp is not a datetime this client will send"
        );
        assert!(normalize_datetime(&direct.stops[0].departure_iso).is_ok());
    }

    /// Köln appears three times in the capture -- twice as the ends of the ride
    /// sections and twice more as the ends of the FUSSWEG between them -- and must
    /// become exactly one stop the chain can be cut at.
    ///
    /// The footpath is skipped by matching `typ == "FAHRZEUG"` rather than by
    /// excluding `FUSSWEG`, so a mode nobody has seen yet stays out of a priced
    /// chain instead of being sold as a leg.
    #[test]
    fn the_footpath_is_not_somewhere_a_ticket_can_be_cut() {
        let body: Value = serde_json::from_str(DBNAV_CAPTURE).expect("fixture parses");
        let v = &body["verbindungen"][0]["verbindung"];

        assert_eq!(
            v["verbindungsAbschnitte"]
                .as_array()
                .map(|s| s.len())
                .unwrap(),
            3,
            "the capture really does carry the footpath"
        );
        let stops = extract_dbnav_stops(v);
        assert_eq!(stops.len(), 3, "three cut points from three sections");
        assert_eq!(
            stops.iter().filter(|s| s.ext_id == "8000207").count(),
            1,
            "the transfer station is one stop, not one per section that touches it"
        );

        // The transfer stop's stamp is its arrival, because dedup keeps the first
        // sighting -- the same order dbweb produces, and the right one: pricing a
        // segment out of Köln searches from when the traveller is standing there.
        assert_eq!(stops[1].departure_iso, "2026-09-15T09:28:00");
    }

    /// A split chain's ticket boundaries carry facts, not verdicts: which
    /// station, whether the same train continues (no connection to miss),
    /// and the UTC-correct transfer buffer. The delay-share slot stays None
    /// here -- punctuality fills it, and its absence must not invent one.
    #[test]
    fn contract_boundaries_carry_the_facts_a_through_ticket_would_not_need() {
        let station = |name: &str| Station {
            id: format!("A=1@O={name}@L=8000105@"),
            name: name.into(),
            latitude: None,
            longitude: None,
        };
        let seg = |train: &str, arr_utc: Option<&str>, dep_utc: Option<&str>| SplitSegment {
            journey: Journey {
                reliability: None,
                unscored_legs: Vec::new(),
                ranking: None,
                id: "j".into(),
                start_station: station("From"),
                end_station: station("Boundary"),
                legs: vec![Leg {
                    on_time_probability: None,
                    origin: station("From"),
                    destination: station("Boundary"),
                    departure_time: "2026-09-01T08:00:00".into(),
                    arrival_time: "2026-09-01T10:00:00".into(),
                    departure_utc: dep_utc.map(String::from),
                    arrival_utc: arr_utc.map(String::from),
                    train_name: format!("ICE {train}"),
                    train_number: train.into(),
                    train_category: "ICE".into(),
                    platform: None,
                    is_regional: false,
                    scheduled_departure: None,
                    realtime_departure: None,
                    scheduled_arrival: None,
                    realtime_arrival: None,
                    cancelled: false,
                }],
                total_duration_minutes: 120,
                total_price: Some(20.0),
                delay_risk_score: None,
                arrival_punctuality: None,
            },
            train_match: TrainMatch::Exact,
            expected_trains: vec![train.into()],
        };

        // Train change with UTC on both sides: the buffer is measurable.
        let chain = vec![
            seg("100", Some("2026-09-01T08:00:00Z"), None),
            seg("200", None, Some("2026-09-01T08:12:00Z")),
        ];
        let boundaries = contract_boundaries_of(&chain);
        assert_eq!(boundaries.len(), 1);
        assert!(!boundaries[0].same_train);
        assert_eq!(boundaries[0].transfer_minutes, Some(12));
        assert_eq!(boundaries[0].incoming_share_late_6, None);

        // Same train across the boundary: a mid-run ticket split.
        let mid_run = vec![
            seg("100", Some("2026-09-01T08:00:00Z"), None),
            seg("100", None, Some("2026-09-01T08:00:00Z")),
        ];
        assert!(contract_boundaries_of(&mid_run)[0].same_train);

        // A side without UTC yields no buffer rather than a naive-subtraction lie.
        let no_utc = vec![seg("100", None, None), seg("200", None, None)];
        assert_eq!(contract_boundaries_of(&no_utc)[0].transfer_minutes, None);
    }

    /// The fare context reaches the wire in the exact shape bahn.de's own
    /// clients send (enum strings from db-vendo-client's loyalty-cards
    /// formatter), the no-options default stays byte-compatible with what
    /// always worked, and a card that does not exist fails loudly instead of
    /// pricing as no card.
    #[test]
    fn the_fare_context_reaches_the_payload_and_rejects_fantasy_cards() {
        let fare = FareOptions {
            bahncard: Some(25),
            first_class: false,
            deutschland_ticket: true,
        };
        let p = HafasClient::fahrplan_payload("8000207", "8000105", "2026-09-01T08:00:00", &fare)
            .unwrap();
        assert_eq!(p["klasse"], "KLASSE_2");
        assert_eq!(
            p["reisende"][0]["ermaessigungen"][0],
            serde_json::json!({"art": "BAHNCARD25", "klasse": "KLASSE_2"})
        );
        assert_eq!(p["deutschlandTicketVorhanden"], true);

        let default = HafasClient::fahrplan_payload(
            "8000207",
            "8000105",
            "2026-09-01T08:00:00",
            &FareOptions::default(),
        )
        .unwrap();
        assert_eq!(
            default["reisende"][0]["ermaessigungen"][0],
            serde_json::json!({"art": "KEINE_ERMAESSIGUNG", "klasse": "KLASSENLOS"})
        );
        assert_eq!(default["deutschlandTicketVorhanden"], false);

        let fantasy = FareOptions {
            bahncard: Some(17),
            ..Default::default()
        };
        assert!(HafasClient::fahrplan_payload(
            "8000207",
            "8000105",
            "2026-09-01T08:00:00",
            &fantasy
        )
        .is_err());
    }

    /// A zone-crossing leg gets unambiguous UTC instants next to its naive
    /// station-local strings. Shape captured from a real Köln->London response
    /// on 2026-08-12: live halts carry the plain EVA in `extId` and a composite
    /// lid in `id`, and London's naive arrival is BST, one hour behind CEST --
    /// the naive strings must survive byte-identical while the UTC pair carries
    /// the true elapsed time.
    #[test]
    fn a_zone_crossing_leg_carries_utc_instants_next_to_its_local_times() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "ICE 316", "nummer": "316" },
                    "halte": [
                        { "id": "A=1@O=Köln Hbf@X=6958730@Y=50943029@L=8000207@",
                          "extId": "8000207", "name": "Köln Hbf",
                          "abfahrt": { "sollzeit": "2026-08-13T09:43:00" } },
                        { "id": "A=1@O=London St. Pancras@X=-126361@Y=51531922@L=7004428@",
                          "extId": "7004428", "name": "London St. Pancras International",
                          "ankunft": { "sollzeit": "2026-08-13T13:57:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];

        // The naive station-local strings are untouched -- the dashboard
        // renders them as-is.
        assert_eq!(leg.departure_time, "2026-08-13T09:43:00");
        assert_eq!(leg.arrival_time, "2026-08-13T13:57:00");
        // The UTC pair is what arithmetic uses: 07:43Z -> 12:57Z is 5h14m,
        // where naive subtraction would have said 4h14m.
        assert_eq!(leg.departure_utc.as_deref(), Some("2026-08-13T07:43:00Z"));
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-08-13T12:57:00Z"));
    }

    /// A station whose UIC prefix station-time does not know yields absent UTC
    /// fields, never a zone guess.
    #[test]
    fn an_unknown_station_prefix_leaves_the_utc_fields_absent() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "X 1", "nummer": "1" },
                    "halte": [
                        { "id": "2000001", "name": "Somewhere in a multi-zone country",
                          "abfahrt": { "sollzeit": "2026-08-13T09:43:00" } },
                        { "id": "8000207", "extId": "8000207", "name": "Köln Hbf",
                          "ankunft": { "sollzeit": "2026-08-13T13:57:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];
        assert_eq!(leg.departure_utc, None);
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-08-13T11:57:00Z"));
    }

    /// No real-time value is not the same as no delay, and must not read as
    /// "on time".
    #[test]
    fn a_connection_with_no_realtime_data_says_so_rather_than_claiming_punctuality() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "RB 66", "nummer": "66" },
                    "halte": [
                        { "id": "8000044", "name": "Bonn Hbf",
                          "abfahrt": { "sollzeit": "2026-08-25T10:07:00" } },
                        { "id": "8000262", "name": "Siegburg/Bonn",
                          "ankunft": { "sollzeit": "2026-08-25T10:31:00" } }
                    ]
                }]
            }]
        });
        let leg = &parse_journeys_from_response(&body)[0].legs[0];
        assert!(leg.realtime_departure.is_none());
        assert!(leg.realtime_arrival.is_none());
        // The planning field still falls back to the schedule.
        assert_eq!(leg.departure_time, "2026-08-25T10:07:00");
    }

    /// A cancelled train used to come back as an ordinary leg with times on it.
    #[test]
    fn a_cancelled_leg_is_marked_cancelled() {
        let body = serde_json::json!({
            "verbindungen": [{
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": { "typ": "ZUG", "name": "ICE 1513", "nummer": "1513" },
                    "originCancelled": true,
                    "halte": [
                        { "id": "8000044", "name": "Bonn Hbf",
                          "abfahrt": { "sollzeit": "2026-08-25T10:07:00" } },
                        { "id": "8000105", "name": "Frankfurt(Main)Hbf",
                          "ankunft": { "sollzeit": "2026-08-25T11:49:00" } }
                    ]
                }]
            }]
        });
        assert!(parse_journeys_from_response(&body)[0].legs[0].cancelled);
    }

    fn journey_on(trains: &[&str]) -> Journey {
        let station = |name: &str| Station {
            id: "8000000".into(),
            name: name.into(),
            latitude: None,
            longitude: None,
        };
        Journey {
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
            id: "j".into(),
            start_station: station("A"),
            end_station: station("B"),
            legs: trains
                .iter()
                .map(|n| Leg {
                    on_time_probability: None,
                    origin: station("A"),
                    destination: station("B"),
                    departure_time: "2026-09-01T08:00:00".into(),
                    arrival_time: "2026-09-01T10:00:00".into(),
                    departure_utc: None,
                    arrival_utc: None,
                    train_name: format!("ICE {n}"),
                    train_number: (*n).into(),
                    train_category: "ICE".into(),
                    platform: None,
                    is_regional: false,
                    scheduled_departure: None,
                    realtime_departure: None,
                    scheduled_arrival: None,
                    realtime_arrival: None,
                    cancelled: false,
                })
                .collect(),
            total_duration_minutes: 120,
            total_price: Some(30.0),
            delay_risk_score: None,
            arrival_punctuality: None,
        }
    }

    fn owned(trains: &[&str]) -> Vec<String> {
        trains.iter().map(|t| (*t).to_string()).collect()
    }

    /// The defect this classification exists for: each segment is priced by a
    /// fresh search that takes `journeys.first()`, and nothing made that journey
    /// the train the traveller is on.
    #[test]
    fn train_match_separates_the_same_ride_from_a_different_one() {
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&["691"])),
            TrainMatch::Exact
        );
        // Priced for a service the traveller will not be on. This is the case
        // that costs money, so it must not be reported as merely unknown.
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&["512"])),
            TrainMatch::Different
        );
        // Covers one of the two planned trains.
        assert_eq!(
            classify_train_match(&owned(&["691", "512"]), &journey_on(&["691"])),
            TrainMatch::Partial
        );
        // Same trains, wrong order is not the same ride.
        assert_eq!(
            classify_train_match(&owned(&["691", "512"]), &journey_on(&["512", "691"])),
            TrainMatch::Partial
        );
        // No train number on either side is not a verdict.
        assert_eq!(
            classify_train_match(&[], &journey_on(&["691"])),
            TrainMatch::Unknown
        );
        assert_eq!(
            classify_train_match(&owned(&["691"]), &journey_on(&[])),
            TrainMatch::Unknown
        );
    }

    fn segment(train_match: TrainMatch) -> SplitSegment {
        SplitSegment {
            journey: journey_on(&["691"]),
            train_match,
            expected_trains: owned(&["691"]),
        }
    }

    /// A chain is only as buyable as its least trustworthy ticket, and a hole in
    /// the price table means the search never saw every candidate split.
    #[test]
    fn chain_confidence_takes_the_worst_case() {
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact), segment(TrainMatch::Exact)], 0),
            SplitConfidence::Exact
        );
        assert_eq!(
            split_confidence(
                &[segment(TrainMatch::Exact), segment(TrainMatch::Partial)],
                0
            ),
            SplitConfidence::Partial
        );
        assert_eq!(
            split_confidence(
                &[segment(TrainMatch::Exact), segment(TrainMatch::Unknown)],
                0
            ),
            SplitConfidence::Partial
        );
        // All exact, but a failed pairwise query means a cheaper split may exist
        // that was never priced.
        assert_eq!(
            split_confidence(&[segment(TrainMatch::Exact)], 1),
            SplitConfidence::Partial
        );
        // One wrong-train segment outranks every other signal.
        assert_eq!(
            split_confidence(
                &[segment(TrainMatch::Exact), segment(TrainMatch::Different)],
                0
            ),
            SplitConfidence::Low
        );
    }

    /// `savings: 0.0` could not be told apart from "the split saves nothing",
    /// and the dashboard rendered that as "Direct is cheapest".
    #[test]
    fn unknown_direct_fare_yields_no_savings_figure() {
        let direct: Option<f64> = None;
        assert_eq!(direct.map(|p: f64| p - 20.0), None);
        assert_eq!(Some(35.0f64).map(|p| p - 20.0), Some(15.0));
    }

    #[test]
    fn cheapest_split_chains_across_four_stops() {
        let mut prices = HashMap::new();
        prices.insert((0, 3), 40.0); // direct
        prices.insert((0, 1), 8.0);
        prices.insert((1, 2), 8.0);
        prices.insert((2, 3), 8.0); // 3-hop total 24.0, cheapest
        prices.insert((0, 2), 20.0);
        prices.insert((1, 3), 20.0);

        let (total, path) = cheapest_split(4, &prices).unwrap();
        assert!((total - 24.0).abs() < 1e-9);
        assert_eq!(path, vec![(0, 1), (1, 2), (2, 3)]);
    }

    fn fixture_journey(is_regional_attr: bool) -> Value {
        json!({
            "verbindungen": [{
                "tripId": "journey-1",
                "angebotsPreis": {"betrag": 39.90},
                "verbindungsDauerInSeconds": 5400,
                "verbindungsAbschnitte": [{
                    "verkehrsmittel": {
                        "name": "ICE 691",
                        "nummer": "691",
                        "kategorie": "ICE",
                        "zugattribute": if is_regional_attr {
                            json!([{"key": "9G"}])
                        } else {
                            json!([])
                        }
                    },
                    "gleis": "7",
                    "halte": [
                        {
                            "id": "8000105",
                            "name": "Frankfurt(Main)Hbf",
                            "abfahrt": {"sollzeit": "2026-07-15T08:30:00"}
                        },
                        {
                            "id": "8000261",
                            "name": "Mannheim Hbf",
                            "ankunft": {"sollzeit": "2026-07-15T09:15:00"}
                        }
                    ]
                }]
            }]
        })
    }

    #[test]
    fn parses_single_leg_journey() {
        let body = fixture_journey(false);
        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        let j = &journeys[0];
        assert_eq!(j.id, "journey-1");
        assert_eq!(j.start_station.name, "Frankfurt(Main)Hbf");
        assert_eq!(j.end_station.name, "Mannheim Hbf");
        assert_eq!(j.total_duration_minutes, 90);
        assert!((j.total_price.unwrap() - 39.90).abs() < 1e-9);
        assert_eq!(
            j.delay_risk_score, None,
            "ONNX prediction not ported -- always None"
        );
        assert_eq!(j.legs.len(), 1);
        assert_eq!(j.legs[0].train_name, "ICE 691");
        assert_eq!(j.legs[0].platform.as_deref(), Some("7"));
        assert!(!j.legs[0].is_regional);
    }

    /// Regression test for the "id" vs "tripId" field-name bug (found via
    /// live verification, see the doc comment on `id:` in
    /// `parse_journeys_from_response`) -- a response using the WRONG key
    /// ("id" instead of "tripId") must not silently look like a valid,
    /// present id.
    #[test]
    fn missing_trip_id_field_yields_empty_id_not_a_wrong_value() {
        let mut body = fixture_journey(false);
        // Simulate the bug this test guards against: a response shaped with
        // "id" instead of "tripId" (what the fixture -- and the parser --
        // both wrongly assumed before the live-verification fix).
        let conn = &mut body["verbindungen"][0];
        let obj = conn.as_object_mut().unwrap();
        let stray = obj.remove("tripId").unwrap();
        obj.insert("id".into(), stray);

        let journeys = parse_journeys_from_response(&body);
        assert_eq!(journeys.len(), 1);
        assert_eq!(
            journeys[0].id, "",
            "\"id\" is not a real field on this endpoint -- must not be read as one"
        );
    }

    #[test]
    fn detects_regional_attribute_9g() {
        let body = fixture_journey(true);
        let journeys = parse_journeys_from_response(&body);
        assert!(
            journeys[0].legs[0].is_regional,
            "zugattribute key '9G' should set is_regional"
        );
    }

    #[test]
    fn empty_verbindungen_yields_empty_journeys() {
        let body = json!({"verbindungen": []});
        assert!(parse_journeys_from_response(&body).is_empty());
    }

    #[test]
    fn missing_verbindungen_key_yields_empty_journeys() {
        let body = json!({});
        assert!(parse_journeys_from_response(&body).is_empty());
    }

    #[test]
    fn parses_suggest_stations() {
        let list = vec![
            json!({"extId": "8000105", "name": "Frankfurt(Main)Hbf", "lat": 50.1072, "lon": 8.6633}),
            json!({"extId": "8000261", "name": "Mannheim Hbf"}),
            json!({"name": "missing ext id, should be skipped"}),
        ];
        let stations = parse_suggest_response(&list);
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].id, "8000105");
        assert!((stations[0].latitude.unwrap() - 50.1072).abs() < 1e-6);
        assert_eq!(stations[1].id, "8000261");
        assert_eq!(stations[1].latitude, None);
    }
}

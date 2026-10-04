//! `tools/sparpreis-watch` — one pass of the Sparpreis price watch, then exit.
//!
//! Ported from `tools/sparpreis-watch.ts` on 2026-10-04, its pure half to `pure.rs` and its test
//! with it. `capabilities/sparpreis-watch/service.toml` names this binary directly, so the
//! twelve-hourly job starts neither an interpreter nor a shell.
//!
//! ## Why the watch exists
//!
//! The research verdict this implements (travel PRD R4, 2026-08-12): Sparpreis prices for a
//! specific train DO fall, but rarely and unpredictably — later-released cheap contingents and DB
//! promo windows are the two real events. Watching a booked train is dead weight; watching a
//! not-yet-booked trip is one cheap cron.
//!
//! ## What it watches, per upcoming plan
//!
//! - Every train stage that is not booked or completed, searched by the stage's own place names on
//!   its date. Before 2026-09-25 only `option_set` items were watched, so a plan whose route
//!   changed kept watching the old route and never the new one: the Berlin plan watched
//!   Bonn → Berlin for six weeks after its stages became Bonn → Stuttgart → Berlin.
//! - Rail `option_set` items (the solver and the agent surface write those), but only while they
//!   still match an unbooked train stage. A plan with no train stage at all keeps the old
//!   behaviour and watches every one.
//!
//! Each watch keeps ONE `option_set` item, `sparpreis-watch:<key>`, whose payload carries the
//! whole price history. It used to write one item per day, which put 30 near-identical rows in one
//! plan; `consolidate` folds those into the single item and deletes them.
//!
//! A `note` item is written when today's cheapest fare is below every earlier observation. "Below
//! the last check" flagged €67.99 → €47.99 as news when €39.99 had already been seen. Durable plan
//! state is the alert surface.
//!
//! ## Composition
//!
//! It talks to trips and transit over HTTP and never to their databases — the documented
//! composition edge (CONTRIBUTING.md#schemas-and-dependency-direction). Fare context
//! (bc, d_ticket, first_class) is replayed from the watched query, so a drop is a drop in the price
//! the traveller would actually pay.
//!
//! ## Differences from the TypeScript, all deliberate
//!
//! The deployment credential is put on each request by `sjel_server::InboundAuth::with_loopback_auth`
//! — the same helper `capabilities/calendar` and every other Rust caller of a loopback capability
//! uses — instead of the TypeScript's own `authorizedLoopbackRequest`. Every request carries a
//! 300-second timeout through `sjel_http::client`, where the TypeScript used the platform's `fetch`
//! default, which is to say none; that is also where the user-agent and the redirect policy come
//! from. A missing credential is a one-line refusal rather than an unhandled rejection. A failed
//! item write or delete is reported and the run continues, where a network-level throw in the
//! TypeScript ended it. JSON object keys are sorted (`serde_json`'s default) where the TypeScript
//! wrote insertion order; a payload is read by name, so nothing that reads it can tell.

mod pure;

use crate::paths::{manifest_port, Paths};
use pure::{
    dropped, history_of, legacy_observations, lowest_seen, rail_watches_of, stage_watches_of,
    still_planned, train_stages, watch_key, with_observation, Observation, RailWatch, Stage,
};
use serde_json::{json, Value};
use sjel_http::Purpose;
use std::process::ExitCode;
use std::time::Duration;

const HELP: &str = "\
tools/sparpreis-watch — one pass of the Sparpreis price watch, then exit.

  tools/sparpreis-watch      re-price every watched rail search and record the result
  tools/sparpreis-watch -h   this help

trips and transit must be answering on their declared ports.
Schedule: capabilities/sparpreis-watch/service.toml
";

/// The TypeScript timed no request at all. `sjel_http::client` refuses to build one without a
/// timeout, and a search that reaches bahn.de through transit's self-paced client deserves
/// generous room: 300 s is the budget `tools/feed-sweep` gives its scan, and it is short enough
/// that a wedged request cannot hold the job past its next slot.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// At most this many searches in one run. Each one is a real bahn.de request through transit.
const CAP: usize = 10;

/// The pause between two searches. The endpoint under this is bahn.de via transit; transit paces
/// itself, and this keeps a multi-watch run from bursting anyway.
const BETWEEN_SEARCHES: Duration = Duration::from_millis(1000);

fn fail(message: &str) -> ExitCode {
    eprintln!("sparpreis-watch: {message}");
    ExitCode::from(1)
}

pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let paths = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let (trips_port, transit_port) = match (
        manifest_port(&paths, "trips"),
        manifest_port(&paths, "transit"),
    ) {
        (Ok(trips), Ok(transit)) => (trips, transit),
        (Err(e), _) | (_, Err(e)) => return fail(&e),
    };
    // Checked once, so an unconfigured deployment is one line rather than a failed request per
    // watch. `with_loopback_auth` re-reads it per call and adds no header when there is none.
    if sjel_server::InboundAuth::from_deployment()
        .bearer_header()
        .is_none()
    {
        return fail("deployment inbound credential is not configured");
    }
    let client = match sjel_http::client(Purpose::new("sparpreis-watch"), REQUEST_TIMEOUT) {
        Ok(c) => c,
        Err(e) => return fail(&format!("client build: {e}")),
    };

    let mut watcher = Watcher {
        client,
        trips: format!("http://127.0.0.1:{trips_port}"),
        transit: format!("http://127.0.0.1:{transit_port}"),
        today: civil_date::today(),
        watched: 0,
        drops: 0,
    };
    match watcher.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

#[derive(Debug, serde::Deserialize)]
struct Plan {
    id: String,
    #[serde(default)]
    date_start: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, serde::Deserialize)]
struct Station {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct Journey {
    /// Kept as JSON rather than `Option<f64>`: the TypeScript filters non-numbers out, so a null,
    /// a missing field and anything else unpriceable all have to survive deserialization.
    #[serde(default)]
    total_price: Value,
    #[serde(default)]
    start_station: Option<Station>,
    #[serde(default)]
    end_station: Option<Station>,
}

struct Watcher {
    client: reqwest::blocking::Client,
    trips: String,
    transit: String,
    today: String,
    watched: usize,
    drops: usize,
}

impl Watcher {
    fn run(&mut self) -> Result<(), String> {
        let url = format!("{}/api/plans", self.trips);
        let response = self.get(&url)?;
        let plans: Vec<Plan> = response.json().map_err(|e| format!("{url}: {e}"))?;
        let today = self.today.clone();
        let upcoming: Vec<&Plan> = plans.iter().filter(|p| p.date_start >= today).collect();
        println!(
            "sparpreis-watch: {}/{} plans upcoming",
            upcoming.len(),
            plans.len()
        );

        for plan in upcoming {
            let (mut items, mut stages) = self.load_items(&plan.id)?;
            let folded = self.consolidate(&plan.id, &items);
            if folded > 0 {
                println!(
                    "sparpreis-watch: folded {folded} per-day observations into single items ({})",
                    plan.title
                );
                (items, stages) = self.load_items(&plan.id)?;
            }

            // Stages first: their searches resolve the station pairs the option_sets are matched
            // against.
            let mut stage_legs: Vec<String> = Vec::new();
            for watch in stage_watches_of(&plan.id, &stages, &today) {
                if self.watched >= CAP {
                    break;
                }
                let journeys = self.observe(&plan.id, &plan.title, &items, &watch)?;
                for journey in journeys.unwrap_or_default() {
                    let (Some(start), Some(end)) = (
                        journey.start_station.and_then(|s| s.id),
                        journey.end_station.and_then(|s| s.id),
                    ) else {
                        continue;
                    };
                    stage_legs.push(pure::leg_key(&start, &end, &watch.time));
                }
            }

            let has_train_stages = !train_stages(&stages).is_empty();
            for watch in rail_watches_of(&plan.id, &items) {
                if !still_planned(&watch, has_train_stages, &stage_legs) {
                    println!(
                        "sparpreis-watch: {}->{} {} matches no unbooked stage, not watched ({})",
                        watch.from, watch.to, watch.time, plan.title
                    );
                    continue;
                }
                if self.watched >= CAP {
                    break;
                }
                self.observe(&plan.id, &plan.title, &items, &watch)?;
            }
            if self.watched >= CAP {
                println!("sparpreis-watch: cap of {CAP} reached, remaining watches skipped this run");
                break;
            }
        }
        println!(
            "sparpreis-watch: {} watched, {} new lows",
            self.watched, self.drops
        );
        Ok(())
    }

    /// One search, recorded on the watch's single item. Returns the journeys, or `None` when the
    /// search failed or priced nothing.
    fn observe(
        &mut self,
        plan_id: &str,
        plan_title: &str,
        items: &[Value],
        watch: &RailWatch,
    ) -> Result<Option<Vec<Journey>>, String> {
        self.watched += 1;
        std::thread::sleep(BETWEEN_SEARCHES);

        let mut url = reqwest::Url::parse(&format!("{}/api/search", self.transit))
            .map_err(|e| format!("bad transit URL: {e}"))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("from", &watch.from);
            query.append_pair("to", &watch.to);
            query.append_pair("time", &watch.time);
            if let Some(bc) = watch.bc {
                query.append_pair("bc", &bc.to_string());
            }
            if watch.d_ticket {
                query.append_pair("d_ticket", "true");
            }
            if watch.first_class {
                query.append_pair("first_class", "true");
            }
        }

        let response = self.get(url.as_str())?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().unwrap_or_default();
            eprintln!(
                "sparpreis-watch: search {}->{} HTTP {status}: {}",
                watch.from,
                watch.to,
                truncate(&body, 200)
            );
            return Ok(None);
        }
        let journeys: Vec<Journey> = response
            .json()
            .map_err(|e| format!("search {url} returned something unreadable: {e}"))?;
        let prices: Vec<f64> = journeys
            .iter()
            .filter_map(|j| j.total_price.as_f64())
            .collect();
        if prices.is_empty() {
            println!(
                "sparpreis-watch: {}->{} returned no priced journey",
                watch.from, watch.to
            );
            return Ok(Some(journeys));
        }
        let cheapest = prices.iter().copied().fold(f64::INFINITY, f64::min);
        let key = watch_key(watch);
        let earlier = history_of(items, &key);
        let low = lowest_seen(&earlier);
        let history = with_observation(
            earlier,
            Observation {
                day: self.today.clone(),
                prices,
            },
        );
        let from_name = journeys
            .first()
            .and_then(|j| j.start_station.as_ref())
            .and_then(|s| s.name.clone())
            .unwrap_or_else(|| watch.from.clone());
        let to_name = journeys
            .first()
            .and_then(|j| j.end_station.as_ref())
            .and_then(|s| s.name.clone())
            .unwrap_or_else(|| watch.to.clone());
        let day = day_of(&watch.time);

        let wrote = self.post_item(
            plan_id,
            json!({
                "item_type": "option_set",
                "external_id": format!("sparpreis-watch:{key}"),
                "title": format!("Sparpreis watch {from_name} → {to_name}, {day}"),
                "payload": {
                    "query": {
                        "from": watch.from,
                        "to": watch.to,
                        "time": watch.time,
                        "bc": watch.bc,
                        "stage_id": watch.stage_id,
                    },
                    "observed_at": crate::time::now_iso(),
                    "options": journeys.iter().take(5)
                        .map(|j| json!({ "total_price": j.total_price }))
                        .collect::<Vec<_>>(),
                    "history": history_json(&history),
                    "lowest": lowest_seen(&history),
                },
            }),
        );
        if !wrote {
            return Ok(Some(journeys));
        }
        if dropped(low, cheapest) {
            self.drops += 1;
            let previous = low.unwrap_or_default();
            self.post_item(
                plan_id,
                json!({
                    "item_type": "note",
                    "external_id": format!("sparpreis-low:{key}:{}", self.today),
                    "title": format!(
                        "Sparpreis new low {from_name} → {to_name}, {day}: €{cheapest} (lowest before: €{previous})"
                    ),
                    "payload": {
                        "previous_low": low,
                        "current": cheapest,
                        "watched_time": watch.time,
                        "stage_id": watch.stage_id,
                    },
                }),
            );
            println!(
                "sparpreis-watch: NEW LOW {from_name}->{to_name} €{previous} -> €{cheapest} ({plan_title})"
            );
        } else {
            println!(
                "sparpreis-watch: {from_name}->{to_name} cheapest €{cheapest}{}",
                match low {
                    Some(low) => format!(" (lowest seen €{low})"),
                    None => " (first observation)".to_owned(),
                }
            );
        }
        Ok(Some(journeys))
    }

    fn load_items(&self, plan_id: &str) -> Result<(Vec<Value>, Vec<Stage>), String> {
        let url = format!("{}/api/plans/{}", self.trips, encode_component(plan_id));
        let details: Value = self
            .get(&url)?
            .json()
            .map_err(|e| format!("{url}: {e}"))?;
        let items = details
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let stages = details
            .get("stages")
            .cloned()
            .and_then(|v| serde_json::from_value::<Vec<Stage>>(v).ok())
            .unwrap_or_default();
        Ok((items, stages))
    }

    /// Folds the per-day observation items into one item per watch, then deletes them. The single
    /// item is written first, so a failed delete leaves a duplicate rather than a gap: `history_of`
    /// reads both and counts each day once.
    fn consolidate(&self, plan_id: &str, items: &[Value]) -> usize {
        let mut removed = 0usize;
        for (key, legacy) in legacy_observations(items) {
            let Some(latest) = legacy.iter().max_by(|a, b| a.day.cmp(&b.day)) else {
                continue;
            };
            let history = history_of(items, &key);
            let stations = key.split(':').take(2).collect::<Vec<_>>().join(" → ");
            let wrote = self.post_item(
                plan_id,
                json!({
                    "item_type": "option_set",
                    "external_id": format!("sparpreis-watch:{key}"),
                    "title": format!("Sparpreis watch {stations}"),
                    "payload": {
                        "query": latest.payload.get("query").cloned().unwrap_or_else(|| json!({})),
                        "options": latest.payload.get("options").cloned().unwrap_or_else(|| json!([])),
                        "observed_at": format!("{}T00:00:00Z", latest.day),
                        "history": history_json(&history),
                        "lowest": lowest_seen(&history),
                    },
                }),
            );
            if !wrote {
                continue;
            }
            for item in &legacy {
                let Some(id) = item.id.as_deref() else {
                    continue;
                };
                let external_id = item.external_id.clone().unwrap_or_default();
                let url = format!(
                    "{}/api/plans/{}/items/{}",
                    self.trips,
                    encode_component(plan_id),
                    encode_component(id)
                );
                match self.delete(&url) {
                    Ok(response) if response.status().is_success() => removed += 1,
                    Ok(response) => eprintln!(
                        "sparpreis-watch: delete {external_id} HTTP {}",
                        response.status().as_u16()
                    ),
                    Err(e) => eprintln!("sparpreis-watch: delete {external_id} failed — {e}"),
                }
            }
        }
        removed
    }

    fn post_item(&self, plan_id: &str, body: Value) -> bool {
        let url = format!("{}/api/plans/{}/items", self.trips, encode_component(plan_id));
        let text = match serde_json::to_string(&body) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("sparpreis-watch: item write could not be encoded — {e}");
                return false;
            }
        };
        let request = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(text);
        match sjel_server::InboundAuth::with_loopback_auth(request, &url).send() {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                eprintln!(
                    "sparpreis-watch: item write HTTP {}",
                    response.status().as_u16()
                );
                false
            }
            Err(e) => {
                eprintln!("sparpreis-watch: item write failed — {e}");
                false
            }
        }
    }

    fn get(&self, url: &str) -> Result<reqwest::blocking::Response, String> {
        sjel_server::InboundAuth::with_loopback_auth(self.client.get(url), url)
            .send()
            .map_err(|e| format!("GET {url}: {e}"))
    }

    fn delete(&self, url: &str) -> Result<reqwest::blocking::Response, String> {
        sjel_server::InboundAuth::with_loopback_auth(self.client.delete(url), url)
            .send()
            .map_err(|e| format!("DELETE {url}: {e}"))
    }
}

/// The history as the payload carries it: `[{"day": ..., "prices": [...]}]`.
fn history_json(history: &[Observation]) -> Vec<Value> {
    history
        .iter()
        .map(|o| json!({ "day": o.day, "prices": o.prices }))
        .collect()
}

/// The day part of a departure time, `slice(0, 10)` without the byte-slicing panic.
fn day_of(time: &str) -> &str {
    time.get(..10).unwrap_or(time)
}

/// `body.slice(0, n)`, counting characters rather than bytes so a multi-byte body cannot split.
fn truncate(body: &str, n: usize) -> String {
    body.chars().take(n).collect()
}

/// `encodeURIComponent`: everything but `A-Za-z0-9-_.!~*'()` is percent-encoded, byte by byte, so
/// a multi-byte character is encoded the way the platform encodes it. `sjel-mcp` has the same set
/// as an `AsciiSet` for the requests it builds; this one is written out so the crate needs no
/// crate for three call sites' worth of path segments.
fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
        {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_id_is_encoded_as_encodeuricomponent_encodes_it() {
        assert_eq!(encode_component("1a2b-3c_4d.5~6"), "1a2b-3c_4d.5~6");
        assert_eq!(encode_component("a b/c?d#e"), "a%20b%2Fc%3Fd%23e");
        assert_eq!(encode_component("ä"), "%C3%A4");
        assert_eq!(encode_component("!*'()"), "!*'()");
    }

    #[test]
    fn a_history_is_written_as_day_and_prices() {
        assert_eq!(
            history_json(&[Observation {
                day: "2026-09-01".to_owned(),
                prices: vec![29.99, 35.0]
            }]),
            vec![json!({ "day": "2026-09-01", "prices": [29.99, 35.0] })]
        );
    }

    #[test]
    fn a_short_time_does_not_panic_on_the_day_slice() {
        assert_eq!(day_of("2026-09-01T08:00:00"), "2026-09-01");
        assert_eq!(day_of("2026-09-01"), "2026-09-01");
        assert_eq!(day_of(""), "");
    }
}

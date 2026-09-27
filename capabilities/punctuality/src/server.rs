//! punctuality-server — the contract other capabilities read this through.
//!
//! `capabilities/transit` needs these numbers and must not reach into this crate to get
//! them (README.md#schemas-and-dependency-direction): a capability depends on another's HTTP surface and schema,
//! never its code. This is that surface.
//!
//! Axum + tokio here, sync `postgres` underneath, same split transit-server already
//! makes: serving needs an async runtime, the queries do not, so blocking work goes
//! through `spawn_blocking` rather than dragging an async driver into the library.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use punctuality::config::Config;
use punctuality::store::{StatRow, Store};

/// One stop to look up. `hour` and `weekend` are the caller's, because only the caller
/// knows which of a leg's two timestamps it is asking about — departure at the origin
/// or arrival at the destination.
/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r(
        "POST",
        "/lookup",
        "Punctuality for a connection. Each stop may carry at_least_minutes to ask for the \
         share of trains at least that late, answered from the stored histogram; null when \
         the row predates it.",
    ),
    r("GET", "/stations", "Station search. Requires a query."),
];

/// Shorthand so the table above reads as a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("punctuality", ROUTES))
}

#[derive(Debug, Deserialize)]
struct StopQuery {
    eva: String,
    train_type: String,
    hour: u8,
    #[serde(default)]
    weekend: bool,
    /// Ask for the share of trains at least this many minutes late. Optional:
    /// omitted, the reply carries the six-minute figure it always did.
    ///
    /// Six minutes was never a considered threshold, it was the one that got its
    /// own column. A transfer with a four-minute buffer and one with a twelve-
    /// minute buffer are different questions, and until now neither could be
    /// asked.
    at_least_minutes: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct LookupBody {
    stops: Vec<StopQuery>,
}

/// What one stop's history says. `null` in the response array where nothing is known —
/// distinct from a zero-risk answer, which is why this is an Option at the array level
/// and not a row of zeroes.
#[derive(Debug, Serialize)]
struct StopStats {
    eva: String,
    station_name: Option<String>,
    train_type: String,
    hour: i16,
    weekend: bool,
    n: i64,
    mean_delay: f32,
    p50: i16,
    p90: i16,
    share_late_6: f32,
    cancel_rate: f32,
    /// The exceedance at the requested threshold.
    ///
    /// Three states, and they are different answers. Absent: nobody asked.
    /// Explicit `null`: asked, and this row predates the stored histogram, so the
    /// question cannot be answered from it. A number: answered. Collapsing the
    /// middle case into absence would let a caller that asked read the silence as
    /// zero risk, which is the failure this endpoint exists to avoid.
    #[serde(skip_serializing_if = "Option::is_none")]
    share_delay_at_least: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    at_least_minutes: Option<i32>,
}

impl From<StatRow> for StopStats {
    fn from(r: StatRow) -> Self {
        Self {
            eva: r.eva,
            station_name: r.station_name,
            train_type: r.train_type,
            hour: r.hour,
            weekend: r.weekend,
            n: r.n,
            mean_delay: r.mean_delay,
            p50: r.p50,
            p90: r.p90,
            share_late_6: r.share_late_6,
            cancel_rate: r.cancel_rate,
            share_delay_at_least: None,
            at_least_minutes: None,
        }
    }
}

impl StopStats {
    /// Answers the caller's own threshold from the stored histogram.
    fn with_threshold(mut self, row: &StatRow, minutes: Option<i32>) -> Self {
        let Some(minutes) = minutes else { return self };
        self.at_least_minutes = Some(minutes);
        // Still an inner option, and still the same three wire states. Whether a
        // histogram can answer an arbitrary threshold is
        // `share_at_least_from_counts`'s verdict -- it refuses an array of the wrong
        // length -- rather than a nullable column's.
        self.share_delay_at_least = Some(punctuality::stats::Cell::share_at_least_from_counts(
            &row.counts,
            minutes,
        ));
        self
    }
}

/// Cells thinner than this are reported as unknown rather than as a statistic. A 100%
/// late rate over four observations is noise wearing a percentage sign, and a consumer
/// cannot tell the difference once it is a float in a JSON field.
const MIN_SAMPLE: i64 = 30;

#[derive(Clone)]
struct AppState {
    /// `Arc<Store>`, not `Arc<Mutex<Store>>`. `Store` is a pool handle plus a schema
    /// name and mutates neither, so the lock guarded nothing while serialising every
    /// query in front of an r2d2 pool that is already thread-safe. It also made any
    /// panic inside a handler permanent: a poisoned `Mutex` fails every later request,
    /// `/health` included, until the process restarts. Issue #175 was exactly that,
    /// triggered by a projection bug two calls deep.
    store: Arc<Store>,
}

type ApiResult = Result<Json<Value>, (axum::http::StatusCode, String)>;

fn fail(e: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
    // Every capability server answers a failure as {"error": "..."} -- the dashboard's
    // client unwraps exactly that field. transit's /api/split still returns a bare
    // sentence and is the odd one out; this one is not going to be the second.
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": e.to_string() }).to_string(),
    )
}

async fn handle_health(State(state): State<AppState>) -> ApiResult {
    let store = state.store;
    let covered = tokio::task::spawn_blocking(move || store.coverage().map_err(|e| e.to_string()))
        .await
        .map_err(fail)?
        .map_err(fail)?;

    Ok(Json(json!({
        "ok": covered.is_some(),
        "capability": "punctuality",
        // A server that answers but has never ingested is up and useless. Saying which
        // window it holds is the difference between "no data for that train" meaning
        // "punctual" and meaning "nobody has run ingest".
        "coverage": covered.map(|(from, to, cells, ran_at)| json!({
            "from_month": from, "to_month": to, "cells": cells,
            "last_ingest_at": ran_at
        })),
    })))
}

async fn handle_lookup(State(state): State<AppState>, Json(body): Json<LookupBody>) -> ApiResult {
    if body.stops.len() > 200 {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            json!({ "error": "at most 200 stops per lookup" }).to_string(),
        ));
    }
    let store = state.store;
    let stats = tokio::task::spawn_blocking(move || -> Result<Vec<Option<StopStats>>, String> {
        body.stops
            .iter()
            .map(|s| {
                store
                    .stop_stats(&s.eva, &s.train_type, s.hour as i16, s.weekend, MIN_SAMPLE)
                    .map(|opt| {
                        opt.map(|row| {
                            StopStats::from(row.clone()).with_threshold(&row, s.at_least_minutes)
                        })
                    })
                    .map_err(|e| e.to_string())
            })
            .collect()
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    Ok(Json(json!({ "stats": stats, "min_sample": MIN_SAMPLE })))
}

#[derive(Deserialize)]
struct StationQuery {
    eva: Option<String>,
    q: Option<String>,
    train_type: Option<String>,
}

async fn handle_station(State(state): State<AppState>, Query(p): Query<StationQuery>) -> ApiResult {
    let store = state.store;
    let value = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        if let Some(q) = p.q {
            let hits = store.find_stations(&q).map_err(|e| e.to_string())?;
            return Ok(json!(hits
                .into_iter()
                .map(|(eva, name)| json!({ "eva": eva, "station_name": name }))
                .collect::<Vec<_>>()));
        }
        let eva = p.eva.ok_or("pass eva=<code> or q=<name fragment>")?;
        let rows = store
            .station_stats(&eva, p.train_type.as_deref(), MIN_SAMPLE)
            .map_err(|e| e.to_string())?;
        Ok(json!(rows
            .into_iter()
            .map(StopStats::from)
            .collect::<Vec<_>>()))
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    Ok(Json(value))
}

#[tokio::main]
async fn main() {
    let cfg = Config::load();
    // rusqlite is a blocking API over a file: an open on an async worker holds that
    // thread for the migration and, under a busy writer, for busy_timeout as well.
    // spawn_blocking moves it off. transit-server does the same dance for
    // reqwest::blocking -- same reason, different library.
    // The error is stringified inside the closure because Box<dyn Error> is not Send
    // and so cannot cross the spawn_blocking boundary.
    let path = cfg.database_path.clone();
    let store =
        match tokio::task::spawn_blocking(move || Store::open(&path).map_err(|e| e.to_string()))
            .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                eprintln!("punctuality-server: cannot reach the database: {e}");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("punctuality-server: store construction panicked: {e}");
                std::process::exit(1);
            }
        };
    let state = AppState {
        store: Arc::new(store),
    };

    // SJEL_PORT is what service-runner.sh exports from the manifest, so the port lives
    // in one file rather than two -- resolution itself lives in sjel_server.
    let port = sjel_server::resolve_port(None, None, 8085);

    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(handle_health))
        .route("/lookup", post(handle_lookup))
        .route("/stations", get(handle_station))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // 127.0.0.1: this reads a database of public data, but it is still a local service
    // with no auth, and binding it to the LAN would be a decision nobody made. That
    // rationale is now sjel_server::serve_local's policy (this crate's bind-failure
    // behavior became the shared default there, too).
    sjel_server::serve_local("punctuality-server", port, app).await;
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}

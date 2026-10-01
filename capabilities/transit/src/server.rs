use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use transit::config::Config;
use transit::hafas::HafasClient;
use transit::store::TransitStore;

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/health",
        "Liveness under the API prefix. Same handler as /health.",
    ),
    r("GET", "/api/suggest", "Station suggestions for a query."),
    r(
        "GET",
        "/api/search",
        "Fare search between two stations on a date. Query: to (EVA) and time \
         (YYYY-MM-DDTHH:MM:SS, seconds optional; anything else is a 400); from (EVA) is \
         optional and defaults to the profile's first home station. Optional bc (25|50), \
         first_class, d_ticket carry the fare context and default from the cards the \
         profile says the traveller holds, so returned prices are discount-correct. \
         Optional priority (cheapest|fastest|fewest_changes|reliable|balanced), weights \
         (price:0.5,duration:0.2,changes:0.1,reliability:0.2, summing to 1.0) or phrase (a \
         sentence: cheapest, direct, schnell und zuverlaessig) override the profile's journey \
         weights for this one search; passing more than one is a 400, and a phrase naming \
         nothing known is a 400 naming the vocabulary. When the \
         profile states journey weights, each journey gains a `ranking` with a score, its \
         rank, the weights and where they came from, and a factor per reason.",
    ),
    r(
        "GET",
        "/api/split",
        "Split-ticket options for a search. Same query as /api/search including the \
         fare context: BahnCard applies per Fahrkarte, so every candidate segment is \
         priced the way it would actually be bought.",
    ),
    r(
        "GET",
        "/api/trips",
        "Saved trip searches with their legs. Optional session_id filters to one \
         `transit plan` session; optional limit (default 100, max 500) bounds the read, \
         and the reply's count/returned/truncated say what was left behind.",
    ),
    r(
        "POST",
        "/api/tickets/extract",
        "Parse a rail ticket confirmation. Body is the raw file bytes; file_name (query) \
         picks the reader by extension (pdf, eml, txt, html). Returns the parse for review \
         and stores nothing: the parser is not fit to run unattended, see the README.",
    ),
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
    Json(route_manifest::manifest("transit", ROUTES))
}

#[derive(Deserialize)]
struct SuggestQuery {
    q: String,
}

#[derive(Deserialize)]
struct RouteQuery {
    /// Optional. Omitted, the profile's first home station is the origin — which
    /// is the point of the operator naming three: a search should not have to be
    /// told where they live.
    #[serde(default)]
    from: Option<String>,
    to: String,
    time: String,
    /// 25 or 50; the payload builder rejects anything else loudly.
    #[serde(default)]
    bc: Option<u8>,
    #[serde(default)]
    first_class: bool,
    #[serde(default)]
    d_ticket: bool,
    /// A preset: cheapest, fastest, fewest_changes, reliable, balanced.
    #[serde(default)]
    priority: Option<String>,
    /// The explicit form: `price:0.5,duration:0.2,changes:0.1,reliability:0.2`.
    #[serde(default)]
    weights: Option<String>,
    /// A sentence: `cheapest`, `schnell und zuverlaessig`, `direct`.
    ///
    /// Resolved by a deterministic vocabulary rather than a model, and refused
    /// with that vocabulary when nothing matches. A silent fallback to the
    /// profile's weights would look exactly like the sentence being understood.
    #[serde(default)]
    phrase: Option<String>,
}

impl RouteQuery {
    /// The fare options, with the traveller's own cards filling what the caller
    /// left out.
    ///
    /// An explicit parameter always wins: a caller naming a BahnCard is asking a
    /// question about that card, not about the profile. Absent both, this is
    /// exactly what it was before the profile existed — which is why every fare
    /// priced until now was a second-class single-adult fare with no discount.
    fn fare(
        &self,
        snapshot: Option<&transit::ranking::ProfileSnapshot>,
    ) -> transit::hafas::FareOptions {
        let cards = snapshot.cloned().unwrap_or_default();
        transit::hafas::FareOptions {
            bahncard: self.bc.or_else(|| cards.bahncard()),
            first_class: self.first_class,
            deutschland_ticket: self.d_ticket || cards.holds_deutschlandticket(),
        }
    }

    /// The origin: the query's, else the first home station.
    fn origin(&self, snapshot: Option<&transit::ranking::ProfileSnapshot>) -> Option<String> {
        self.from
            .clone()
            .or_else(|| snapshot.and_then(|snapshot| snapshot.home_stations.first().cloned()))
    }
}

/// Every capability server answers a failure as `{"error": "..."}` -- the dashboard's
/// client unwraps exactly that field, and without it a reader saw the raw JSON of a
/// failure, or worse, a bare sentence that is not JSON at all. These handlers returned
/// `e.to_string()` and were the one surface in the repo still breaking that contract.
fn fail(
    status: axum::http::StatusCode,
    message: impl std::fmt::Display,
) -> (axum::http::StatusCode, String) {
    (status, json!({ "error": message.to_string() }).to_string())
}

fn hafas_fail(e: transit::hafas::HafasError) -> (axum::http::StatusCode, String) {
    // "no cheaper split exists" is a result. Answering 500 made the absence of a bargain
    // look like a broken server, and the dashboard had to render it as one.
    //
    // A malformed `time` is the caller's, and relaying it was the worst answer this
    // server gave: bahn.de refuses a timestamp without seconds with an empty-bodied 500,
    // which came back out of here as `{"error":"HAFAS query failed with status 500: "}`
    // -- a sentence naming neither the caller's mistake nor a real fault, and read as the
    // upstream blocking us for an evening. 400, and the message names the format.
    let status = match e {
        transit::hafas::HafasError::NoSplitFound => axum::http::StatusCode::NOT_FOUND,
        transit::hafas::HafasError::InvalidDatetime(_) => axum::http::StatusCode::BAD_REQUEST,
        _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    };
    fail(status, e)
}

#[derive(Clone)]
struct AppState {
    hafas_client: Arc<HafasClient>,
    config: Arc<Config>,
}

async fn handle_suggest(
    State(state): State<AppState>,
    Query(params): Query<SuggestQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let client = state.hafas_client;
    match tokio::task::spawn_blocking(move || client.suggest_stations(&params.q)).await {
        Ok(Ok(stations)) => Ok(Json(serde_json::to_value(stations).unwrap_or_default())),
        Ok(Err(e)) => Err(hafas_fail(e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// An EVA id, or a name resolved to one.
///
/// HAFAS takes an EVA id and returns **nothing at all** for a name — an empty
/// journey list with HTTP 200, which is indistinguishable from "no trains on that
/// day". A confident empty answer for a well-formed query is the worst failure an
/// API has, and it is why a station name is resolved through the same suggest
/// surface the UI uses rather than passed through.
///
/// An all-digit input is taken as an EVA and not looked up: that is what the
/// caller asked for, and a suggest on `8000207` would be a round trip to be told
/// what was already given.
///
/// A name that resolves to nothing is a 400 naming it, never an empty answer.
fn resolve_station(
    client: &transit::hafas::HafasClient,
    input: &str,
) -> Result<String, (axum::http::StatusCode, String)> {
    let trimmed = input.trim();
    if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }
    match client.suggest_stations(trimmed) {
        // Every hit is examined, not just the first: a fuzzy top hit must not hide
        // a correct second one.
        Ok(stations) => stations
            .iter()
            .find(|station| matches_query(trimmed, &station.name))
            .map(|station| station.id.clone())
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    format!("no station matches {trimmed:?}; pass an EVA id to skip resolution"),
                )
            }),
        Err(error) => Err(hafas_fail(error)),
    }
}

/// Whether a suggest hit actually answers what was asked.
///
/// HAFAS's suggest is very fuzzy. Measured 2026-09-23: `Nowhere At All` returns
/// `Hannover Karl-Wiechert-Allee` as its first hit, because `Alle` matches
/// `Allee`, and a search built on it goes to the wrong city with HTTP 200. A wrong
/// destination the caller cannot detect is worse than an empty answer, so a hit is
/// accepted only when every significant word of the query appears in the station
/// name — a check anyone can run by eye, and one that fails towards a named 400
/// rather than towards a plausible guess.
///
/// Deliberately strict, and the cost is stated: a query spelled without an umlaut
/// (`Munchen`) will not match `München` and is refused. That is recoverable and
/// visible; the other direction is neither.
fn matches_query(query: &str, station_name: &str) -> bool {
    let name = station_name.to_lowercase();
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    !words.is_empty() && words.iter().all(|word| name.contains(word.as_str()))
}

async fn handle_search(
    State(state): State<AppState>,
    Query(params): Query<RouteQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    // Resolved before the blocking task because it is pure, and because a bad
    // weight set is a caller error that should not cost a search.
    let override_weights = transit::ranking::resolve_weights(
        params.priority.as_deref(),
        params.weights.as_deref(),
        params.phrase.as_deref(),
    )
    .map_err(|reason| (axum::http::StatusCode::BAD_REQUEST, reason))?;

    let client = state.hafas_client;
    // Everything blocking happens inside one task: the search, the punctuality
    // enrichment and the single profile read, which is itself a blocking HTTP
    // call. `punctuality::enrich` leaves the journeys untouched when the
    // statistics service is unreachable and the ranking does nothing when
    // nothing has been stated, so a search never fails for want of either.
    let outcome = tokio::task::spawn_blocking(
        move || -> Result<Vec<transit::travel::Journey>, (axum::http::StatusCode, String)> {
            let snapshot = transit::ranking::read_profile();
            let Some(from) = params.origin(snapshot.as_ref()) else {
                return Err((
                    axum::http::StatusCode::BAD_REQUEST,
                    "no origin: pass from=, or set hard.home_stations on the profile".to_string(),
                ));
            };
            let fare = params.fare(snapshot.as_ref());
            let origin = resolve_station(&client, &from)?;
            let destination = resolve_station(&client, &params.to)?;
            let mut journeys = client
                .search_connections(&origin, &destination, &params.time, &fare)
                .map_err(hafas_fail)?;
            transit::punctuality::enrich(&mut journeys);
            // After enrichment, because reliability is one of the ranking's
            // factors and the only one that is not relative to the rest of the
            // set.
            transit::ranking::rank_journeys(&mut journeys, override_weights);
            Ok(journeys)
        },
    )
    .await;

    match outcome {
        Ok(Ok(journeys)) => Ok(Json(serde_json::to_value(journeys).unwrap_or_default())),
        Ok(Err(failure)) => Err(failure),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn handle_split(
    State(state): State<AppState>,
    Query(params): Query<RouteQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let client = state.hafas_client;
    // Same two profile facts as a plain search, and no ranking: a split chain is
    // one answer, not a set to order.
    let outcome = tokio::task::spawn_blocking(
        move || -> Result<transit::travel::SplitResult, (axum::http::StatusCode, String)> {
            let snapshot = transit::ranking::read_profile();
            let Some(from) = params.origin(snapshot.as_ref()) else {
                return Err((
                    axum::http::StatusCode::BAD_REQUEST,
                    "no origin: pass from=, or set hard.home_stations on the profile".to_string(),
                ));
            };
            let fare = params.fare(snapshot.as_ref());
            let origin = resolve_station(&client, &from)?;
            let destination = resolve_station(&client, &params.to)?;
            let mut result = client
                .search_split_tickets(&origin, &destination, &params.time, &fare)
                .map_err(hafas_fail)?;
            transit::punctuality::enrich_split(&mut result);
            Ok(result)
        },
    )
    .await;

    match outcome {
        Ok(Ok(result)) => Ok(Json(serde_json::to_value(result).unwrap_or_default())),
        Ok(Err(failure)) => Err(failure),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn handle_health() -> Json<Value> {
    // rusqlite is a blocking API over a file: a probe on an async worker holds
    // that thread for the open and, under a busy writer, for busy_timeout as
    // well. spawn_blocking, same as handle_list_trips.
    let store_status = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        if TransitStore::open(&cfg.database_path).is_ok() {
            "ok".to_string()
        } else {
            "offline".to_string()
        }
    })
    .await
    .unwrap_or_else(|_| "offline".to_string());

    Json(json!({
        "status": "ok",
        "service": "transit",
        "version": env!("CARGO_PKG_VERSION"),
        "store": store_status,
    }))
}

#[derive(Deserialize)]
struct ExtractQuery {
    /// Only the extension is read, to choose the reader. A ticket's own filename
    /// is a personal fact and is echoed back rather than stored anywhere.
    file_name: String,
}

/// Parses a ticket confirmation and returns the parse. Stores nothing.
///
/// `transit import <file>` has done this since the port, printed the JSON and
/// forgotten it, so the parser had no caller but a human at a terminal. This is
/// the same function behind HTTP.
///
/// It deliberately does not write a booking record. `extractor.rs` emits one leg
/// per train number, all sharing origin, destination and times, assigns dates
/// positionally, takes the first price match rather than the total, and falls
/// back to `<year>-01-01` when no date parses. Behind a human reviewing every
/// field that is fine. Behind a scanner it writes wrong itineraries confidently,
/// which is the failure this endpoint must not enable.
async fn handle_extract_ticket(
    Query(params): Query<ExtractQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    if body.is_empty() {
        return Err(fail(
            axum::http::StatusCode::BAD_REQUEST,
            "request body is the ticket file's bytes, and it is empty",
        ));
    }
    match transit::extractor::extract_from_bytes(&body, &params.file_name) {
        Ok(ticket) => Ok(Json(serde_json::to_value(ticket).unwrap_or_default())),
        // A file this parser cannot read is the caller's input, not a server
        // fault: an image, or a format with no reader. 400 tells it to send
        // something else rather than to retry the same bytes.
        Err(e) => Err(fail(axum::http::StatusCode::BAD_REQUEST, e)),
    }
}

/// One stored trip with its legs, as JSON.
///
/// Written out here rather than derived, because `TripRow`/`TripLegRow` are the
/// store's row shapes and deriving `Serialize` on them would make every column
/// rename a breaking API change. The CLI's `session_summary` deliberately serves
/// a flattened version of the same rows: a human choosing between fares wants
/// price and duration, while the caller of this endpoint wants the legs it would
/// otherwise have to query Postgres directly to see.
fn trip_json(t: &transit::store::TripRow, legs: &[transit::store::TripLegRow]) -> Value {
    json!({
        "trip_id": t.id,
        "status": t.status,
        "origin_eva": t.origin_eva,
        "destination_eva": t.destination_eva,
        "trigger_reason": t.trigger_reason,
        "total_duration_minutes": t.total_duration_minutes,
        "total_price": t.total_price,
        "created_at": t.created_at,
        "session_id": t.session_id,
        // When the fare was last seen, so a reader can tell a ten-week-old price
        // from a fresh one. Null means unknown, never recent.
        "priced_at": t.priced_at,
        "legs": legs.iter().map(|l| json!({
            "origin_eva": l.origin_eva,
            "origin_name": l.origin_name,
            "destination_eva": l.destination_eva,
            "destination_name": l.destination_name,
            "departure_time": l.departure_time,
            "arrival_time": l.arrival_time,
            "train_name": l.train_name,
            "train_number": l.train_number,
            "train_category": l.train_category,
            "platform": l.platform,
            "is_regional": l.is_regional,
        })).collect::<Vec<Value>>(),
    })
}

/// A bounded read says what it left behind. `count` is every trip matching the
/// filter, `returned` is how many came back, and `truncated` says whether those
/// two disagree -- borrowed from knowledge-graph's `/api/graph/unit`, for the
/// same reason: a capped answer that looked complete would read as the whole set.
#[derive(Deserialize)]
struct TripsQuery {
    session_id: Option<String>,
    limit: Option<i64>,
}

/// How many trips one unfiltered read returns before it starts saying `truncated`.
const TRIPS_DEFAULT_LIMIT: i64 = 100;
/// The ceiling a caller can raise `limit` to. A trip carries its full leg set, so
/// an unbounded read is a response size nobody asked for.
const TRIPS_MAX_LIMIT: i64 = 500;

async fn handle_list_trips(
    State(state): State<AppState>,
    Query(params): Query<TripsQuery>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let database_path = state.config.database_path.clone();
    // Clamped rather than rejected: a caller asking for more than the ceiling wants
    // as much as it can get, and `truncated` already tells it what it did not get.
    let limit = params
        .limit
        .unwrap_or(TRIPS_DEFAULT_LIMIT)
        .clamp(1, TRIPS_MAX_LIMIT);
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = TransitStore::open(&database_path).map_err(|e| e.to_string())?;
        let session_id = params.session_id.as_deref();
        let count = store.count_trips(session_id).map_err(|e| e.to_string())?;
        let trips = store
            .list_trips(session_id, Some(limit))
            .map_err(|e| e.to_string())?;
        let rendered: Vec<Value> = trips.iter().map(|(t, legs)| trip_json(t, legs)).collect();
        Ok(json!({
            "count": count,
            "returned": rendered.len(),
            "truncated": count > rendered.len() as i64,
            "session_id": params.session_id,
            "trips": rendered,
        }))
    })
    .await
    {
        Ok(Ok(val)) => Ok(Json(val)),
        Ok(Err(e)) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
        Err(e) => Err(fail(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

// No /discover or /pulse/* proxy routes here — see
// dashboard/README.md. transit-server
// serves transit's own API only; aggregating scouting/pulse behind one origin
// is the dashboard's concern (or a dedicated gateway), not something transit
// hardcodes another capability's port to do. Re-add a proxy only when the
// dashboard names a concrete need it can't solve with a multi-target dev
// proxy / reverse proxy on its own side.

#[tokio::main]
async fn main() {
    let config = Arc::new(Config::load());
    // HafasClient wraps reqwest::blocking::Client, which spins up its own
    // background tokio runtime internally. Constructing it directly inside
    // #[tokio::main]'s async context panics on drop ("cannot drop a runtime
    // in a context where blocking is not allowed") -- spawn_blocking moves
    // the construction off the async runtime thread.
    let hafas_client = Arc::new(
        tokio::task::spawn_blocking(HafasClient::new)
            .await
            .expect("hafas client construction panicked"),
    );

    let state = AppState {
        hafas_client,
        config,
    };

    // Port contract and loopback bind live in sjel_server; the old 0.0.0.0 bind
    // here was never a documented decision and is retired with it.
    let port = sjel_server::resolve_port(Some("TRANSIT_PORT"), None, 3000);
    sjel_server::serve_local("transit", port, build_router(state)).await;
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_TRANSIT_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "transit";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// Two surfaces make this more than hygiene. `GET /api/trips` returns the
/// operator's saved trip searches with their legs — where they went, when, and
/// what it cost — which `CorsLayer::permissive()` made readable by any page open
/// in their browser. `POST /api/tickets/extract` takes raw file bytes and hands
/// them to `pdf_extract`, so the same permissive layer let a hostile page choose
/// the bytes a PDF parser runs on; that is the input class `osv-scanner.toml`'s
/// RUSTSEC-2026-0192 entry is about, and the entry's reason assumed those bytes
/// were the operator's.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`), so a route appended under it would silently
/// lose the refusal.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(handle_health))
        .route("/api/health", get(handle_health))
        .route("/api/suggest", get(handle_suggest))
        .route("/api/search", get(handle_search))
        .route("/api/split", get(handle_split))
        .route("/api/trips", get(handle_list_trips))
        .route("/api/tickets/extract", post(handle_extract_ticket))
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
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

/// The router-level proof that `libs/sjel-server`'s predicate tests cannot give:
/// a route registered BELOW the `.layer()` call passes every test of
/// `origin_allowed_by` and still answers a hostile page.
///
/// `tower::ServiceExt::oneshot` rather than a loopback listener, so the test
/// needs no port and no HTTP client.
///
/// No route here reads the store. `/api/trips` would open the deployment's
/// SQLite file, and the guard answers it before the handler runs, so the
/// refusal is asserted on that path and the control is asserted on
/// `/api/tickets/extract`, whose empty-body 400 comes from the handler and
/// touches nothing.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// The HafasClient is built off the runtime for the reason `main` states:
    /// `reqwest::blocking::Client` builds a runtime, and building one inside an
    /// async context panics on drop.
    async fn router() -> Router {
        let hafas_client = Arc::new(
            tokio::task::spawn_blocking(HafasClient::new)
                .await
                .expect("hafas client construction panicked"),
        );
        build_router(AppState {
            hafas_client,
            config: Arc::new(tokio::task::spawn_blocking(Config::load).await.unwrap()),
        })
    }

    async fn respond(method: &str, path: &str, origin: Option<&str>) -> (StatusCode, String) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = router()
            .await
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("a body");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    async fn answer(method: &str, path: &str, origin: Option<&str>) -> StatusCode {
        respond(method, path, origin).await.0
    }

    #[tokio::test]
    async fn a_foreign_origin_reaches_neither_the_saved_trips_nor_the_pdf_parser() {
        for (method, path) in [("GET", "/api/trips"), ("POST", "/api/tickets/extract")] {
            assert_eq!(
                answer(method, path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{method} {path} answered a foreign origin — it is registered below the guard layer"
            );
        }
    }

    /// The other half, with the query string the route requires.
    ///
    /// Without `?file_name=`, this asked for 400 and got the `Query` extractor's
    /// rejection — "Failed to deserialize query string: missing field
    /// `file_name`" — which is a rejection before the handler, not the handler's
    /// answer. It still told a refusal from an admission, because the guard
    /// answers 403, but it was not the thing the comment claimed to have proved.
    /// With the file name supplied, the 400 is `handle_extract_ticket`'s own
    /// sentence about an empty body, and the body is asserted so the control
    /// cannot drift back to an extractor rejection unnoticed.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            let (status, body) =
                respond("POST", "/api/tickets/extract?file_name=ticket.pdf", origin).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "the guard refused a caller it must admit: {origin:?}"
            );
            assert!(
                body.contains("the ticket file's bytes"),
                "400 came from somewhere other than the handler: {body}"
            );
        }
    }
}

#[cfg(test)]
mod station_resolution_tests {
    use super::matches_query;

    #[test]
    fn a_real_name_matches_its_station() {
        assert!(matches_query("Berlin Hbf", "Berlin Hbf"));
        assert!(matches_query("berlin hbf", "Berlin Hbf"));
        assert!(matches_query("Frankfurt(Main)Hbf", "Frankfurt(Main)Hbf"));
        assert!(matches_query("Köln Hbf", "Köln Hbf"));
    }

    #[test]
    fn the_fuzzy_hit_that_motivated_this_is_refused() {
        // Measured 2026-09-23: HAFAS returns this as the FIRST hit for that query,
        // because "Alle" matches "Allee". Taking it sends the search to the wrong
        // city with HTTP 200, which the caller cannot detect.
        assert!(!matches_query(
            "Nowhere At All",
            "Hannover Karl-Wiechert-Allee"
        ));
        assert!(!matches_query(
            "Nowhere At All",
            "Therese-Giehse-Allee, München"
        ));
        assert!(!matches_query(
            "Nowhere At All",
            "Großbeeren Märkische Allee Süd"
        ));
    }

    #[test]
    fn every_significant_word_must_appear() {
        assert!(!matches_query("Berlin Hbf", "Berlin Südkreuz"));
        assert!(!matches_query("Berlin Gesundbrunnen", "Berlin Hbf"));
        assert!(matches_query("Berlin", "Berlin Hbf"));
    }

    #[test]
    fn a_query_with_no_words_matches_nothing() {
        assert!(!matches_query("", "Berlin Hbf"));
        assert!(!matches_query("   ", "Berlin Hbf"));
        assert!(!matches_query("...", "Berlin Hbf"));
    }

    #[test]
    fn the_strictness_is_stated_rather_than_hidden() {
        // A query without an umlaut is refused. Recoverable and visible, which is
        // the direction this check is built to fail in.
        assert!(!matches_query("Munchen", "München Hbf"));
        assert!(matches_query("München", "München Hbf"));
    }
}

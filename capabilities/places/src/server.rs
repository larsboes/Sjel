//! places HTTP surface (README "HTTP surface", port 8093). Same shape as
//! finance's server: blocking store work in `spawn_blocking`, `/ready` proves
//! the database, and `GET /routes` serves the manifest the coverage test below
//! checks against this file's own source.
//!
//! One deliberate departure from the sibling servers: no permissive CORS.
//! They guard no C2 table; this one serves the companion register (README D4),
//! so browser cross-origin access is refused instead — see `origin_allowed`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use places::config::Config;
use places::geocode::{GeocodeQuery, Geocoder, StructuredQuery};
use places::store::{stable_id, PlacesStore, Review, ReviewOutcome, PRESENCE_RADIUS_KM};
use places::{layers, today};

const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/places",
        "List/search the place registry. Optional ?q= substring and ?kind= venue|city|station|address|region. \
         A shared registry: it holds no opinion about whether anyone liked a place.",
    ),
    r(
        "GET",
        "/api/visits",
        "Every recorded visit: the operator was at this place, then, and rated it so. Newest \
         first, joined to the registry so a rating arrives with a name and a coordinate. \
         Personal records in their own table -- a star rating is not a property of a place, \
         which is exactly why it is not a column on one. Empty until a Takeout review export \
         has been imported (`places backfill takeout`).",
    ),
    r(
        "POST",
        "/api/geocode",
        "Cached forward geocode. Body: { query } or { structured: { street, postalcode, city, country } }. Place text only; a repeated query never leaves the host.",
    ),
    r(
        "GET",
        "/api/layers/spend",
        "Venue features, city aggregates and a ranked summary over location-linked finance transactions. GeoJSON, cents, EUR implied.",
    ),
    r(
        "GET",
        "/api/layers/travel",
        "Trip destinations with past/upcoming phase, transit legs as LineStrings, station points and spend-presence evidence. GeoJSON.",
    ),
    r(
        "GET",
        "/api/layers/people",
        "Confirmed, currently-valid companion-register rows. GeoJSON.",
    ),
    r(
        "GET",
        "/api/unplaced",
        "Expense transactions with no place link, grouped by exact description, ranked by total. Cents, EUR implied, capped at 200 groups.",
    ),
    r(
        "POST",
        "/api/unplaced/assign",
        "Link every unlinked transaction whose description matches exactly to one place. Body: { description, place_id | geocode_query, precision: venue|city }. A city-kind place is linked at city precision whatever was requested (D1). Writes source=manual links.",
    ),
    r(
        "GET",
        "/api/people/proposals",
        "Proposed register rows awaiting human review.",
    ),
    r(
        "POST",
        "/api/people/places",
        "Propose where a person is, as the operator states it. Body: { person, city, from?, to? } \
         (dates YYYY-MM-DD). Only the city text is geocoded. Writes a PROPOSED row, source=operator; \
         confirming it is still the confirm route's job.",
    ),
    r(
        "POST",
        "/api/people/proposals/:id/confirm",
        "Confirm one register proposal. The only path that produces state=confirmed.",
    ),
    r(
        "POST",
        "/api/people/proposals/:id/dismiss",
        "Dismiss one register proposal.",
    ),
    r(
        "GET",
        "/api/people/presence",
        "How many known companions are near a coordinate in a window, and for how many days. \
         Query: latitude, longitude, from, to (YYYY-MM-DD, from <= to) — all four required. \
         There is NO radius parameter: places owns it (50 km) and echoes it. Confirmed rows \
         only, and the reply carries no person, no place name, no row id and no confidence.",
    ),
    r(
        "GET",
        "/api/places/:id/climate",
        "Twelve months of climate normals for one registered place, folded from ten complete calendar years. Optional ?from=YYYY-MM-DD&to=YYYY-MM-DD marks the months a plan window covers. Empty months with fetched_at null means the place has none yet — run `places-server climate fetch`.",
    ),
    r(
        "GET",
        "/api/climate",
        "Climate normals for up to 8 places at once. Exactly one of ?place_ids=<id>,<id> or ?at=<lat>,<lon>;<lat>,<lon> (semicolon between pairs, comma inside a pair — NOT a repeated key). Optional ?from=&to=. One result per requested key, in request order.",
    ),
];

const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("places", ROUTES))
}

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
}

type ApiResponse = (StatusCode, Json<Value>);

fn respond(status: StatusCode, value: Value) -> ApiResponse {
    (status, Json(value))
}

fn failed(error: String) -> ApiResponse {
    respond(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "ok": false, "capability": "places", "error": error }),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "capability": "places" }))
}

async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => respond(
            StatusCode::OK,
            json!({ "ok": true, "capability": "places" }),
        ),
        Ok(Err(error)) => respond(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "places", "error": error }),
        ),
        Err(_) => respond(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "places", "error": "readiness check failed" }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct PlacesQuery {
    q: Option<String>,
    kind: Option<String>,
}

/// `GET /api/visits`.
///
/// Answers with an empty list rather than a 404 on a registry that has never seen
/// a review export: "no visits recorded" is a fact, and the alternative makes a
/// caller distinguish two states that mean the same thing to it.
async fn list_visits(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.visits())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(visits)) => (
            StatusCode::OK,
            Json(json!({
                "count": visits.len(),
                "visits": visits,
            })),
        ),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(format!("store task failed: {error}")),
    }
}

async fn list_places(
    State(state): State<AppState>,
    Query(query): Query<PlacesQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.search_places(query.q.as_deref(), query.kind.as_deref()))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(found)) => {
            let rendered: Vec<Value> = found
                .iter()
                .map(|place| {
                    json!({
                        "id": place.id,
                        "name": place.name,
                        "kind": place.kind,
                        "city": place.city,
                        "country_code": place.country_code,
                        "latitude": place.latitude,
                        "longitude": place.longitude,
                        "source": place.source,
                        "external_ref": place.external_ref,
                    })
                })
                .collect();
            respond(StatusCode::OK, json!({ "places": rendered }))
        }
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct GeocodeRequest {
    query: Option<String>,
    structured: Option<StructuredQuery>,
}

async fn geocode(
    State(state): State<AppState>,
    Json(request): Json<GeocodeRequest>,
) -> ApiResponse {
    let query = match (request.query, request.structured) {
        (Some(free), None) => GeocodeQuery::Free(free),
        (None, Some(structured)) => GeocodeQuery::Structured(structured),
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of query or structured" }),
            )
        }
    };
    // Emptiness is checked here, for both variants, so a blank query is the
    // client's 400 and never surfaces as the geocoder's own error via 500.
    if query.is_empty() {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "geocode query must not be empty" }),
        );
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let geocoder = Geocoder::new(&store);
        geocoder
            .geocode(&query, None, &now)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(outcome)) => {
            let place = outcome.place.map(|place| {
                json!({
                    "place_id": place.id,
                    "name": place.name,
                    // The registry kind, so a client can refuse venue
                    // precision for a city-kind result (README D1) — the
                    // dashboard's "Pin venue" guard reads exactly this.
                    "kind": place.kind,
                    "latitude": place.latitude,
                    "longitude": place.longitude,
                    "city": place.city,
                    "country_code": place.country_code,
                })
            });
            respond(
                StatusCode::OK,
                json!({
                    "status": if outcome.found { "ok" } else { "not_found" },
                    "cached": outcome.cached,
                    "place": place,
                }),
            )
        }
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

// ─── Climate normals (README D5, ISA F4) ─────────────────────────────────────

/// Optional plan window. Only the month numbers it covers are used, so a
/// request cannot narrow the normals themselves — a normal is the whole month
/// or it is nothing.
#[derive(Debug, Deserialize)]
struct ClimateWindow {
    from: Option<String>,
    to: Option<String>,
}

/// The month numbers a `from`..`to` window touches, capped at twelve. A window
/// longer than a year covers every month, which is the honest answer rather than
/// an error.
fn window_months(from: Option<&str>, to: Option<&str>) -> Vec<u32> {
    let index = |value: &str| -> Option<i64> {
        let year: i64 = value.get(..4)?.parse().ok()?;
        let month: i64 = value.get(5..7)?.parse().ok()?;
        (1..=12).contains(&month).then_some(year * 12 + month - 1)
    };
    let (Some(first), Some(last)) = (from.and_then(index), to.and_then(index)) else {
        return Vec::new();
    };
    if last < first {
        return Vec::new();
    }
    (first..=last.min(first + 11))
        .map(|slot| (slot.rem_euclid(12) + 1) as u32)
        .collect()
}

/// One place's stored normals as the wire carries them. `months: []` with
/// `fetched_at: null` is the never-fetched state, which the UI turns into "run
/// the fetch verb" rather than into an empty grid.
fn climate_body(store: &PlacesStore, place_id: &str, in_window: &[u32]) -> Result<Value, String> {
    let months = store.climate_get(place_id).map_err(|e| e.to_string())?;
    let meta = store.climate_meta(place_id).map_err(|e| e.to_string())?;
    let best = places::climate::best_months(&months);
    let rendered: Vec<Value> = months
        .iter()
        .map(|month| {
            json!({
                "month": month.month,
                "t_max_mean": month.t_max_mean,
                "t_min_mean": month.t_min_mean,
                "rain_days_mean": month.rain_days_mean,
                "precipitation_mm_mean": month.precipitation_mm_mean,
                "daylight_hours_mean": month.daylight_hours_mean,
                "sunshine_hours_mean": month.sunshine_hours_mean,
                "days_observed": month.days_observed,
                "best_month": best.contains(&month.month),
                "in_window": in_window.contains(&month.month),
            })
        })
        .collect();
    Ok(json!({
        "source": meta.as_ref().map(|meta| meta.source.clone()),
        "period": meta.as_ref().map(|meta| json!({
            "start": meta.period_start,
            "end": meta.period_end,
            "years": meta.years_covered,
        })),
        "fetched_at": meta.as_ref().map(|meta| meta.fetched_at.clone()),
        "months": rendered,
    }))
}

fn place_summary(place: &places::store::Place, distance_km: Option<f64>) -> Value {
    json!({
        "id": place.id,
        "name": place.name,
        "kind": place.kind,
        "latitude": place.latitude,
        "longitude": place.longitude,
        "distance_km": distance_km,
    })
}

async fn place_climate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(window): Query<ClimateWindow>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let in_window = window_months(window.from.as_deref(), window.to.as_deref());
    match tokio::task::spawn_blocking(move || -> Result<Option<Value>, String> {
        let store = PlacesStore::open(&database_path).map_err(|e| e.to_string())?;
        let Some(place) = store.place(&id).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let mut body = climate_body(&store, &place.id, &in_window)?;
        let object = body.as_object_mut().expect("climate_body builds an object");
        object.insert("place".into(), place_summary(&place, None));
        object.insert(
            "best_months_rule".into(),
            json!(places::climate::BEST_MONTHS_RULE),
        );
        object.insert("attribution".into(), json!(places::climate::ATTRIBUTION));
        Ok(Some(body))
    })
    .await
    {
        Ok(Ok(Some(body))) => respond(StatusCode::OK, body),
        Ok(Ok(None)) => respond(StatusCode::NOT_FOUND, json!({ "error": "no such place" })),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct ClimateBatch {
    place_ids: Option<String>,
    /// Semicolon between pairs, comma inside a pair. NOT a repeated `at=` key:
    /// axum's `Query` deserializes through serde_urlencoded, which cannot fill a
    /// sequence from repeated keys, so `?at=..&at=..` would answer 400 for every
    /// multi-destination request. Both delimiters are safe because every value
    /// here is a number, and comma-splitting is this file's own precedent.
    at: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

/// At most eight keys per request. A bound rather than a limit anybody will hit:
/// a plan has a handful of destinations, and an unbounded batch would turn one
/// request into an unbounded scan of the registry per key.
const MAX_CLIMATE_KEYS: usize = 8;

async fn climate(State(state): State<AppState>, Query(query): Query<ClimateBatch>) -> ApiResponse {
    let selectors = (query.place_ids.as_deref(), query.at.as_deref());
    let keys: Vec<String> = match selectors {
        (Some(ids), None) => ids.split(',').map(|id| id.trim().to_string()).collect(),
        (None, Some(at)) => at.split(';').map(|pair| pair.trim().to_string()).collect(),
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of place_ids=<id>,<id> or at=<lat>,<lon>;<lat>,<lon>" }),
            )
        }
    };
    let keys: Vec<String> = keys.into_iter().filter(|key| !key.is_empty()).collect();
    if keys.is_empty() || keys.len() > MAX_CLIMATE_KEYS {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("send 1 to {MAX_CLIMATE_KEYS} keys") }),
        );
    }
    let by_id = query.place_ids.is_some();
    let database_path = state.database_path.clone();
    let in_window = window_months(query.from.as_deref(), query.to.as_deref());

    match tokio::task::spawn_blocking(move || -> Result<Vec<Value>, String> {
        let store = PlacesStore::open(&database_path).map_err(|e| e.to_string())?;
        // Loaded once for the whole batch, not once per key.
        let registry = if by_id {
            Vec::new()
        } else {
            store.places_with_climate().map_err(|e| e.to_string())?
        };
        let mut results = Vec::with_capacity(keys.len());
        for key in &keys {
            // Err carries the reason this key has no place, and every reason is
            // about what the caller actually sent: an unparsable pair is told so
            // rather than being told the registry is empty near a coordinate it
            // never sent.
            let resolution: Result<(&str, places::store::Place, Option<f64>), String> = if by_id {
                match store.place(key).map_err(|e| e.to_string())? {
                    Some(place) => Ok(("id", place, None)),
                    None => Err("no place with that id".to_string()),
                }
            } else {
                match parse_pair(key) {
                    None => Err("not a lat,lon pair".to_string()),
                    Some((latitude, longitude)) => {
                        match places::climate::resolve_at(&registry, latitude, longitude) {
                            places::climate::Resolution::Registry { place, distance_km } => {
                                Ok(("registry", place, Some(distance_km)))
                            }
                            places::climate::Resolution::Nearest { place, distance_km } => {
                                Ok(("nearest", place, Some(distance_km)))
                            }
                            // The refusal is built where the radius is, so the
                            // sentence and the constant cannot drift.
                            places::climate::Resolution::Unmatched { reason } => Err(reason),
                        }
                    }
                }
            };
            let entry = match resolution {
                Ok((resolved_by, place, distance_km)) => {
                    let mut body = climate_body(&store, &place.id, &in_window)?;
                    let object = body.as_object_mut().expect("an object");
                    object.insert("key".into(), json!(key));
                    object.insert("resolved_by".into(), json!(resolved_by));
                    object.insert("matched_place".into(), place_summary(&place, distance_km));
                    object.insert("reason".into(), Value::Null);
                    body
                }
                Err(reason) => json!({
                    "key": key,
                    "resolved_by": Value::Null,
                    "matched_place": Value::Null,
                    "reason": reason,
                    "source": Value::Null,
                    "period": Value::Null,
                    "fetched_at": Value::Null,
                    "months": [],
                }),
            };
            results.push(entry);
        }
        Ok(results)
    })
    .await
    {
        Ok(Ok(results)) => respond(
            StatusCode::OK,
            json!({
                "best_months_rule": places::climate::BEST_MONTHS_RULE,
                "attribution": places::climate::ATTRIBUTION,
                "results": results,
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

fn parse_pair(pair: &str) -> Option<(f64, f64)> {
    let (latitude, longitude) = pair.split_once(',')?;
    Some((
        latitude.trim().parse().ok()?,
        longitude.trim().parse().ok()?,
    ))
}

async fn spend_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::spend_layer(&store))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn travel_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::travel_layer(&store, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn people_layer(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::people_layer(&store, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn unplaced(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| layers::unplaced_groups(&store))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn assign_unplaced(
    State(state): State<AppState>,
    Json(request): Json<layers::AssignUnplaced>,
) -> ApiResponse {
    // Bad bodies are the client's 400 before any store work, mirroring the
    // geocode handler's emptiness rule.
    if request.description.trim().is_empty() {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "description must not be empty" }),
        );
    }
    let by_place_id = match (&request.place_id, &request.geocode_query) {
        (Some(_), None) => true,
        (None, Some(query)) if !query.trim().is_empty() => false,
        (None, Some(_)) => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "geocode_query must not be empty" }),
            )
        }
        _ => {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "send exactly one of place_id or geocode_query" }),
            )
        }
    };
    if !matches!(request.precision.as_str(), "venue" | "city") {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "precision must be venue or city" }),
        );
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let geocoder = Geocoder::new(&store);
        layers::assign_unplaced(&store, &geocoder, "finance", &request, &now)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(body))) => respond(StatusCode::OK, body),
        Ok(Ok(None)) => respond(
            StatusCode::NOT_FOUND,
            json!({
                "error": if by_place_id {
                    "no place with that id"
                } else {
                    "the geocode query resolved to no place"
                }
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_proposals(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = PlacesStore::open(&database_path).map_err(|error| error.to_string())?;
        let rows = store
            .person_places_in_state("proposed")
            .map_err(|error| error.to_string())?;
        let mut proposals = Vec::with_capacity(rows.len());
        for row in rows {
            let place = store
                .place(&row.place_id)
                .map_err(|error| error.to_string())?;
            proposals.push(json!({
                "id": row.id,
                "person": row.person,
                "place_name": place.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                "city": place.as_ref().and_then(|p| p.city.clone()),
                "latitude": place.as_ref().and_then(|p| p.latitude),
                "longitude": place.as_ref().and_then(|p| p.longitude),
                "date_start": row.date_start,
                "date_end": row.date_end,
                "confidence_bp": i64::from(row.confidence_bp),
                "source": row.source,
                "state": "proposed",
            }));
        }
        Ok(json!({ "proposals": proposals }))
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::OK, body),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

/// The explicit human review path — with `dismiss` below, the ONLY code that
/// can move a register row to `confirmed` (README D4, ISA PLC-7).
#[derive(Deserialize)]
struct StatedPlace {
    person: String,
    city: String,
    from: Option<String>,
    to: Option<String>,
}

/// `YYYY-MM-DD` by shape. The store compares dates as text, so the shape is what matters.
fn is_day(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// Where someone is, as the operator states it: "Ron is in Bonn from 13 to 20 Oct".
///
/// Added 2026-09-25 (PRD Q116): the register had no way in except backfills, and "where
/// is someone right now" is the fact a trip needs most. The operator is the source, so the
/// row carries full confidence, but it is still written `proposed`: PLC-7 keeps the confirm
/// route the one writer of `confirmed`, and the dashboard calls it in the same action.
/// Only the city text reaches the geocoder, never the person's name (README D3).
async fn state_person_place(
    State(state): State<AppState>,
    Json(body): Json<StatedPlace>,
) -> ApiResponse {
    let person = body.person.trim().to_string();
    let city = body.city.trim().to_string();
    if person.is_empty() || city.is_empty() {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "person and city are both required" }),
        );
    }
    let from = body.from.filter(|d| !d.trim().is_empty());
    let to = body.to.filter(|d| !d.trim().is_empty());
    for day in [&from, &to].into_iter().flatten() {
        if !is_day(day) {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("{day:?} is not a YYYY-MM-DD date") }),
            );
        }
    }
    if let (Some(from), Some(to)) = (&from, &to) {
        if from > to {
            return respond(
                StatusCode::BAD_REQUEST,
                json!({ "error": "from must not be after to" }),
            );
        }
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = PlacesStore::open(&database_path).map_err(|e| e.to_string())?;
        let outcome = Geocoder::new(&store)
            .geocode(&GeocodeQuery::Free(city.clone()), Some("city"), &now)
            .map_err(|e| e.to_string())?;
        let Some(place) = outcome.place else {
            return Err(format!("no place found for {city:?}"));
        };
        let id = stable_id(
            "pp",
            &format!(
                "operator:{person}:{}:{}:{}",
                place.id,
                from.as_deref().unwrap_or(""),
                to.as_deref().unwrap_or("")
            ),
        );
        store
            .propose_person_place(
                &id,
                &person,
                &place.id,
                from.as_deref(),
                to.as_deref(),
                10_000,
                "operator",
                &now,
            )
            .map_err(|e| e.to_string())?;
        Ok(json!({ "id": id, "state": "proposed", "place_name": place.name }))
    })
    .await
    {
        Ok(Ok(body)) => respond(StatusCode::CREATED, body),
        Ok(Err(error)) if error.starts_with("no place found") => {
            respond(StatusCode::UNPROCESSABLE_ENTITY, json!({ "error": error }))
        }
        Ok(Err(error)) => respond(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => respond(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn confirm_proposal(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    review(state, id, Review::Confirmed).await
}

async fn dismiss_proposal(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    review(state, id, Review::Dismissed).await
}

async fn review(state: AppState, id: String, decision: Review) -> ApiResponse {
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| store.review_person_place(&id, decision, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(ReviewOutcome::Applied)) => respond(
            StatusCode::OK,
            json!({ "ok": true, "state": decision.as_str() }),
        ),
        Ok(Ok(ReviewOutcome::NoSuchRow)) => respond(
            StatusCode::NOT_FOUND,
            json!({ "error": "no register row with that id" }),
        ),
        // 409, never 404: the row exists, and telling a caller it does not
        // would be a second wrong answer on top of the refused write. The state
        // found is named so the surface can say which one.
        Ok(Ok(ReviewOutcome::Refused { state })) => respond(
            StatusCode::CONFLICT,
            json!({ "error": "that proposal was already reviewed", "state": state }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Deserialize)]
struct PresenceQuery {
    latitude: f64,
    longitude: f64,
    from: String,
    to: String,
}

/// A count and an overlap. See `PlacesStore::confirmed_presence` for the whole
/// argument; the short version is that this is the only shape in which the C2
/// register reaches a planner, and it carries no identity at all.
///
/// Registered ABOVE the `.layer()` call in `build_router`, because axum wraps
/// only the routes added before it.
async fn people_presence(
    State(state): State<AppState>,
    Query(query): Query<PresenceQuery>,
) -> ApiResponse {
    if query.from > query.to {
        return respond(
            StatusCode::BAD_REQUEST,
            json!({ "error": "from must be on or before to" }),
        );
    }
    let database_path = state.database_path.clone();
    let (from, to) = (query.from.clone(), query.to.clone());
    match tokio::task::spawn_blocking(move || {
        PlacesStore::open(&database_path)
            .and_then(|store| {
                store.confirmed_presence(query.latitude, query.longitude, &query.from, &query.to)
            })
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(presence)) => respond(
            StatusCode::OK,
            json!({
                "radius_km": PRESENCE_RADIUS_KM,
                "from": from,
                "to": to,
                "known_companions": presence.known_companions,
                "overlap_days": presence.overlap_days,
            }),
        ),
        Ok(Err(error)) => respond(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

/// Which routes the origin guard covers, and the rule that decides it.
///
/// The predicate and its doc block live in `libs/sjel-server/src/origin.rs`
/// now, because `trips` needs the same refusal for the plan-search body and a
/// second copy of a security predicate is drift. What stays here is the wiring
/// and the reason it is wired this way.
///
/// axum applies `Router::layer` only to routes registered BEFORE it (axum 0.7
/// `src/docs/routing/layer.md`: "Additional routes added after `layer` is
/// called will not have the middleware added"). Every route this capability
/// serves must therefore sit above the `.layer()` call in [`build_router`], and
/// `a_foreign_origin_cannot_read_people_presence` drives the wired router to
/// prove it — a test of the predicate alone passes with the route unguarded.
const CAPABILITY: &str = "places";

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/places", get(list_places))
        .route("/api/visits", get(list_visits))
        .route("/api/geocode", post(geocode))
        .route("/api/layers/spend", get(spend_layer))
        .route("/api/layers/travel", get(travel_layer))
        .route("/api/layers/people", get(people_layer))
        .route("/api/unplaced", get(unplaced))
        .route("/api/unplaced/assign", post(assign_unplaced))
        .route("/api/people/proposals", get(list_proposals))
        .route("/api/people/places", post(state_person_place))
        .route("/api/people/proposals/:id/confirm", post(confirm_proposal))
        .route("/api/people/proposals/:id/dismiss", post(dismiss_proposal))
        .route("/api/people/presence", get(people_presence))
        .route("/api/places/:id/climate", get(place_climate))
        .route("/api/climate", get(climate))
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the C2 guard.
        .layer(middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

pub async fn serve() {
    let config = Config::load();
    let state = AppState {
        database_path: Arc::new(config.database_path),
    };
    sjel_server::serve_local("places-server", config.port, build_router(state)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch database file this process owns. `store::db_tests` has the same
    /// helper, but it is `#[cfg(test)]` inside the library and this file is the
    /// binary, so it cannot be reached from here.
    fn scratch_database(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("places-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The manifest is data a caller reads to learn the surface; a served route
    /// missing from it is invisible. `undeclared_routes` reads this file's own
    /// source, so adding a `.route()` without a manifest entry fails here
    /// (ISA PLC-1, `libs/route-manifest/README.md`).
    #[test]
    fn the_manifest_covers_every_served_route() {
        assert!(
            route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty(),
            "a served route is missing from the manifest"
        );
    }

    /// The defect this test exists for: axum 0.7's `Query` deserializes through
    /// serde_urlencoded, which cannot fill a `Vec` from a repeated key, so a
    /// `?at=..&at=..` extractor would have answered 400 for every
    /// multi-destination request. A pure resolver test cannot fail on a
    /// query-string shape, so the assertion is made here, through the extractor.
    #[tokio::test]
    async fn two_coordinates_in_one_at_parameter_answer_two_results_in_request_order() {
        let path = scratch_database("climate-handler");
        let store = PlacesStore::open(&path).unwrap();
        for (id, name, latitude, longitude) in [
            ("place_first", "First", 52.52, 13.40),
            ("place_second", "Second", 41.90, 12.50),
        ] {
            store
                .upsert_place(
                    &places::store::Place {
                        id: id.into(),
                        name: name.into(),
                        kind: "city".into(),
                        address: None,
                        city: None,
                        country_code: None,
                        latitude: Some(latitude),
                        longitude: Some(longitude),
                        source: "test".into(),
                        external_ref: Some(format!("test:{id}")),
                    },
                    "2026-09-05",
                )
                .unwrap();
        }

        let state = AppState {
            database_path: Arc::new(path),
        };
        let query = ClimateBatch {
            place_ids: None,
            at: Some("52.52,13.40;41.90,12.50".into()),
            from: None,
            to: None,
        };
        let (status, Json(body)) = climate(State(state), Query(query)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let results = body["results"].as_array().expect("results is an array");
        assert_eq!(results.len(), 2, "one result per requested key");
        assert_eq!(results[0]["key"], "52.52,13.40", "request order is kept");
        assert_eq!(results[1]["key"], "41.90,12.50");
        assert_eq!(results[0]["resolved_by"], "registry");
        assert_eq!(results[0]["matched_place"]["id"], "place_first");
        assert_eq!(results[1]["matched_place"]["id"], "place_second");
        // Registered but never fetched: an honest empty, with the stamp null so
        // the UI says "run the fetch verb" instead of drawing an empty grid.
        assert_eq!(results[0]["fetched_at"], Value::Null);
        assert_eq!(results[0]["months"].as_array().unwrap().len(), 0);
    }

    /// Every refusal here happens before the geocoder is built, so no request leaves.
    #[tokio::test]
    async fn a_stated_place_is_refused_before_any_lookup_when_malformed() {
        let state = AppState {
            database_path: Arc::new(scratch_database("stated-place")),
        };
        for (person, city, from, to, needle) in [
            ("Ron", " ", None, None, "required"),
            (" ", "Bonn", None, None, "required"),
            ("Ron", "Bonn", Some("14.10.2026"), None, "YYYY-MM-DD"),
            (
                "Ron",
                "Bonn",
                Some("2026-10-20"),
                Some("2026-10-13"),
                "after",
            ),
        ] {
            let (status, Json(body)) = state_person_place(
                State(state.clone()),
                Json(StatedPlace {
                    person: person.into(),
                    city: city.into(),
                    from: from.map(str::to_string),
                    to: to.map(str::to_string),
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(
                body["error"].as_str().unwrap_or_default().contains(needle),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn a_climate_batch_needs_exactly_one_selector() {
        let state = AppState {
            database_path: Arc::new(scratch_database("climate-selector")),
        };
        for (place_ids, at) in [
            (None, None),
            (Some("place_a".to_string()), Some("52.5,13.4".to_string())),
        ] {
            let (status, Json(body)) = climate(
                State(state.clone()),
                Query(ClimateBatch {
                    place_ids,
                    at,
                    from: None,
                    to: None,
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(
                body["error"].as_str().unwrap_or_default().contains("at="),
                "the 400 names both forms: {body}"
            );
        }
    }

    /// A key that never parsed is not a key with no match nearby. Telling a
    /// caller "no registered place with normals within 60 km" about a typo sends
    /// it looking for the wrong bug.
    #[tokio::test]
    async fn a_malformed_pair_is_told_it_is_malformed() {
        let state = AppState {
            database_path: Arc::new(scratch_database("climate-malformed")),
        };
        let (status, Json(body)) = climate(
            State(state),
            Query(ClimateBatch {
                place_ids: None,
                at: Some("abc;52.52,13.40".into()),
                from: None,
                to: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["results"][0]["key"], "abc");
        assert_eq!(body["results"][0]["reason"], "not a lat,lon pair");
        // The second key parses, and answers on its own terms.
        assert_eq!(body["results"][1]["key"], "52.52,13.40");
        assert!(
            body["results"][1]["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("60 km"),
            "an empty registry still answers with the distance sentence: {body}"
        );
    }

    #[test]
    fn a_plan_window_marks_the_months_it_covers() {
        assert_eq!(
            window_months(Some("2026-06-10"), Some("2026-08-02")),
            vec![6, 7, 8]
        );
        // Across a year boundary.
        assert_eq!(
            window_months(Some("2026-12-20"), Some("2027-01-04")),
            vec![12, 1]
        );
        // Longer than a year: every month, not an error.
        assert_eq!(
            window_months(Some("2026-03-01"), Some("2030-03-01")).len(),
            12
        );
        // No window is no marks, never all of them.
        assert!(window_months(None, Some("2026-08-02")).is_empty());
        assert!(window_months(Some("2026-08-02"), Some("2026-06-10")).is_empty());
    }

    #[test]
    fn only_the_two_review_decisions_exist() {
        assert_eq!(Review::Confirmed.as_str(), "confirmed");
        assert_eq!(Review::Dismissed.as_str(), "dismissed");
    }

    /// The router-level proof the two predicate tests (now in
    /// `libs/sjel-server/src/origin.rs`) could not give: a foreign `Origin`
    /// gets 403 from the WIRED router, so a route registered below the
    /// `.layer()` call fails here rather than shipping unguarded.
    ///
    /// Driven over a real loopback listener rather than through
    /// `tower::ServiceExt`, which is the pattern `libs/sjel-server`'s own
    /// `http_tests` already use and which needs no new dependency.
    #[tokio::test]
    async fn a_foreign_origin_cannot_read_people_presence() {
        let state = AppState {
            database_path: Arc::new(std::env::temp_dir().join("places-origin-test.db")),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(state)).await;
        });
        let client = reqwest::Client::new();

        // Every C2 surface, including the one added tonight. The database does
        // not have to exist: the guard runs before the handler.
        for path in [
            "/api/people/presence?latitude=50.0&longitude=8.0&from=2026-10-01&to=2026-10-08",
            "/api/people/proposals",
            "/api/layers/people",
        ] {
            let response = client
                .get(format!("{base}{path}"))
                .header("Origin", "https://evil.example")
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                403,
                "{path} answered a foreign origin — it is registered below the guard layer"
            );
        }

        // The control: no Origin header is not a browser cross-origin call, so
        // the request reaches the handler (which then fails on the database).
        let allowed = client
            .get(format!("{base}/api/people/proposals"))
            .send()
            .await
            .unwrap();
        assert_ne!(
            allowed.status(),
            403,
            "a server-to-server caller must not be refused"
        );
    }
}

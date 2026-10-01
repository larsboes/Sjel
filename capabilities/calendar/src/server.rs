use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use calendar::config::Config;
use calendar::content;
use calendar::correlate::{self, Candidate};
use calendar::date;
use calendar::google_sync::{self, HttpCalendarApi, Settings};
use calendar::markdown_import;
use calendar::model::{
    Commitment, Entry, NewContext, NewEntry, NewRhythm, UpdateContext, UpdateEntry, UpdateRhythm,
};
use calendar::store::CalendarStore;

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
    config: Arc<Config>,
}

type ApiResponse = (StatusCode, Json<Value>);

fn response<T: serde::Serialize>(status: StatusCode, value: T) -> ApiResponse {
    (
        status,
        Json(
            serde_json::to_value(value)
                .unwrap_or_else(|_| json!({ "error": "serialization failed" })),
        ),
    )
}

/// What this capability answers, served as data beside `/health`.
///
/// Query parameters that are *required* are named in the summary: a caller
/// reading a path alone has no way to learn that `from` and `to` are mandatory,
/// and discovering that from a 400 is the thing this endpoint exists to avoid.
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
        "/api/entries",
        "Entries overlapping a day window. Requires from, to; optional kind (CSV).",
    ),
    r("POST", "/api/entries", "Create an entry."),
    r("GET", "/api/entries/{id}", "One entry."),
    r(
        "PATCH",
        "/api/entries/{id}",
        "Patch an entry. Any patch detaches it from its rhythm.",
    ),
    r("DELETE", "/api/entries/{id}", "Delete an entry."),
    r(
        "PUT",
        "/api/entries/external",
        "Idempotent provider contribution. Requires source + external_id.",
    ),
    r(
        "GET",
        "/api/content/{source}/{id}",
        "The entry as content-item-v2. :source is always 'calendar'.",
    ),
    r(
        "GET",
        "/api/proposals",
        "Un-adopted external entries awaiting a decision. Requires from, to.",
    ),
    r(
        "GET",
        "/api/google/drafts",
        "Unreviewed Google imports. Requires from, to.",
    ),
    r(
        "GET",
        "/api/contexts",
        "Planning contexts overlapping a window. Requires from, to.",
    ),
    r("POST", "/api/contexts", "Create a planning context."),
    r("PATCH", "/api/contexts/{id}", "Patch a planning context."),
    r("DELETE", "/api/contexts/{id}", "Delete a planning context."),
    r("GET", "/api/rhythms", "Every rhythm."),
    r(
        "POST",
        "/api/rhythms",
        "Create a rhythm and materialize its future instances.",
    ),
    r("GET", "/api/rhythms/{id}", "One rhythm."),
    r("PATCH", "/api/rhythms/{id}", "Patch a rhythm."),
    r("DELETE", "/api/rhythms/{id}", "Delete a rhythm."),
    r(
        "POST",
        "/api/rhythms/{id}/materialize",
        "Re-materialize a rhythm's future instances.",
    ),
    r(
        "POST",
        "/api/verdicts",
        "Feasibility verdicts for a batch of dated candidates.",
    ),
    r(
        "GET",
        "/api/windows",
        "Runs of days where travel is possible. Requires from, to.",
    ),
    r(
        "GET",
        "/api/trip-drafts",
        "Events clustered by city and time proximity. Requires from, to.",
    ),
    r(
        "POST",
        "/api/trip-plans/{plan_id}/sync",
        "Write a plan's stages back as away entries (booked committed, option_selected planned, \
         planning and open possible), and any \
         booking's free-cancellation date as a deadline entry. Idempotent by external_id; \
         deletes nothing.",
    ),
    r(
        "POST",
        "/api/trip-drafts/materialize",
        "Turn a draft into a trips.plan.",
    ),
    r(
        "POST",
        "/api/google/import",
        "Import Google events as non-blocking drafts.",
    ),
    r(
        "POST",
        "/api/google/import-preview",
        "Read-only review of what an import would write.",
    ),
    r(
        "POST",
        "/api/google/import-selected",
        "Import an explicit selection, rejecting changed revisions.",
    ),
    r(
        "POST",
        "/api/google/export",
        "Push opted-in entries to Google.",
    ),
    r("GET", "/api/google/exports", "The export opt-in ledger."),
    r(
        "PUT",
        "/api/entries/{id}/google-export",
        "Opt an entry in to export.",
    ),
    r(
        "DELETE",
        "/api/entries/{id}/google-export",
        "Opt an entry out. The Google event is left alone.",
    ),
    r(
        "GET",
        "/api/markdown/sources",
        "Declared markdown event sources.",
    ),
    r(
        "POST",
        "/api/markdown/preview",
        "Read-only scan of a markdown source. Requires source.",
    ),
    r(
        "POST",
        "/api/markdown/import",
        "Import an explicit selection of scanned notes. Requires source and external_ids.",
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
    Json(route_manifest::manifest("calendar", ROUTES))
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "capability": "calendar"
    }))
}

/// Readiness: whether this capability can actually serve, which liveness does not answer.
///
/// `health` is a literal and cannot observe the database, so with the store unreachable this
/// capability reported itself up while every query behind it failed (#126). Availability is
/// judged here instead.
async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => response(
            StatusCode::OK,
            json!({ "ok": true, "capability": "calendar" }),
        ),
        // 503, not 400: the request was fine, the dependency is not, and a caller that retries
        // should be told to come back rather than to fix its input.
        Ok(Err(error)) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "calendar", "error": error }),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "calendar", "error": "readiness check failed" }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct EntriesQuery {
    from: String,
    to: String,
    /// Optional CSV kind filter: ?kind=busy,event
    kind: Option<String>,
}

/// Google drafts are source-owned imports still at `possible`, not entries of
/// a made-up `draft` kind. The dedicated endpoint keeps that meaning inside
/// Calendar rather than duplicating it in the dashboard.
#[derive(serde::Deserialize)]
struct GoogleDraftsQuery {
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
struct ProposalsQuery {
    from: String,
    to: String,
}

/// One entry as a LIST states it: the row, and what it is worth protecting.
///
/// `content.rs` has declared a class for this whole source since it was written
/// — `classification()`, "where the operator is and when is personal, whatever
/// the event itself is" — and it reached exactly one surface: the per-entry
/// content projection at `GET /content/calendar/{id}`. Nothing that reads a
/// LIST ever saw it, so the dashboard's ladder answered `null` for a calendar
/// row while the capability had an answer the whole time (B50, PRD §13.1).
///
/// A wrapper rather than a field on `Entry`: the class is a property of the
/// source, not a column, and putting it on the model would make fifteen struct
/// literals — most of them tests — carry a value none of them decides.
///
/// The VALUE only, matching comms' feed list
/// (`capabilities/comms/src/server/contracts.rs:174`). The rationale and the
/// method are not repeated on every row of a window that routinely holds
/// hundreds; they are one fetch away on `GET /content/calendar/{id}`, which
/// serves the whole `DataClass`.
#[derive(serde::Serialize)]
struct EntryListItem {
    #[serde(flatten)]
    entry: Entry,
    data_class: String,
}

impl EntryListItem {
    /// Classified once per request, not once per row: the declaration takes no
    /// argument, so a per-row call would allocate the same string N times.
    fn all(entries: Vec<Entry>) -> Vec<Self> {
        let data_class = content::classification().value;
        entries
            .into_iter()
            .map(|entry| Self {
                entry,
                data_class: data_class.clone(),
            })
            .collect()
    }
}

async fn list_entries(
    State(state): State<AppState>,
    Query(query): Query<EntriesQuery>,
) -> ApiResponse {
    let kinds: Vec<String> = query
        .kind
        .unwrap_or_default()
        .split(',')
        .map(|kind| kind.trim().to_string())
        .filter(|kind| !kind.is_empty())
        .collect();
    let from = query.from;
    let to = query.to;
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_entries(&from, &to, &kinds))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, EntryListItem::all(entries)),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_google_drafts(
    State(state): State<AppState>,
    Query(query): Query<GoogleDraftsQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_google_drafts(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, EntryListItem::all(entries)),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_external_proposals(
    State(state): State<AppState>,
    Query(query): Query<ProposalsQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_external_proposals(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entries)) => response(StatusCode::OK, EntryListItem::all(entries)),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_entry(State(state): State<AppState>, Json(input): Json<NewEntry>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.create_entry(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entry)) => response(StatusCode::CREATED, entry),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn upsert_external_entry(
    State(state): State<AppState>,
    Json(input): Json<NewEntry>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.upsert_external_entry(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(entry)) => response(StatusCode::OK, entry),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// The same entry as `content-item-v2`, for the one dashboard reader that also
/// renders feed articles and mail. A projection, not a second copy: the store
/// is not touched and nothing is written.
///
/// Path shape mirrors comms' `/content/{source}/{id}` deliberately. One contract
/// served under two different URL shapes is the same duplication the contract
/// exists to remove, one layer down — so the reader can build the URL from the
/// source alone rather than carrying a per-capability special case.
async fn get_content_item(
    State(state): State<AppState>,
    Path((source, id)): Path<(String, String)>,
) -> ApiResponse {
    // The segment is part of the shared shape, not a lookup key — calendar
    // serves exactly one source and says so rather than silently ignoring it.
    if source != "calendar" {
        return response(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("calendar serves the 'calendar' content source, not '{source}'") }),
        );
    }
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.get_entry(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(entry))) => response(StatusCode::OK, content::from_entry(&entry)),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_entry(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.get_entry(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(entry))) => response(StatusCode::OK, entry),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateEntry>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.update_entry(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(entry))) => response(StatusCode::OK, entry),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_entry(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.delete_entry(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(StatusCode::NOT_FOUND, json!({ "error": "entry not found" })),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ContextsQuery {
    from: String,
    to: String,
}

async fn list_contexts(
    State(state): State<AppState>,
    Query(query): Query<ContextsQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_contexts(&query.from, &query.to))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(contexts)) => response(StatusCode::OK, contexts),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_context(
    State(state): State<AppState>,
    Json(input): Json<NewContext>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.create_context(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(context)) => response(StatusCode::CREATED, context),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_context(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateContext>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.update_context(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(context))) => response(StatusCode::OK, context),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "context not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_context(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.delete_context(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "context not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_rhythms(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_rhythms())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(rhythms)) => response(StatusCode::OK, rhythms),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_rhythm(State(state): State<AppState>, Json(input): Json<NewRhythm>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.create_rhythm(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok((rhythm, created))) => response(
            StatusCode::CREATED,
            json!({ "rhythm": rhythm, "instances_created": created }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_rhythm(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.get_rhythm(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(rhythm))) => response(StatusCode::OK, rhythm),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_rhythm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateRhythm>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.update_rhythm(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some((rhythm, affected)))) => response(
            StatusCode::OK,
            json!({ "rhythm": rhythm, "future_instances_affected": affected }),
        ),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct DeleteRhythmQuery {
    delete_instances: Option<bool>,
}

async fn delete_rhythm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeleteRhythmQuery>,
) -> ApiResponse {
    let delete_instances = query.delete_instances.unwrap_or(false);
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.delete_rhythm(&id, delete_instances))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "deleted": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn materialize_rhythm(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.materialize_rhythm(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(created))) => response(StatusCode::OK, json!({ "instances_created": created })),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "rhythm not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---- correlation (Phase C) ------------------------------------------------

#[derive(serde::Deserialize)]
struct VerdictsRequest {
    candidates: Vec<Candidate>,
}

/// Soft feasibility verdicts for a batch of dated candidates. Batched because
/// Feed's Discover view asks about a screenful of opportunities at once, and
/// one span-covering read beats one query per row.
async fn candidate_verdicts(
    State(state): State<AppState>,
    Json(input): Json<VerdictsRequest>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let candidates = input.candidates;
        let window = correlate::query_window(&candidates)?;
        let entries = match window {
            Some((from, to)) => CalendarStore::open(&database_path)
                .and_then(|store| store.list_entries(&from, &to, &[]))
                .map_err(|error| error.to_string())?,
            // No candidates, no query — an empty ask is not an error.
            None => Vec::new(),
        };
        let verdicts = correlate::verdicts_for(&candidates, &entries)?;
        Ok(json!({ "verdicts": verdicts }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct MaterializeBody {
    /// The entries this trip is made of. Explicit rather than "the draft at
    /// place X": drafts are recomputed per request, so naming one by position
    /// would race any edit made between reading and confirming.
    entry_ids: Vec<String>,
    #[serde(default)]
    title: Option<String>,
}

/// Writes a plan's committed travel back into the calendar.
///
/// The loop only ran one way. Calendar could turn clustered entries into a
/// `trips.plan`, but a plan whose stage was `booked` produced no entry, so
/// `POST /api/verdicts` called that week free and `GET /api/windows` offered it
/// as a feasible travel window. The system could propose a trip on top of a trip
/// it had created itself.
///
/// The direction is calendar-to-trips on purpose, and stays that way: calendar
/// already holds the trips HTTP client, the base URL and the idempotence ledger,
/// while trips has no outbound client at all. Having trips push would give two
/// capabilities an HTTP client for each other and put the deadline entry in a
/// second `external_id` namespace with nothing reconciling them.
///
/// Idempotent by construction rather than by ledger: every entry is written
/// through the same `upsert_external_entry` the Google import uses, keyed
/// `trip:stage:<id>` or `trip:booking:<id>`, so running it twice updates in
/// place. Nothing is deleted -- a stage that stops being booked leaves its entry
/// behind, and removing it is the operator's call, not a sync's.
async fn sync_trip_plan(State(state): State<AppState>, Path(plan_id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let config = state.config.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_path).map_err(|e| e.to_string())?;
        let client = sjel_http::client(sjel_http::Purpose::new("calendar-trips"), std::time::Duration::from_secs(20))
            .map_err(|e| format!("client build: {e}"))?;
        let base = config.trips_base_url.trim_end_matches('/').to_string();

        let url = format!("{base}/api/plans/{plan_id}");
        let request = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url);
        let response = request
            .send()
            .map_err(|e| format!("GET {url}: {e}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(format!("no plan {plan_id}"));
        }
        if !response.status().is_success() {
            return Err(format!("trips answered {} for {plan_id}", response.status()));
        }
        let plan: Value = response
            .json()
            .map_err(|e| format!("trips returned something unreadable: {e}"))?;

        let mut written = Vec::new();
        let mut skipped = Vec::new();

        for stage in plan
            .get("stages")
            .and_then(|s| s.as_array())
            .map(|s| s.as_slice())
            .unwrap_or_default()
        {
            let status = stage.get("status").and_then(|s| s.as_str()).unwrap_or("");
            // `planning` and `open` are not a decision yet, so they are written as
            // `possible`, which never blocks a day: the trip is visible in the
            // calendar without making the week look taken. Until 2026-09-25 they
            // were skipped, and a trip twelve days out with no leg booked did not
            // appear at all. `completed` is history and stays out.
            let commitment = match status {
                "booked" => Commitment::Committed,
                "option_selected" => Commitment::Planned,
                "planning" | "open" => Commitment::Possible,
                _ => continue,
            };
            let stage_id = stage.get("id").and_then(|s| s.as_str()).unwrap_or_default();
            let Some(date) = stage.get("date").and_then(|d| d.as_str()) else {
                // A stage with no date cannot be placed, and a travel day guessed
                // from the plan window would block a day nobody chose.
                skipped.push(json!({ "stage": stage_id, "reason": "no date" }));
                continue;
            };
            let Some(day) = date::parse_date(date) else {
                skipped.push(json!({ "stage": stage_id, "reason": "unreadable date" }));
                continue;
            };

            let place = |key: &str| -> String {
                stage
                    .get(key)
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string()
            };
            let entry = NewEntry {
                kind: "away".to_string(),
                commitment,
                title: format!("{} → {}", place("origin"), place("destination")),
                starts_at: date.to_string(),
                // Calendar ends are exclusive; a one-day all-day entry ends the
                // next day. The same unit mismatch the materialize path documents,
                // in the other direction.
                ends_at: date::format_date(day + 1),
                all_day: true,
                location: Some(place("destination")),
                notes: None,
                source: "trips".to_string(),
                external_id: Some(format!("trip:stage:{stage_id}")),
                rhythm_id: None,
                payload: json!({ "plan_id": plan_id, "stage_id": stage_id, "stage_status": status }),
            };
            let saved = store.upsert_external_entry(&entry).map_err(|e| e.to_string())?;
            written.push(json!({ "entry_id": saved.id, "kind": "away", "stage": stage_id }));
        }

        // Booking deadlines, from the same read. A free-cancellation date is the
        // one field in a booking with a deadline attached, and `deadline` is
        // calendar's kind for exactly that: visible evidence, never a time block.
        for item in plan
            .get("items")
            .and_then(|i| i.as_array())
            .map(|i| i.as_slice())
            .unwrap_or_default()
        {
            if item.get("item_type").and_then(|t| t.as_str()) != Some("booking") {
                continue;
            }
            let payload = item.get("payload").cloned().unwrap_or(Value::Null);
            let Some(until) = payload
                .get("free_cancellation_until")
                .and_then(|d| d.as_str())
            else {
                continue;
            };
            let Some(day) = date::parse_date(&until[..until.len().min(10)]) else {
                continue;
            };
            let item_id = item.get("id").and_then(|i| i.as_str()).unwrap_or_default();
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("booking");
            let entry = NewEntry {
                kind: "deadline".to_string(),
                // A cancellation deadline is a fact about a booking that exists,
                // not something you might do.
                commitment: Commitment::Committed,
                title: format!("Free cancellation ends: {title}"),
                starts_at: date::format_date(day),
                ends_at: date::format_date(day + 1),
                all_day: true,
                location: None,
                notes: payload
                    .get("order_ref")
                    .and_then(|r| r.as_str())
                    .map(|r| format!("Order {r}")),
                source: "trips".to_string(),
                external_id: Some(format!("trip:booking:{item_id}")),
                rhythm_id: None,
                payload: json!({ "plan_id": plan_id, "item_id": item_id }),
            };
            let saved = store.upsert_external_entry(&entry).map_err(|e| e.to_string())?;
            written.push(json!({ "entry_id": saved.id, "kind": "deadline", "item": item_id }));
        }

        Ok(json!({
            "plan_id": plan_id,
            "written": written.len(),
            "entries": written,
            "skipped": skipped,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Turns a set of entries into a `trips.plan`.
///
/// Calendar posts to trips' public HTTP API and never touches its store. The
/// ledger it does own records which entries already became a plan, so asking
/// twice returns the plan that exists instead of making a second one.
async fn materialize_trip(
    State(state): State<AppState>,
    Json(body): Json<MaterializeBody>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let config = state.config.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        if body.entry_ids.is_empty() {
            return Err("entry_ids is required".into());
        }
        let store = CalendarStore::open(&database_path).map_err(|e| e.to_string())?;

        let client = sjel_http::client(
            sjel_http::Purpose::new("calendar-trips"),
            std::time::Duration::from_secs(20),
        )
        .map_err(|e| format!("client build: {e}"))?;
        let base = config.trips_base_url.trim_end_matches('/').to_string();

        // Already a trip? Only if trips still has it. The ledger records what
        // calendar did, not what trips kept, so a plan deleted over there would
        // otherwise leave the entry permanently refusing to become one again
        // and pointing at something that no longer exists.
        for entry_id in &body.entry_ids {
            let Some(plan_id) = store.trip_plan_for(entry_id).map_err(|e| e.to_string())? else {
                continue;
            };
            let url = format!("{base}/api/plans/{plan_id}");
            let probe = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
                .send()
                .map_err(|e| {
                    // Unreachable is not the same as gone. Forgetting the row
                    // here would turn a trips outage into a duplicate plan, so
                    // this fails loudly instead.
                    format!("cannot ask trips whether {plan_id} still exists ({e})")
                })?;
            if probe.status().is_success() {
                return Ok(json!({
                    "plan_id": plan_id,
                    "created": false,
                    "reason": format!("{entry_id} already belongs to {plan_id}"),
                }));
            }
            if probe.status() == reqwest::StatusCode::NOT_FOUND {
                let forgotten = store
                    .forget_trip_materialization(&plan_id)
                    .map_err(|e| e.to_string())?;
                eprintln!("  trips: {plan_id} is gone, forgetting {forgotten} stale ledger row(s)");
                continue;
            }
            return Err(format!(
                "trips answered {} for {plan_id}, which is neither yes nor no",
                probe.status()
            ));
        }

        let mut entries = Vec::new();
        for entry_id in &body.entry_ids {
            let entry = store
                .get_entry(entry_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no entry {entry_id}"))?;
            entries.push(entry);
        }

        let drafts = correlate::cluster_trips(&entries, i64::MAX, None)?;
        let draft = drafts
            .drafts
            .first()
            .ok_or("none of those entries can be placed, so there is nothing to travel to")?;
        if drafts.drafts.len() > 1 {
            return Err(format!(
                "those entries are in {} different places; one trip goes to one place",
                drafts.drafts.len()
            ));
        }

        // trips' date_end is INCLUSIVE; every end in calendar is exclusive.
        // Handing ends_before straight over would add a day to every trip, and
        // it would look like a rounding quirk rather than a unit mismatch.
        let date_end = date::parse_date(&draft.ends_before)
            .map(|day| date::format_date(day - 1))
            .ok_or("unreadable draft end")?;

        let title = body
            .title
            .clone()
            .unwrap_or_else(|| format!("{} — {}", draft.place, draft.starts_on));
        let payload = json!({
            "title": title,
            "origin": { "id": "", "name": config.home_city.clone().unwrap_or_default() },
            "destinations": [{ "id": "", "name": draft.place }],
            "date_start": draft.starts_on,
            "date_end": date_end,
            "interests": draft.titles.join(" · "),
        });

        let url = format!("{base}/api/plans");
        let request =
            sjel_server::InboundAuth::with_loopback_auth(client.post(&url).json(&payload), &url);
        let response = request.send().map_err(|e| format!("POST {url}: {e}"))?;
        let status = response.status();
        let plan: Value = response
            .json()
            .map_err(|e| format!("trips returned something unreadable: {e}"))?;
        if !status.is_success() {
            return Err(format!("trips refused the plan ({status}): {plan}"));
        }
        let plan_id = plan
            .get("id")
            .and_then(|id| id.as_str())
            .ok_or("trips accepted the plan but returned no id")?
            .to_string();

        // Only now, after trips confirmed it exists.
        store
            .record_trip_materialization(&body.entry_ids, &plan_id)
            .map_err(|e| e.to_string())?;

        Ok(json!({ "plan_id": plan_id, "created": true, "plan": plan }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct TripDraftsQuery {
    from: String,
    to: String,
    /// How far apart two things in the same place can be and still be one
    /// journey. Defaults to the five days the issue's own example uses.
    max_gap_days: Option<i64>,
    /// The place that is never a trip. Falls back to `home_city` in the
    /// capability config, and passing neither clusters everything including
    /// where you live, which is visible rather than silently wrong.
    home: Option<String>,
}

/// Which events belong to one journey.
///
/// Recomputed per request rather than stored: a draft is a function of the
/// entries, and every one of them can move. Materialising a draft into a real
/// `trips.plan` is an explicit act elsewhere, which is what keeps this cheap
/// enough to recompute and keeps calendar out of trips' domain.
async fn trip_drafts(
    State(state): State<AppState>,
    Query(query): Query<TripDraftsQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let config_home = state.config.home_city.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let max_gap_days = query.max_gap_days.unwrap_or(5);
        if max_gap_days < 0 {
            return Err("max_gap_days cannot be negative".into());
        }
        let home = query.home.clone().or(config_home);
        let entries = CalendarStore::open(&database_path)
            .and_then(|store| store.list_entries(&query.from, &query.to, &[]))
            .map_err(|error| error.to_string())?;
        let drafts = correlate::cluster_trips(&entries, max_gap_days, home.as_deref())?;
        Ok(json!({
            "from": query.from,
            "to": query.to,
            "max_gap_days": max_gap_days,
            "home": home,
            "drafts": drafts.drafts,
            "unclustered": drafts.unclustered,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct WindowsQuery {
    from: String,
    to: String,
    /// Shortest run worth returning; defaults to a single day.
    min_days: Option<usize>,
}

/// The runs of days travel is possible in — what a fare search should be
/// constrained to. Calendar computes availability and stops there; handing
/// these days to `transit plan --dates` is the caller's move, so neither
/// capability learns the other's domain (see the README's why-block).
async fn windows(State(state): State<AppState>, Query(query): Query<WindowsQuery>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let from_day = date::parse_date(&query.from).ok_or("from must be YYYY-MM-DD")?;
        let to_day = date::parse_date(&query.to).ok_or("to must be YYYY-MM-DD")?;
        if to_day <= from_day {
            return Err("to must be after from (the window end is exclusive)".into());
        }
        let min_days = query.min_days.unwrap_or(1);
        let entries = CalendarStore::open(&database_path)
            .and_then(|store| store.list_entries(&query.from, &query.to, &[]))
            .map_err(|error| error.to_string())?;
        let windows = correlate::feasible_windows(from_day, to_day, &entries, min_days)?;
        Ok(json!({
            "from": query.from,
            "to": query.to,
            "min_days": min_days,
            "windows": windows,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---- google sync (Phase E) ------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct SyncRequest {
    /// Reports what would happen and writes nothing. Not the default: a run
    /// the operator asked for should do the thing.
    #[serde(default)]
    dry_run: bool,
}

/// A deliberate, bounded provider slice for the import-review UI. Dates are
/// date-only and form an exclusive `[from, to)` window, like entry queries.
#[derive(serde::Deserialize)]
struct GoogleImportPreviewRequest {
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
struct GoogleSelectedImportRequest {
    from: String,
    to: String,
    selected: Vec<google_sync::SelectedGoogleEvent>,
}

/// Resolves the settings both runs need, turning a missing home timezone or
/// calendar id into a 400 that names the config key rather than a 500.
fn google_settings(state: &AppState) -> Result<Settings, ApiResponse> {
    Settings::resolve(&state.config)
        .map_err(|error| response(StatusCode::BAD_REQUEST, json!({ "error": error })))
}

/// Pulls the configured Google calendar in as drafts.
///
/// Blocking work — the Google client and the store are both synchronous — so
/// it runs on the blocking pool like every other handler here. Missing
/// credentials surface as a 400 naming the file and key, never as an empty
/// success.
async fn google_import(
    State(state): State<AppState>,
    Json(input): Json<SyncRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_path).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::import(&store, &api, &settings, input.dry_run)?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Read-only candidate review. This intentionally does not reuse the broad
/// unattended-import window: an operator should first see a small, chosen
/// time range and any likely duplicates.
async fn google_import_preview(
    State(state): State<AppState>,
    Json(input): Json<GoogleImportPreviewRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_path).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let preview = google_sync::preview(&store, &api, &settings, &input.from, &input.to)?;
        serde_json::to_value(preview).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Commits only explicit, still-current selections from a prior preview. The
/// Google event revisions are checked before any Axon entry is written.
async fn google_import_selected(
    State(state): State<AppState>,
    Json(input): Json<GoogleSelectedImportRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_path).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::import_selected(
            &store,
            &api,
            &settings,
            &input.from,
            &input.to,
            &input.selected,
        )?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        // A stale candidate is an expected review result, not a server error.
        Ok(Err(error)) if error.contains("review again") => {
            response(StatusCode::CONFLICT, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Pushes the opted-in entries, and only those.
async fn google_export(
    State(state): State<AppState>,
    Json(input): Json<SyncRequest>,
) -> ApiResponse {
    let settings = match google_settings(&state) {
        Ok(settings) => settings,
        Err(error) => return error,
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = CalendarStore::open(&database_path).map_err(|error| error.to_string())?;
        let env_path = settings.google.env_path();
        let api = HttpCalendarApi::new(&env_path);
        let report = google_sync::export(&store, &api, &settings, input.dry_run)?;
        serde_json::to_value(report).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_export_optins(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.list_export_optins())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(optins)) => response(StatusCode::OK, optins),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize, Default)]
struct OptInRequest {
    /// Which Google calendar this entry belongs on. Defaults to the configured
    /// one; recorded on the ledger row so a later config change cannot
    /// relocate an event that has already been pushed.
    #[serde(default)]
    google_calendar_id: Option<String>,
}

/// Opts one entry in to export. Nothing exports until this is called, and
/// `store::opt_in_export` refuses the entries that must never be pushed (an
/// imported Google event, a generated rhythm instance).
async fn opt_in_export(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<OptInRequest>>,
) -> ApiResponse {
    let requested = body.map(|Json(input)| input).unwrap_or_default();
    let calendar_id = match requested
        .google_calendar_id
        .or_else(|| state.config.google.calendar_id.clone())
    {
        Some(calendar_id) => calendar_id,
        None => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "no google_calendar_id given and none configured — set google.calendar_id in the overlay's calendar.json or pass it in the body"
                }),
            );
        }
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.opt_in_export(&id, &calendar_id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(optin)) => response(StatusCode::OK, optin),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Opts an entry back out. The Google event it already created is deliberately
/// left alone: deleting someone's calendar entry as a side effect of a toggle
/// is not a decision this capability makes.
async fn opt_out_export(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        CalendarStore::open(&database_path)
            .and_then(|store| store.opt_out_export(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "opted_out": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "entry is not opted in to export" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---------------------------------------------------------------------------
// Markdown event import (#231). Three endpoints because the contract has three
// steps and collapsing them would lose the middle one: see what is there, then
// say what to write, then write exactly that.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct MarkdownScanRequest {
    source: String,
}

#[derive(serde::Deserialize)]
struct MarkdownImportRequest {
    source: String,
    /// The notes to write, by the `external_id` the preview showed. No "import
    /// everything" flag: a caller that wants the whole preview sends the whole
    /// preview's ids back, which keeps "I reviewed this" and "write it" the
    /// same act.
    external_ids: Vec<String>,
}

/// What the operator has declared, so a caller does not have to read the
/// overlay's config file to find out. Paths included: reviewing an import means
/// knowing which store it came from.
async fn list_markdown_sources(State(state): State<AppState>) -> ApiResponse {
    response(
        StatusCode::OK,
        json!({ "sources": state.config.markdown_sources }),
    )
}

fn markdown_source(
    state: &AppState,
    id: &str,
) -> Result<calendar::markdown_import::MarkdownSource, ApiResponse> {
    state.config.markdown_source(id).cloned().ok_or_else(|| {
        response(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("no enabled markdown source '{id}'") }),
        )
    })
}

/// Reads a declared source and writes nothing. Filesystem work, so it runs on
/// the blocking pool like every other handler here.
async fn markdown_preview(
    State(state): State<AppState>,
    Json(input): Json<MarkdownScanRequest>,
) -> ApiResponse {
    let source = match markdown_source(&state, &input.source) {
        Ok(source) => source,
        Err(error) => return error,
    };
    match tokio::task::spawn_blocking(move || markdown_import::scan(&source)).await {
        Ok(Ok(preview)) => response(StatusCode::OK, preview),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Writes exactly the notes named, and re-scans first rather than trusting the
/// caller's copy of the preview: the file is the source of truth, and it may
/// have changed since it was reviewed. An id the fresh scan no longer offers is
/// a 400 naming it, not a silent skip.
///
/// Every write goes through `upsert_external_entry`, so the `(source,
/// external_id)` unique index makes a second import an update. That is what
/// makes this safe to re-run, which is what makes it safe to run at all.
async fn markdown_import_selected(
    State(state): State<AppState>,
    Json(input): Json<MarkdownImportRequest>,
) -> ApiResponse {
    let source = match markdown_source(&state, &input.source) {
        Ok(source) => source,
        Err(error) => return error,
    };
    if input.external_ids.is_empty() {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "external_ids is required: an import names what it writes" }),
        );
    }
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let preview = markdown_import::scan(&source)?;
        let selected = markdown_import::plan(&preview, &input.external_ids)?;
        let store = CalendarStore::open(&database_path).map_err(|error| error.to_string())?;
        let mut imported = Vec::new();
        for candidate in selected {
            let entry = store
                .upsert_external_entry(&candidate.entry)
                .map_err(|error| format!("{}: {error}", candidate.external_id))?;
            imported.push(json!({ "external_id": candidate.external_id, "id": entry.id }));
        }
        Ok(json!({
            "source": preview.source,
            "imported": imported,
            "count": imported.len(),
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::load();
    let port = config.port;
    let state = AppState {
        database_path: Arc::new(config.database_path.clone()),
        config: Arc::new(config),
    };
    sjel_server::serve_local("calendar", port, build_router(state)).await;
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_CALENDAR_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "calendar";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// This is the largest personal read in the workspace and it sat under
/// `CorsLayer::permissive()` with nothing above it: `GET /api/entries` returns
/// the operator's calendar — titles, times, locations and the context each
/// entry belongs to — to any page open in their browser. `POST
/// /api/rhythms/{id}/materialize` and `POST /api/trip-plans/{plan_id}/sync` take
/// no request body at all, so a cross-site *simple* POST reaches them with no
/// preflight for CORS to refuse; refusing the request, which is what this guard
/// does, is what closes that.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`), so a route appended under it would silently
/// lose the refusal.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/entries", get(list_entries).post(create_entry))
        .route("/api/google/drafts", get(list_google_drafts))
        .route("/api/proposals", get(list_external_proposals))
        .route("/api/entries/external", put(upsert_external_entry))
        .route(
            "/api/entries/{id}",
            get(get_entry).patch(update_entry).delete(delete_entry),
        )
        .route("/api/content/{source}/{id}", get(get_content_item))
        .route("/api/contexts", get(list_contexts).post(create_context))
        .route(
            "/api/contexts/{id}",
            axum::routing::patch(update_context).delete(delete_context),
        )
        .route("/api/rhythms", get(list_rhythms).post(create_rhythm))
        .route(
            "/api/rhythms/{id}",
            get(get_rhythm).patch(update_rhythm).delete(delete_rhythm),
        )
        .route("/api/rhythms/{id}/materialize", post(materialize_rhythm))
        .route("/api/verdicts", post(candidate_verdicts))
        .route("/api/windows", get(windows))
        .route("/api/trip-drafts", get(trip_drafts))
        .route("/api/trip-drafts/materialize", post(materialize_trip))
        .route("/api/trip-plans/{plan_id}/sync", post(sync_trip_plan))
        .route("/api/google/import", post(google_import))
        .route("/api/google/import-preview", post(google_import_preview))
        .route("/api/google/import-selected", post(google_import_selected))
        .route("/api/google/export", post(google_export))
        .route("/api/google/exports", get(list_export_optins))
        .route(
            "/api/entries/{id}/google-export",
            put(opt_in_export).delete(opt_out_export),
        )
        .route("/api/markdown/sources", get(list_markdown_sources))
        .route("/api/markdown/preview", post(markdown_preview))
        .route("/api/markdown/import", post(markdown_import_selected))
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod entry_list_class_tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            id: "evt-1".into(),
            kind: "event".into(),
            commitment: Commitment::Possible,
            title: "Rust meetup".into(),
            starts_at: "2026-09-10T19:00:00".into(),
            ends_at: "2026-09-10T21:00:00".into(),
            all_day: false,
            location: Some("Bonn".into()),
            notes: None,
            source: "web".into(),
            external_id: None,
            rhythm_id: None,
            payload: json!({}),
            created_at: "1788000000".into(),
            updated_at: "1788000000".into(),
        }
    }

    /// The class on the list is the one `content.rs` declares, read from it.
    ///
    /// Both assertions are load-bearing. The first says the list did not invent
    /// a class; the second says the declaration itself is still c1, so a change
    /// to `classification()` cannot slide past a test that only compares the
    /// list against it.
    #[test]
    fn the_list_states_the_class_the_source_declares() {
        let body = serde_json::to_value(EntryListItem::all(vec![entry()]))
            .expect("an entry list serializes");

        assert_eq!(body[0]["data_class"], content::classification().value);
        assert_eq!(body[0]["data_class"], "c1");
        // The row itself is untouched: `flatten` adds a key, it does not nest.
        assert_eq!(body[0]["id"], "evt-1");
        assert_eq!(body[0]["title"], "Rust meetup");
    }

    /// A handler that serves entries and states no class.
    ///
    /// The three that exist — entries, drafts, external proposals — share one
    /// line, and the failure this guards is a fourth that does not:
    /// `GET /api/entries` publishing a class while `/api/proposals` did not
    /// would be the same contract gap B50 found, one endpoint further along.
    ///
    /// The needle is composed at runtime on purpose. This module is inside
    /// `server.rs`, so `include_str!` reads the test's own source too, and a
    /// literal needle would match itself and pass forever.
    #[test]
    fn no_handler_serves_a_list_of_entries_without_a_class() {
        let bare = format!("response(StatusCode::OK, {}", "entries)");
        let wrapped = format!("EntryListItem::all({}", "entries)");
        let source = include_str!("server.rs");
        assert!(
            !source.contains(&bare),
            "a list handler answers with bare entries; wrap it in EntryListItem::all"
        );
        assert_eq!(
            source.matches(&wrapped).count(),
            3,
            "entries, drafts and external proposals are the three entry lists"
        );
    }
}

#[cfg(test)]
mod route_manifest_tests {
    use super::ROUTES;

    /// A stale manifest is worse than none, because it gets believed. This
    /// reads the router's own source, so adding a `.route()` without a summary
    /// fails here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}

/// The router-level proof that `libs/sjel-server`'s predicate tests cannot give:
/// a route registered BELOW the `.layer()` call passes every test of
/// `origin_allowed_by` and still answers a hostile page.
///
/// `/routes` is the control rather than `/api/entries`, because every data
/// handler here opens the deployment's SQLite file and a test must not. The
/// refusal is asserted on the data routes, where the guard answers before the
/// handler runs and nothing is opened.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn router() -> Router {
        let config = Config::load();
        build_router(AppState {
            database_path: Arc::new(config.database_path.clone()),
            config: Arc::new(config),
        })
    }

    async fn answer(method: &str, path: &str, origin: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers")
            .status()
    }

    /// The two POSTs in this list take no request body, so a hostile page can
    /// send them as *simple* requests — no preflight, nothing for a CORS policy
    /// to refuse. They are the reason the guard refuses the request rather than
    /// merely withholding a response header.
    #[tokio::test]
    async fn a_foreign_origin_can_neither_read_the_calendar_nor_drive_a_write() {
        for (method, path) in [
            ("GET", "/api/entries"),
            ("GET", "/api/windows"),
            ("GET", "/api/contexts"),
            ("POST", "/api/rhythms/r-1/materialize"),
            ("POST", "/api/trip-plans/p-1/sync"),
        ] {
            assert_eq!(
                answer(method, path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{method} {path} answered a foreign origin — it is registered below the guard layer"
            );
        }
    }

    /// The other half. 200 from `/routes` is a handler answering.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                answer("GET", "/routes", origin).await,
                StatusCode::OK,
                "the guard refused a caller it must admit: {origin:?}"
            );
        }
    }
}

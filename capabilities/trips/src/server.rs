use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde::Serialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use trips::config::{Config, ObsidianConfig};
use trips::kiwi::KiwiClient;
use trips::obsidian::{read_trip_note, scan_trip_notes, ObsidianTripCandidate};
use trips::store::{
    CreatePlan, CreatePlanItem, PlaceRef, PlanSource, TripPlan, TripsStore, UpdatePlan,
};

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r("GET", "/api/plans", "Every trip plan."),
    route_manifest::Route {
        method: "POST",
        path: "/api/plans",
        summary: "Create a trip plan.",
        request_schema: Some(route_manifest::schema_of::<CreatePlan>),
    },
    r(
        "GET",
        "/api/plans/:id",
        "One trip plan with its stages and items.",
    ),
    r(
        "PATCH",
        "/api/plans/:id",
        "Patch a trip plan. Optional expected_updated_at (body) makes the write conditional: \
         a mismatch is 409 with code stale_plan instead of overwriting another writer. \
         budget_cents + currency record what the trip is meant to cost.",
    ),
    r(
        "DELETE",
        "/api/plans/:id",
        "Delete a trip plan. Optional expected_updated_at (query) makes it conditional, \
         409 with code stale_plan on a mismatch.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/plans/:id/items",
        summary: "Add an item to a plan. Four item_types (transport, option_set, booking, \
                  stay) promise a payload shape and are validated on write: see \
                  schemas/trip-plan.schema.json.",
        request_schema: Some(route_manifest::schema_of::<CreatePlanItem>),
    },
    r(
        "PATCH",
        "/api/plans/:plan_id/items/:item_id",
        "Move an item to a day. Body is {day: \"YYYY-MM-DD\"} or {day: null} to unset.",
    ),
    r(
        "POST",
        "/api/plans/:id/outcome",
        "Record how a stage actually went. Requires stage_id; every other field is kept as \
         observed. Refused when the stage has no selected_option_id, because there is then \
         nothing to compare an actual against.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/bases",
        summary: "Where to base yourself between two fixed points. Body: { from, anchor, \
                  from_date, anchor_date, max_candidates? }. Prices the two legs that bound \
                  a stay -- reaching the base and leaving it for the anchor -- and reports \
                  who the companion register already puts nearby. Ranked cheapest total \
                  first; a base with only one leg priced has no total rather than a partial \
                  one. Synchronous, because a fare search answers in well under a second \
                  warm and ten candidates is about ten seconds. `stay_cents` is ALWAYS null \
                  with a reason: no accommodation source exists, and an estimated nightly \
                  rate would be indistinguishable from a found one.",
        request_schema: Some(route_manifest::schema_of::<trips::bases::BaseRequest>),
    },
    r(
        "GET",
        "/api/places",
        "Every place across all plans, with visit counts and merge_candidates: distinct \
         place ids whose names normalise to the same string. Computed on read.",
    ),
    r(
        "DELETE",
        "/api/plans/:plan_id/items/:item_id",
        "Remove an item from a plan.",
    ),
    r(
        "GET",
        "/api/import/obsidian/scan",
        "Vault trip notes that could be imported. Read-only.",
    ),
    r(
        "POST",
        "/api/import/obsidian",
        "Import one vault trip note.",
    ),
    r(
        "POST",
        "/api/import/obsidian/all",
        "Import every scanned vault trip note.",
    ),
    r(
        "GET",
        "/api/flights/search",
        "Flight search via Kiwi.com's open MCP endpoint. Query: from, to (IATA or place \
         name), date (YYYY-MM-DD), optional flex_days (0-10, widens the search +/- N days) \
         and return_date. Segments carry naive airport-local times AND resolved UTC \
         instants; hidden_ground_transfers surfaces airport changes route[] hides. \
         Self-rate-limited; the endpoint publishes no quota, treat withdrawal as expected.",
    ),
    r(
        "GET",
        "/api/flights/grid",
        "Cheapest flight per departure day across a flexible window: one Kiwi search with \
         flex_days (default 3, max 10) around date, reduced to cheapest-per-day. Query: \
         from, to, date (YYYY-MM-DD), optional flex_days. Days won by a hidden \
         self-transfer are flagged. Date flexibility is the 40-54% price axis; this is \
         where the money is.",
    ),
    r(
        "GET",
        "/api/flights/when",
        "When could I go: every day of a span priced and joined with the calendar. \
         Query: from, to, date_from, date_to (YYYY-MM-DD, span capped at 42 days). \
         Free days rank cheapest-first, then planned, committed last with the \
         colliding entries named. An unreachable calendar marks every day \
         load=unknown and sets calendar/degraded in the reply, rather than \
         ranking a committed week as free. Costs one Kiwi search per 21 span days.",
    ),
    r(
        "GET",
        "/api/flights/pivot",
        "Itineraries through the friend graph: origin to each configured pivot city, a \
         free night or two there, then onward -- the routing no commercial engine can \
         know. Query: to, date (YYYY-MM-DD); optional from (defaults to the configured \
         home airport). Pivots come from the overlay's trips.json travel section; without \
         them this answers 400, not an empty success. Every option is separate tickets \
         with no through-protection, and says so.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/plans/:id/retrospective",
        summary: "Record or correct one plan's retrospective: exactly the three fields PRD 8.2 \
                  rules -- cost_cents (in the PLAN's currency; a plan with none refuses a cost), \
                  again (yes|no|maybe|not_taken) and change_note. One row per plan, so a second POST is a \
                  correction and answers 200. Different from POST /api/plans/:id/outcome, which \
                  measures one stage against the option it was chosen under.",
        request_schema: Some(route_manifest::schema_of::<RetrospectiveBody>),
    },
    r(
        "GET",
        "/api/retrospectives/pending",
        "Closed trips with no retrospective, inside the 45-day window: what the dashboard \
         ladder should raise today. No parameters. Archived plans are excluded, because \
         archiving is the operator's explicit 'I am done with this'.",
    ),
    r(
        "GET",
        "/api/plans/:id/cost",
        "What one trip was meant to cost, what was committed to, and what was actually paid. \
         No parameters: the window is the plan's own dates, so nobody can ask for a partial \
         total. Carries all four of finance's per-trip figures rather than one flattened \
         total, and when finance does not answer they are null with a named reason -- never 0. \
         option_set and transport prices are floats with no currency and are reported \
         separately, labelled offered-not-paid.",
    ),
    r(
        "GET",
        "/api/retrospectives/summary",
        "The feed-forward weight, per DESTINATION only. Carries the formula, the bounds and \
         the contract a consumer is held to: multiply a candidate's rank by factor and show \
         the basis, never filter on it. by_companion is a sentence rather than data -- see \
         capabilities/trips/README.md for the three preconditions.",
    ),
    // --- plan search and pack lists (night-2026-09-03) --------------------
    route_manifest::Route {
        method: "POST",
        path: "/api/plan-search",
        summary: "Start a multi-constraint destination search. Body: origin, exactly one of \
                  month (YYYY-MM) or date_window {from,to} (span capped at 42 days), optional \
                  min_days, budget_cents, currency, modes[], interests, max_candidates \
                  (default 8, clamped 1-20). Answers 202 with a job number; 503 when ten \
                  searches already run. A month search with no calendar FAILS rather than \
                  ranking every day as free.",
        request_schema: Some(route_manifest::schema_of::<trips::plan_search::PlanSearchRequest>),
    },
    r(
        "GET",
        "/api/plan-search/:id",
        "One search's state: running (with since_ms), failed (with error), or done with the \
         ranked result — reach per source, degraded[], considered/priced/unpriced and the \
         candidates with their visible score factors. 404 once the job is evicted.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/plan-search/:id/adopt",
        summary: "Record a finished search as one option_set item on an existing plan. \
                  Body: {plan_id}. 400 for an archived plan, 404 for an expired job. \
                  Money is integer minor units; the payload carries no companion field.",
        request_schema: Some(route_manifest::schema_of::<AdoptRequest>),
    },
    r(
        "GET",
        "/api/plans/:id/pack",
        "Pack lists for a plan, with what is still missing computed server-side. Optional \
         ?stage=<destination place id> adds missing_for_stage over that leg's list plus \
         every whole-trip list. interior_reachable distinguishes an unreachable inventory \
         from a deleted item.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/plans/:id/pack",
        summary: "Create a pack list. Body: {name, stage_destination_id?, stage_sequence?, \
                  template_key?}. The binding is the stage's DESTINATION place id, because \
                  a stage id is a pure function of position.",
        request_schema: Some(route_manifest::schema_of::<trips::pack::CreatePackList>),
    },
    r(
        "DELETE",
        "/api/plans/:id/pack/:list_id",
        "Delete one pack list and its items.",
    ),
    route_manifest::Route {
        method: "PUT",
        path: "/api/plans/:id/pack/:list_id/items",
        summary: "Replace a pack list's items. Body: {items:[{item_ref, packed, note?}]}. \
                  item_ref holds an interior_item.id as a soft reference with no foreign key.",
        request_schema: Some(route_manifest::schema_of::<trips::pack::PutPackItems>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/intent/draft",
        summary: "Turn a natural language sentence into a CreatePlan draft body using the local \
                  foundation model if available, falling back to heuristic parsing if offline. \
                  Body: { sentence }.",
        request_schema: Some(route_manifest::schema_of::<trips::intent::IntentDraftRequest>),
    },
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
    Json(route_manifest::manifest("trips", ROUTES))
}

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
    obsidian: Option<ObsidianConfig>,
    travel: Arc<trips::config::TravelPrefs>,
    /// Held across one export so two concurrent writes cannot interleave inside one
    /// projected file. `std::fs::write` is not atomic, and a half-written safety copy
    /// is the one state this whole mechanism exists to prevent.
    export_lock: Arc<tokio::sync::Mutex<()>>,
}

/// Re-export every plan to the vault after any successful write.
///
/// A layer rather than a line at the end of nine handlers. Nine call sites would put
/// the rule "a plan write is projected" in nine places, and the tenth mutation route
/// somebody adds later would silently stop projecting — which is a data-loss bug that
/// looks like nothing at all. Here the rule is one sentence in one place, and it
/// covers routes that do not exist yet.
///
/// Non-GET plus a 2xx status is the test (`completes_a_write`), less 202 and less a
/// response a handler marked `NotAWrite`. It exports on other writes that touch no
/// plan, and that costs a listing and thirteen unchanged-file comparisons — cheaper
/// than a second definition of which routes mutate.
///
/// After the response, never before it: the export reads the store, so running it
/// first would project the state the write is about to replace.
///
/// A failure is logged and the request still succeeds. Refusing a plan write because
/// the vault is unreachable would trade a durable row for a missing file, which is the
/// wrong way round; `trips export-vault` is the repair, and it is a command a human
/// can run when this process is not even up.
async fn project_after_write(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = request.method().clone();
    let response = next.run(request).await;
    if !completes_a_write(&method, &response) {
        return response;
    }
    let Some(vault) = state.obsidian.clone() else {
        return response;
    };
    let database_path = state.database_path.clone();
    let lock = state.export_lock.clone();
    tokio::spawn(async move {
        let _held = lock.lock().await;
        let outcome = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let root = markdown_root::MarkdownRoot::declare(vault.root.clone())
                .map_err(|e| e.to_string())?;
            let store = TripsStore::open(&database_path).map_err(|e| e.to_string())?;
            let plans = store.list_every_plan().map_err(|e| e.to_string())?;
            trips::projection::export_all(&root, &plans).map_err(|e| e.to_string())
        })
        .await;
        match outcome {
            Ok(Ok(report)) => {
                for path in &report.refused {
                    eprintln!("trips: vault projection refused, a human owns {path}");
                }
            }
            Ok(Err(error)) => eprintln!(
                "trips: vault projection failed ({error}); the rows are safe, the copy is stale — run `trips export-vault`"
            ),
            Err(error) => eprintln!("trips: vault projection task failed ({error})"),
        }
    });
    response
}

/// Marks a non-GET response that wrote nothing, so `project_after_write` skips it.
///
/// A handler opts OUT, rather than every writing handler opting in, so a new
/// mutation route still projects by default. `POST /api/intent/draft` is the one
/// user: it answers 200 with a draft nobody has submitted, and without this every
/// draft cost a full vault projection.
#[derive(Debug, Clone, Copy)]
struct NotAWrite;

/// Whether a finished request completed a write that the vault copy must follow.
///
/// 202 is excluded because it means no write has COMPLETED yet -- a general
/// property of the status code, not a per-route list, so this does not
/// reintroduce the second definition of "which routes mutate" the note on
/// `project_after_write` warns against. `POST /api/plan-search` answers 202 and
/// would otherwise cost a listing and thirteen file comparisons per search.
fn completes_a_write(method: &axum::http::Method, response: &axum::response::Response) -> bool {
    *method != axum::http::Method::GET
        && response.status().is_success()
        && response.status() != StatusCode::ACCEPTED
        && response.extensions().get::<NotAWrite>().is_none()
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

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "capability": "trips"
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
        TripsStore::open(&database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => response(StatusCode::OK, json!({ "ok": true, "capability": "trips" })),
        // 503, not 500: the request was fine, the dependency is not, and a caller that retries
        // should be told to come back rather than to fix its input.
        Ok(Err(error)) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "trips", "error": error }),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "trips", "error": "readiness check failed" }),
        ),
    }
}

async fn list_plans(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.list_plans())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(plans)) => response(StatusCode::OK, plans),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_plan(State(state): State<AppState>, Json(input): Json<CreatePlan>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.create_plan(&input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(plan)) => response(StatusCode::CREATED, plan),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdatePlan>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.update_plan(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(plan))) => response(StatusCode::OK, plan),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => write_conflict_or_bad_request(error),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// A stale revision is 409, not 400: the request was well-formed and the caller
/// should re-read and retry, which is a different instruction from "fix your
/// input". `code` is there so a caller can branch without parsing prose.
fn write_conflict_or_bad_request(error: String) -> ApiResponse {
    if error.starts_with("stale_plan:") {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": error, "code": "stale_plan" }),
        );
    }
    response(StatusCode::BAD_REQUEST, json!({ "error": error }))
}

#[derive(serde::Deserialize)]
struct SetDay {
    /// Explicit null clears the day; omitting the field is the same request with
    /// no instruction in it, so it is rejected rather than guessed at.
    day: Option<String>,
}

async fn set_item_day(
    State(state): State<AppState>,
    Path((plan_id, item_id)): Path<(String, String)>,
    Json(input): Json<SetDay>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.set_item_day(&plan_id, &item_id, input.day.as_deref()))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(item))) => response(StatusCode::OK, item),
        Ok(Ok(None)) => response(StatusCode::NOT_FOUND, json!({ "error": "item not found" })),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct OutcomeBody {
    stage_id: String,
    /// Whatever was actually observed. Left open on purpose: nobody has filled
    /// one of these in yet, so fixing a shape now would be designing an analysis
    /// before there is anything to analyse. The fields that matter get promoted
    /// to a declared variant once two real trips show which ones they are.
    #[serde(flatten)]
    outcome: serde_json::Map<String, Value>,
}

async fn record_outcome(
    State(state): State<AppState>,
    Path(plan_id): Path<String>,
    Json(body): Json<OutcomeBody>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| {
                store.record_outcome(&plan_id, &body.stage_id, &Value::Object(body.outcome))
            })
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(item)) => response(StatusCode::CREATED, item),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// Exactly the three fields PRD §8.2 rules, and `deny_unknown_fields` so a
/// fourth cannot arrive by accident.
///
/// No `currency`: `cost_cents` is denominated in the plan's own currency, so the
/// wire body, the table and the form are all the same three fields.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RetrospectiveBody {
    #[serde(default)]
    cost_cents: Option<i64>,
    again: String,
    #[serde(default)]
    change_note: String,
}

async fn record_retrospective(
    State(state): State<AppState>,
    Path(plan_id): Path<String>,
    Json(body): Json<RetrospectiveBody>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| {
                store.put_retrospective(
                    &plan_id,
                    body.cost_cents,
                    body.again.trim(),
                    &body.change_note,
                )
            })
            .map_err(|error| error.to_string())
    })
    .await
    {
        // A second write corrects the first, so it is 200 rather than 201.
        Ok(Ok(Some((row, created)))) => response(
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            row,
        ),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// The cost roll-up: a computed read that stores nothing.
///
/// The finance call happens inside the same `spawn_blocking` as the store read,
/// because both are blocking and neither may run on the async runtime. Its
/// failure is a value, not a `?`: `cost::roll_up` takes the `Result` and turns it
/// into four nulls and a reason, which is why no handler can quietly
/// `unwrap_or_default()` it into zeroes.
async fn plan_cost(State(state): State<AppState>, Path(plan_id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Option<trips::cost::CostRollup>, String> {
        let store = TripsStore::open(&database_path).map_err(|e| e.to_string())?;
        let Some(details) = store.get_plan(&plan_id).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let spending = trips::finance_client::trip_spending(&plan_id);
        Ok(Some(trips::cost::roll_up(&details, spending)))
    })
    .await
    {
        Ok(Ok(Some(rollup))) => response(StatusCode::OK, rollup),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn pending_retrospectives(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    // `windows::today()`, not a hand-rolled conversion: `day_number`'s epoch is
    // proleptic year 0, so a raw Unix day count reads as a date in the first
    // century and the window silently matches nothing.
    let today = trips::windows::today();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.pending_retrospectives(&today))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(pending)) => response(
            StatusCode::OK,
            json!({
                "pending": pending,
                "window_days": trips::store::RETROSPECTIVE_WINDOW_DAYS,
            }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn retrospective_summary(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_path).map_err(|e| e.to_string())?;
        // Every plan, archived ones included: a finished trip is exactly the one
        // a retrospective is written about.
        let plans: Vec<TripPlan> = store
            .list_every_plan()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|details| details.plan)
            .collect();
        let rows = store.retrospectives().map_err(|e| e.to_string())?;
        Ok::<_, String>(trips::retrospective::summary(&rows, &plans))
    })
    .await
    {
        Ok(Ok(summary)) => response(StatusCode::OK, summary),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_places(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.list_places())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(places)) => response(StatusCode::OK, places),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn get_plan(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.get_plan(&id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(plan))) => response(StatusCode::OK, plan),
        Ok(Ok(None)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct DeleteQuery {
    /// Same guard as PATCH, and in the same commit deliberately: a stale delete
    /// is worse than a stale patch, because there is nothing left to re-apply.
    expected_updated_at: Option<String>,
}

async fn delete_plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        if let Some(expected) = params.expected_updated_at.as_deref() {
            match store.get_plan(&id).map_err(|error| error.to_string())? {
                None => return Ok(false),
                Some(details) if details.plan.updated_at != expected => {
                    return Err(format!(
                        "stale_plan: expected_updated_at {expected} but the plan is at {}; \
                         re-read it before deleting",
                        details.plan.updated_at
                    ));
                }
                Some(_) => {}
            }
        }
        store.delete_plan(&id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::NO_CONTENT, Value::Null),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "trip plan not found" }),
        ),
        Ok(Err(error)) => write_conflict_or_bad_request(error),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<CreatePlanItem>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.add_item(&id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(item)) => response(StatusCode::CREATED, item),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_item(
    State(state): State<AppState>,
    Path((plan_id, item_id)): Path<(String, String)>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| store.delete_item(&plan_id, &item_id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::NO_CONTENT, Value::Null),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "itinerary item not found" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn scan_obsidian(State(state): State<AppState>) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        let mut candidates = scan_trip_notes(&obsidian.root, &obsidian.trips_dir)
            .map_err(|error| error.to_string())?;
        for candidate in &mut candidates {
            candidate.imported_plan_id = store
                .find_plan_by_source("obsidian", &candidate.reference)
                .map_err(|error| error.to_string())?
                .map(|plan| plan.id);
        }
        Ok::<_, String>(candidates)
    })
    .await
    {
        Ok(Ok(candidates)) => response(StatusCode::OK, candidates),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ImportObsidianTrip {
    reference: String,
    origin: PlaceRef,
}

enum ImportOutcome {
    Imported(TripPlan),
    Existing(TripPlan),
}

impl ImportOutcome {
    fn plan(self) -> TripPlan {
        match self {
            Self::Imported(plan) | Self::Existing(plan) => plan,
        }
    }
}

#[derive(Serialize)]
struct SkippedObsidianTrip {
    reference: String,
    title: String,
    issues: Vec<String>,
}

#[derive(Serialize)]
struct ImportAllObsidianResult {
    imported: Vec<TripPlan>,
    existing: Vec<TripPlan>,
    skipped: Vec<SkippedObsidianTrip>,
}

fn import_obsidian_candidate(
    store: &TripsStore,
    candidate: ObsidianTripCandidate,
    origin: PlaceRef,
) -> Result<ImportOutcome, String> {
    if let Some(existing) = store
        .find_plan_by_source("obsidian", &candidate.reference)
        .map_err(|error| error.to_string())?
    {
        return Ok(ImportOutcome::Existing(existing));
    }
    if !candidate.issues.is_empty() {
        return Err(candidate.issues.join(" · "));
    }
    let destination = candidate
        .destination
        .ok_or_else(|| "Destination is missing".to_owned())?;
    let date_start = candidate
        .date_start
        .ok_or_else(|| "Start date is missing".to_owned())?;
    let date_end = candidate
        .date_end
        .ok_or_else(|| "End date is missing".to_owned())?;
    let cover_image_url = candidate
        .cover
        .as_ref()
        .filter(|value| value.starts_with("https://"))
        .cloned();
    let plan = store
        .create_plan(&CreatePlan {
            title: candidate.title.clone(),
            origin,
            destinations: vec![destination],
            date_start,
            date_end,
            interests: candidate.summary.clone(),
            travelers: candidate.travelers,
            transport_modes: candidate.transport_modes,
            stages: Vec::new(),
            cover_image_url,
            source: Some(PlanSource {
                kind: "obsidian".into(),
                reference: candidate.reference.clone(),
            }),
        })
        .map_err(|error| error.to_string())?;
    store
        .add_item(
            &plan.id,
            &CreatePlanItem {
                item_type: "note".into(),
                day: None,
                external_id: candidate.reference.clone(),
                title: "Obsidian-Reisenotiz".into(),
                payload: json!({
                    "vault_path": candidate.reference,
                    "summary": candidate.summary,
                    "status": candidate.status,
                    "cover": candidate.cover,
                }),
            },
        )
        .map_err(|error| error.to_string())?;
    let imported = store
        .get_plan(&plan.id)
        .map_err(|error| error.to_string())?
        .map(|details| details.plan)
        .ok_or_else(|| "imported trip could not be reloaded".to_owned())?;
    Ok(ImportOutcome::Imported(imported))
}

async fn import_obsidian(
    State(state): State<AppState>,
    Json(input): Json<ImportObsidianTrip>,
) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        let candidate =
            read_trip_note(&obsidian.root, &input.reference).map_err(|error| error.to_string())?;
        import_obsidian_candidate(&store, candidate, input.origin).map(ImportOutcome::plan)
    })
    .await
    {
        Ok(Ok(plan)) => response(StatusCode::CREATED, plan),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn import_all_obsidian(
    State(state): State<AppState>,
    Json(input): Json<ImportAllObsidianTrips>,
) -> ApiResponse {
    let Some(obsidian) = state.obsidian.clone() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Obsidian import is not configured for this machine" }),
        );
    };
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        let candidates = scan_trip_notes(&obsidian.root, &obsidian.trips_dir)
            .map_err(|error| error.to_string())?;
        let mut result = ImportAllObsidianResult {
            imported: Vec::new(),
            existing: Vec::new(),
            skipped: Vec::new(),
        };
        for candidate in candidates {
            let reference = candidate.reference.clone();
            let title = candidate.title.clone();
            match import_obsidian_candidate(&store, candidate, input.origin.clone()) {
                Ok(ImportOutcome::Imported(plan)) => result.imported.push(plan),
                Ok(ImportOutcome::Existing(plan)) => result.existing.push(plan),
                Err(error) => result.skipped.push(SkippedObsidianTrip {
                    reference,
                    title,
                    issues: vec![error],
                }),
            }
        }
        Ok::<_, String>(result)
    })
    .await
    {
        Ok(Ok(result)) => response(StatusCode::OK, result),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct ImportAllObsidianTrips {
    origin: PlaceRef,
}

#[derive(serde::Deserialize)]
struct FlightSearchParams {
    from: String,
    to: String,
    date: String,
    #[serde(default)]
    flex_days: u8,
    #[serde(default)]
    return_date: Option<String>,
}

/// One live Kiwi search per call, on a blocking thread because the client is
/// deliberately synchronous (and self-paced) like the bahn.de one in transit.
/// Upstream failure is a 502 carrying the error text: this route proxies a
/// third party and must never dress its outage as an empty result.
async fn search_flights(Query(params): Query<FlightSearchParams>) -> ApiResponse {
    match tokio::task::spawn_blocking(move || {
        KiwiClient::new().search(
            &params.from,
            &params.to,
            &params.date,
            params.flex_days,
            params.return_date.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(result)) => response(StatusCode::OK, result),
        Ok(Err(error)) => response(
            StatusCode::BAD_GATEWAY,
            json!({ "error": error.to_string() }),
        ),
        Err(join_error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": join_error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct FlightGridParams {
    from: String,
    to: String,
    date: String,
    flex_days: Option<u8>,
}

/// The flexible-date money question: one widened search, reduced to
/// cheapest-per-day. Defaults to +/-3 days because that alone moved a
/// measured fare 23%.
async fn flight_grid(Query(params): Query<FlightGridParams>) -> ApiResponse {
    let flex = params.flex_days.unwrap_or(3).min(10);
    match tokio::task::spawn_blocking(move || {
        KiwiClient::new()
            .search(&params.from, &params.to, &params.date, flex, None)
            .map(|result| {
                let days = trips::kiwi::cheapest_per_day(&result);
                json!({
                    "currency": result.currency,
                    "flex_days": flex,
                    "days": days,
                })
            })
    })
    .await
    {
        Ok(Ok(grid)) => response(StatusCode::OK, grid),
        Ok(Err(error)) => response(
            StatusCode::BAD_GATEWAY,
            json!({ "error": error.to_string() }),
        ),
        Err(join_error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": join_error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct FlightWhenParams {
    from: String,
    to: String,
    date_from: String,
    date_to: String,
}

/// Where calendar-server listens. Mirrors punctuality.rs's rationale in
/// transit verbatim -- and this is now the SECOND capability hardcoding a
/// sibling's port, so the spine mechanism that comment deferred (service-runner
/// exporting declared siblings' ports) is justified and tracked as follow-up.
fn calendar_base_url() -> String {
    axon_config::env_var("SJEL_CALENDAR_URL").unwrap_or_else(|_| "http://127.0.0.1:8087".to_string())
}

/// Calendar's `GET /api/entries` for a day range, as `flight_when` asks for it.
///
/// Its own function so a test can read the string that actually leaves this
/// process. Both dates must already have been through [`clipped_span`].
fn calendar_entries_url(from: &str, to: &str) -> String {
    format!("{}/api/entries?from={from}&to={to}", calendar_base_url())
}

/// A day range `flight_when` will search: both dates on the day-number scale, and
/// both re-rendered from the numbers they parsed as.
#[derive(Debug)]
struct DaySpan {
    from_day: i64,
    to_day: i64,
    from: String,
    to: String,
}

/// Parse the two query dates, bound the span, and clip both to `YYYY-MM-DD`.
/// `Err` carries the sentence the route answers 400 with.
///
/// The clip is the part that is not cosmetic. `day_number` checks the SHAPE of a
/// date and stops at the day field — `libs/civil-date/src/lib.rs` says so: "The
/// day field is read two characters wide, so a trailing wall time
/// (`2026-08-08T10:00`) parses as its date rather than failing", which `places`
/// depends on. So `date_from=2026-01-01&limit=99999` passes every check here, and
/// the raw string used to go on into [`calendar_entries_url`]'s query string and
/// into the grid filter that compares it against a day. `iso_of_day_number` is
/// `day_number`'s documented inverse and builds its string from integers, so only
/// digits and hyphens come out.
///
/// CodeQL rust/request-forgery, alert 67 — and alert 40, the same line before the
/// file grew, dismissed on the grounds that the base URL is `SJEL_CALENDAR_URL`.
/// The base is. The two dates beside it are request data.
fn clipped_span(date_from: &str, date_to: &str) -> Result<DaySpan, &'static str> {
    let from_day = trips::windows::day_number(date_from).ok_or("date_from is not ISO")?;
    let to_day = trips::windows::day_number(date_to).ok_or("date_to is not ISO")?;
    if to_day < from_day || to_day - from_day > 42 {
        return Err("span must be 0-42 days, date_from first");
    }
    Ok(DaySpan {
        from_day,
        to_day,
        from: trips::windows::iso_of_day_number(from_day),
        to: trips::windows::iso_of_day_number(to_day),
    })
}

/// The fuzzy-timeframe answer: every day of the span priced via the grid,
/// joined with committed calendar time, ranked free-cheapest-first.
///
/// A calendar that cannot be reached no longer degrades to all-free. It used
/// to: `.ok()/.and_then/.unwrap_or_default()` fed an EMPTY entry list into
/// `day_loads`, which starts every day at `DayLoad::Free`, so a fully committed
/// week ranked cheapest-first and nothing in the body said the calendar had
/// never answered. The old doc comment called that deliberate and cited
/// punctuality enrichment as the precedent; the precedent does not hold,
/// because transit reports its gap (`unscored_legs`) and this did not. Now the
/// days come back `DayLoad::Unknown`, the body carries `calendar` and
/// `degraded`, and the ranking says what it does not know
/// (Packs/travel/ISA.md: "a guess that looks like a measurement is worse than
/// a blank").
async fn flight_when(Query(params): Query<FlightWhenParams>) -> ApiResponse {
    let span = match clipped_span(&params.date_from, &params.date_to) {
        Ok(span) => span,
        Err(error) => return response(StatusCode::BAD_REQUEST, json!({ "error": error })),
    };
    let DaySpan {
        from_day,
        to_day,
        from: date_from,
        to: date_to,
    } = span;
    let result = tokio::task::spawn_blocking(move || {
        let client = KiwiClient::new();
        let mut grid: Vec<trips::kiwi::GridDay> = Vec::new();
        for center in trips::windows::window_centers(from_day, to_day, 10) {
            let center_iso = trips::windows::iso_of_day_number(center);
            let search = client.search(&params.from, &params.to, &center_iso, 10, None)?;
            grid.extend(
                trips::kiwi::cheapest_per_day(&search)
                    .into_iter()
                    .filter(|d| d.date >= date_from && d.date <= date_to),
            );
        }
        // Two windows can overlap at the seam; keep the cheaper day.
        grid.sort_by(|a, b| a.date.cmp(&b.date).then(a.price.total_cmp(&b.price)));
        grid.dedup_by(|later, earlier| later.date == earlier.date);

        let entries: Result<Vec<trips::windows::CalendarSpan>, String> = (|| {
            let client = axon_http::client(
                axon_http::Purpose::new("trips-calendar"),
                std::time::Duration::from_secs(3),
            )
            .map_err(|error| error.to_string())?;
            let response = client
                .get(calendar_entries_url(&date_from, &date_to))
                .send()
                .map_err(|_| "calendar is not answering".to_string())?;
            if !response.status().is_success() {
                return Err(format!("calendar answered {}", response.status().as_u16()));
            }
            response
                .json()
                .map_err(|_| "unreadable calendar reply".to_string())
        })();
        let (loads, calendar) = match &entries {
            Ok(entries) => (
                trips::windows::day_loads(&date_from, &date_to, entries),
                Ok(()),
            ),
            Err(reason) => (
                trips::windows::unknown_loads(&date_from, &date_to),
                Err(reason.clone()),
            ),
        };
        Ok::<_, trips::kiwi::KiwiError>((trips::windows::rank(grid, &loads), calendar))
    })
    .await;

    match result {
        Ok(Ok((days, calendar))) => response(
            StatusCode::OK,
            json!({
                "days": days,
                "calendar": match &calendar {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => format!("error: {reason}"),
                },
                "degraded": if calendar.is_ok() {
                    Vec::<String>::new()
                } else {
                    vec!["calendar".to_string()]
                },
            }),
        ),
        Ok(Err(error)) => response(
            StatusCode::BAD_GATEWAY,
            json!({ "error": error.to_string() }),
        ),
        Err(join_error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": join_error.to_string() }),
        ),
    }
}

#[derive(serde::Deserialize)]
struct FlightPivotParams {
    to: String,
    date: String,
    #[serde(default)]
    from: Option<String>,
}

/// The Mallorca move (PRD F4): fly to a city where a friend's couch is free,
/// stay a night or two, fly on. Enumerated over the configured pivot graph
/// because that graph is the one thing no commercial engine knows. Not
/// general virtual interlining -- rebuilding Kiwi loses; a handful of pivots
/// times a couple of offsets is bounded and personal.
async fn flight_pivot(
    State(state): State<AppState>,
    Query(params): Query<FlightPivotParams>,
) -> ApiResponse {
    let travel = state.travel.clone();
    // The origin is the traveller's, so it comes from the profile. This used to
    // read `travel.home_airport` from this capability's own overlay config, which
    // made it the second home for one fact and left the airports the operator
    // named invisible to every flight route.
    let Some(from) = params
        .from
        .or_else(trips::traveler_client::first_home_airport)
    else {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"error": "no origin: pass from=, or set hard.home_airports on the profile"}),
        );
    };
    if travel.pivots.is_empty() {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"error": "no pivots configured: add travel.pivots to the overlay's trips.json"}),
        );
    }
    let Some(date_number) = trips::windows::day_number(&params.date) else {
        return response(StatusCode::BAD_REQUEST, json!({"error": "date is not ISO"}));
    };

    let to = params.to.clone();
    let date = params.date.clone();
    let result = tokio::task::spawn_blocking(move || {
        let client = KiwiClient::new();
        let cheapest = |r: &trips::kiwi::FlightSearchResult| {
            r.options
                .iter()
                .min_by(|a, b| a.price.total_cmp(&b.price))
                .cloned()
        };
        let direct = client
            .search(&from, &to, &date, 0, None)
            .ok()
            .and_then(|r| cheapest(&r));

        const PIVOT_CAP: usize = 4;
        let skipped = travel.pivots.len().saturating_sub(PIVOT_CAP);
        let mut options = Vec::new();
        for pivot in travel.pivots.iter().take(PIVOT_CAP) {
            let Ok(leg_in) = client.search(&from, &pivot.iata, &date, 0, None) else {
                continue;
            };
            let Some(leg_in) = cheapest(&leg_in) else { continue };
            for nights in 1..=pivot.max_nights.max(1) {
                let onward_date = trips::windows::iso_of_day_number(date_number + i64::from(nights));
                let Ok(leg_out) = client.search(&pivot.iata, &to, &onward_date, 0, None) else {
                    continue;
                };
                let Some(leg_out) = cheapest(&leg_out) else { continue };
                let total = leg_in.price + leg_out.price;
                options.push(json!({
                    "pivot": { "name": pivot.name, "iata": pivot.iata },
                    "nights_at_pivot": nights,
                    "total_price": total,
                    "savings_vs_direct": direct.as_ref().map(|d| d.price - total),
                    "separate_tickets": true,
                    "note": "two contracts, no through-protection; a delayed first leg is your own risk",
                    "legs": [leg_in, leg_out.clone()],
                }));
            }
        }
        options.sort_by(|a, b| {
            let pa = a["total_price"].as_f64().unwrap_or(f64::MAX);
            let pb = b["total_price"].as_f64().unwrap_or(f64::MAX);
            pa.total_cmp(&pb)
        });
        json!({
            "direct": direct,
            "options": options,
            "pivots_skipped_over_cap": skipped,
        })
    })
    .await;

    match result {
        Ok(body) => response(StatusCode::OK, body),
        Err(join_error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": join_error.to_string() }),
        ),
    }
}

// ---- plan search (PRD 8.2: "October, under 300 euro, by train") -----------

/// Start a search and answer with its number, not its result.
///
/// The composition copies `flight_when`'s shape — bound the window in the
/// handler, `spawn_blocking`, one timed HTTP call per capability — and
/// overturns its stated all-free calendar contract: `plan_search::compose`
/// reports what it reached instead of assuming every day is free.
async fn plan_search_start(
    State(state): State<AppState>,
    Json(body): Json<trips::plan_search::PlanSearchRequest>,
) -> ApiResponse {
    let request = match trips::plan_search::validate(body) {
        Ok(request) => request,
        Err(error) => return response(StatusCode::BAD_REQUEST, json!({ "error": error })),
    };
    let database_path = state.database_path.clone();
    let started = trips::jobs::start(move |started| {
        // Trips' own plan destinations are a candidate source and a local read,
        // so a database failure fails the job rather than degrading silently.
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        let plan_destinations = store
            .list_plans()
            .map_err(|error| error.to_string())?
            .into_iter()
            .flat_map(|plan| plan.destinations)
            .map(|destination| trips::plan_search::DestinationCandidate {
                place_id: destination.id.clone(),
                destination,
                sources: vec!["plan".into()],
            })
            .collect();
        // The same rule covers retrospectives: they are a local read, so a
        // failure fails the job instead of ranking without the operator's history.
        let plans: Vec<_> = store
            .list_every_plan()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|detail| detail.plan)
            .collect();
        let rows = store.retrospectives().map_err(|error| error.to_string())?;
        let retro_by_dest = trips::retrospective::summary(&rows, &plans)
            .by_destination
            .into_iter()
            .map(|df| (df.key.clone(), df))
            .collect();
        let sources = trips::upstream::HttpSources::new(started).with_retrospectives(retro_by_dest);
        trips::plan_search::compose(&request, plan_destinations, &sources)
    });
    match started {
        Ok(job) => response(StatusCode::ACCEPTED, json!({ "job": job })),
        // Interior's sentence: ten running searches answer a refusal rather
        // than dropping one silently.
        Err(error) => response(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": error })),
    }
}

/// `POST /api/bases` — where to base yourself between two fixed points.
///
/// Synchronous, unlike `plan-search`. That one needs a job because eight
/// candidates at up to twenty seconds of fare search each is a two-hundred-second
/// worst case; this prices two legs per candidate and a fare search answers in
/// well under a second warm, so ten candidates is about ten seconds. A second copy
/// of the job machinery for a tenth of the budget would be machinery for its own
/// sake.
async fn base_search(Json(body): Json<trips::bases::BaseRequest>) -> ApiResponse {
    let outcome = tokio::task::spawn_blocking(move || {
        let sources = trips::upstream::HttpSources::new(std::time::Instant::now());
        trips::bases::rank_bases(&body, &sources).map_err(|error| error.to_string())
    })
    .await;

    match outcome {
        Ok(Ok(result)) => response(
            StatusCode::OK,
            serde_json::to_value(result).unwrap_or_default(),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// `{id, state, since_ms | result | error}` — the job's tag is flattened beside
/// its number so a reader switches on one field.
#[derive(Serialize)]
struct JobBody {
    id: u64,
    #[serde(flatten)]
    state: trips::jobs::JobState,
}

async fn plan_search_status(Path(id): Path<u64>) -> ApiResponse {
    match trips::jobs::read(id) {
        Some(state) => response(StatusCode::OK, JobBody { id, state }),
        None => response(StatusCode::NOT_FOUND, json!({ "error": SEARCH_EXPIRED })),
    }
}

/// What a caller is told when the job number is unknown or already evicted.
/// Never an empty option set, which would read as "the search found nothing".
const SEARCH_EXPIRED: &str = "that search result has expired — run the search again";

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct AdoptRequest {
    plan_id: String,
}

/// Write the whole option space onto a plan the operator already created.
///
/// Machine proposes, human confirms: creating the plan stays the explicit
/// `POST /api/plans`. The refusal for an archived plan happens here rather than
/// inside `add_item`, whose unconditional `status = 'saved'` would un-archive
/// the plan as a side effect of recording an option set.
async fn plan_search_adopt(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<AdoptRequest>,
) -> ApiResponse {
    let Some(trips::jobs::JobState::Done { result }) = trips::jobs::read(id) else {
        return response(StatusCode::NOT_FOUND, json!({ "error": SEARCH_EXPIRED }));
    };
    let database_path = state.database_path.clone();
    let outcome = tokio::task::spawn_blocking(move || -> Result<_, (StatusCode, String)> {
        let store = TripsStore::open(&database_path)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        let plan = store
            .get_plan(&body.plan_id)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, format!("no plan {}", body.plan_id)))?;
        if plan.plan.status == "archived" {
            return Err((
                StatusCode::BAD_REQUEST,
                "that plan is archived — un-archive it before adopting a search".into(),
            ));
        }
        let item = CreatePlanItem {
            item_type: "option_set".into(),
            day: None,
            external_id: trips::plan_search::adopt_external_id(&result.observed_at, id),
            title: format!(
                "Plan search: {} option(s), {} priced",
                result.candidates.len(),
                result.priced
            ),
            payload: trips::plan_search::adopt_payload(&result),
        };
        store
            .add_item(&body.plan_id, &item)
            .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))
    })
    .await;
    match outcome {
        Ok(Ok(item)) => response(StatusCode::CREATED, item),
        Ok(Err((status, error))) => response(status, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

// ---- pack lists ----------------------------------------------------------

#[derive(serde::Deserialize)]
struct PackQuery {
    /// The `PlaceRef.id` of a stage destination. With it, `missing_for_stage`
    /// covers that leg's list plus every whole-trip list.
    #[serde(default)]
    stage: Option<String>,
}

/// Where interior serves its inventory. Same hardcoded-sibling-port shape, and
/// the same caveat, as `calendar_base_url` above.
fn interior_base_url() -> String {
    axon_config::env_var("SJEL_INTERIOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8092".to_string())
}

/// `item_ref` -> the item, or `None` when interior could not be reached.
///
/// `None` is a third state, not an empty index: "interior is down" and "the
/// item was deleted" would otherwise look identical, and only one of them is
/// something the operator should act on.
///
/// Every field below is read with a fallback rather than a `?`: a deployment whose interior
/// predates B51 answers without the seven columns, and a pack list that refuses to render
/// there would be a worse answer than one that renders labels and says the attributes are
/// absent.
fn interior_index() -> Option<std::collections::HashMap<String, trips::pack::InventoryItem>> {
    let body: Value = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?
        .get(format!("{}/api/inventory", interior_base_url()))
        .send()
        .ok()?
        .json()
        .ok()?;
    Some(
        body.as_array()?
            .iter()
            .filter_map(|row| {
                let item = row.get("item")?;
                let flag = |key: &str| item[key].as_bool();
                Some((
                    item["id"].as_str()?.to_string(),
                    trips::pack::InventoryItem {
                        label: item["label"].as_str().unwrap_or_default().to_string(),
                        weight_g: item["weight_g"].as_i64(),
                        category: item["category"].as_str().map(str::to_string),
                        pack_location: item["pack_location"].as_str().map(str::to_string),
                        packable: flag("packable"),
                        waterproof: flag("waterproof"),
                        quick_dry: flag("quick_dry"),
                        trip_types: item["trip_types"]
                            .as_array()
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(|value| value.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    },
                ))
            })
            .collect(),
    )
}

async fn list_pack(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PackQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = TripsStore::open(&database_path).map_err(|error| error.to_string())?;
        let plan = store
            .get_plan(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no plan {id}"))?;
        let lists = trips::pack::lists_for_plan(&store, &id).map_err(|error| error.to_string())?;
        let index = interior_index();
        Ok(trips::pack::render(
            &lists,
            &plan.plan.stages,
            index.as_ref(),
            query.stage.as_deref(),
        ))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::NOT_FOUND, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_pack_list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<trips::pack::CreatePackList>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| trips::pack::create_list(&store, &id, &input))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(list)) => response(
            StatusCode::CREATED,
            json!({
                "id": list.id,
                "name": list.name,
                "stage_destination_id": list.stage_destination_id,
                "stage_sequence": list.stage_sequence,
                "template_key": list.template_key,
            }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn delete_pack_list(
    State(state): State<AppState>,
    Path((id, list_id)): Path<(String, String)>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| trips::pack::delete_list(&store, &id, &list_id))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "ok": true })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "no pack list with that id on this plan" }),
        ),
        Ok(Err(error)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn put_pack_items(
    State(state): State<AppState>,
    Path((id, list_id)): Path<(String, String)>,
    Json(input): Json<trips::pack::PutPackItems>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let count = input.items.len();
    match tokio::task::spawn_blocking(move || {
        TripsStore::open(&database_path)
            .and_then(|store| trips::pack::replace_items(&store, &id, &list_id, &input.items))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(true)) => response(StatusCode::OK, json!({ "ok": true, "count": count })),
        Ok(Ok(false)) => response(
            StatusCode::NOT_FOUND,
            json!({ "error": "no pack list with that id on this plan" }),
        ),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

/// A draft persists nothing (`trips::intent`), so its 200 carries `NotAWrite`.
async fn draft_intent(
    Json(body): Json<trips::intent::IntentDraftRequest>,
) -> axum::response::Response {
    let sentence = body.sentence.clone();
    let answer = match tokio::task::spawn_blocking(move || {
        trips::intent::resolve_draft_or_heuristic(&sentence)
    })
    .await
    {
        Ok(Ok(draft)) => response(StatusCode::OK, draft),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(error) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    };
    not_a_write(answer)
}

fn not_a_write(answer: ApiResponse) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut response = answer.into_response();
    response.extensions_mut().insert(NotAWrite);
    response
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_TRIPS_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "trips";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`), so a route appended under it would silently
/// lose the refusal. `a_foreign_origin_cannot_read_a_plan_search_result` drives
/// this router to prove the ones registered today are covered.
///
/// Why trips refuses foreign origins at all, when it did not before: the
/// plan-search body carries the operator's feasible calendar windows and a
/// companion hint derived from the C2 register, and `CorsLayer::permissive()`
/// makes every route above it readable by any page open in the operator's
/// browser. That is the attack `places` refuses by hand for the same data
/// (`capabilities/places/src/server.rs`, the `build_router` note). Applying it
/// to the whole router rather than to the new routes alone also closes an
/// existing leak: `GET /api/flights/when` returns calendar entry titles in
/// `collisions` (`trips::windows::rank`). A request with no `Origin` passes, so
/// `capabilities/calendar`'s server-to-server POST into trips is unaffected.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/plans", get(list_plans).post(create_plan))
        .route(
            "/api/plans/:id",
            get(get_plan).patch(update_plan).delete(delete_plan),
        )
        .route("/api/plans/:id/items", post(add_item))
        .route(
            "/api/plans/:plan_id/items/:item_id",
            delete(delete_item).patch(set_item_day),
        )
        .route("/api/places", get(list_places))
        .route("/api/flights/search", get(search_flights))
        .route("/api/flights/grid", get(flight_grid))
        .route("/api/flights/when", get(flight_when))
        .route("/api/flights/pivot", get(flight_pivot))
        .route("/api/plans/:id/outcome", post(record_outcome))
        .route("/api/plans/:id/retrospective", post(record_retrospective))
        .route("/api/plans/:id/cost", get(plan_cost))
        .route("/api/retrospectives/pending", get(pending_retrospectives))
        .route("/api/retrospectives/summary", get(retrospective_summary))
        .route("/api/import/obsidian/scan", get(scan_obsidian))
        .route("/api/import/obsidian/all", post(import_all_obsidian))
        .route("/api/import/obsidian", post(import_obsidian))
        .route("/api/plan-search", post(plan_search_start))
        .route("/api/bases", post(base_search))
        .route("/api/plan-search/:id", get(plan_search_status))
        .route("/api/plan-search/:id/adopt", post(plan_search_adopt))
        .route("/api/plans/:id/pack", get(list_pack).post(create_pack_list))
        .route("/api/plans/:id/pack/:list_id", delete(delete_pack_list))
        .route("/api/plans/:id/pack/:list_id/items", put(put_pack_items))
        .route("/api/intent/draft", post(draft_intent))
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            project_after_write,
        ))
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            axon_server::origin::refuse_foreign_origins,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let config = Config::load();
    let state = AppState {
        database_path: Arc::new(config.database_path),
        obsidian: config.obsidian,
        travel: Arc::new(config.travel),
        export_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    // Loopback via axon_server; the old 0.0.0.0 bind here was never a
    // documented decision and is retired with it.
    axon_server::serve_local("trips-server", config.port, build_router(state)).await;
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    /// The contract the dashboard depends on: an unreachable database is reported as
    /// unavailable rather than as a healthy service (#126). Before the split, the only
    /// surface axon-status polled was `health`, which is a literal and answers 200 here.
    #[tokio::test]
    async fn readiness_fails_when_the_database_is_unreachable() {
        // A file where a directory has to be: the store cannot be opened there,
        // which is what a missing or unwritable database file reads as now.
        let blocker =
            std::env::temp_dir().join(format!("trips-ready-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        let state = AppState {
            database_path: Arc::new(blocker.join("axon.db")),
            obsidian: None,
            travel: Arc::new(Default::default()),
            export_lock: Arc::new(tokio::sync::Mutex::new(())),
        };

        let (status, Json(body)) = ready(State(state)).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "an unreachable database was reported as ready: {body}"
        );
        assert_eq!(body["ok"], false);

        // The control: liveness is deliberately unaffected, because the process is fine.
        let Json(live) = health().await;
        assert_eq!(live["ok"], true, "liveness must not depend on the database");
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;

    /// The plan-search body carries the operator's feasible windows and the
    /// companion hint, and `GET /api/flights/when` has been returning calendar
    /// entry titles cross-origin since it shipped. This drives the WIRED router,
    /// because a test of the predicate alone passes even when a route is
    /// registered below the guard layer (axum 0.7 `routing/layer.md`).
    #[tokio::test]
    async fn a_foreign_origin_cannot_read_a_plan_search_result() {
        let state = AppState {
            database_path: Arc::new(std::env::temp_dir().join("trips-origin-test.db")),
            obsidian: None,
            travel: Arc::new(Default::default()),
            export_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(state)).await;
        });
        let client = reqwest::Client::new();

        for path in [
            "/api/plan-search/1",
            "/api/flights/when?from=FRA&to=OSL&date_from=2026-10-01&date_to=2026-10-08",
            "/api/plans",
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

        // The control, and the receipt that calendar's server-to-server POST
        // into trips is unaffected: a request with no Origin passes the guard.
        let allowed = client
            .get(format!("{base}/api/plan-search/1"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            allowed.status(),
            404,
            "with no Origin the request must reach the handler, which has no job 1"
        );
    }
}

#[cfg(test)]
mod calendar_url_tests {
    use super::{calendar_entries_url, clipped_span};

    const HOSTILE: &str = "2026-01-01&to=2030-12-31&limit=99999";

    /// The premise first: `day_number` is the only thing `flight_when` checked a
    /// date against, and it accepts a date with anything glued to the end of it.
    /// That is deliberate in `libs/civil-date` — `places` needs `2026-08-08T10:00`
    /// to parse — so the clip belongs at the caller that puts the string in a URL.
    #[test]
    fn the_shape_check_accepts_a_date_with_a_query_string_glued_to_it() {
        assert!(trips::windows::day_number(HOSTILE).is_some());
    }

    /// CodeQL rust/request-forgery, alert 67 (and alert 40, the same line before
    /// the file grew). The URL carries one `from` and one `to` whatever the caller
    /// sent, because `clipped_span` re-renders both from their day numbers.
    #[test]
    fn a_hostile_date_reaches_the_calendar_url_clipped() {
        let span = clipped_span(HOSTILE, "2026-01-14").expect("the shape check passes");
        assert_eq!(span.from, "2026-01-01");
        assert_eq!(span.to, "2026-01-14");

        let url = calendar_entries_url(&span.from, &span.to);
        assert!(
            url.ends_with("/api/entries?from=2026-01-01&to=2026-01-14"),
            "unexpected URL: {url}"
        );
        assert_eq!(
            url.matches('&').count(),
            1,
            "one parameter separator, not three: {url}"
        );

        // The same URL built from the raw string, so the difference this fix makes
        // is visible rather than asserted about.
        let unclipped = calendar_entries_url(HOSTILE, "2026-01-14");
        assert_eq!(unclipped.matches('&').count(), 3, "{unclipped}");
    }

    /// The clip did not eat the three refusals it was folded in with.
    #[test]
    fn the_span_is_still_parsed_ordered_and_bounded() {
        assert_eq!(
            clipped_span("nonsense", "2026-01-14").unwrap_err(),
            "date_from is not ISO"
        );
        assert_eq!(
            clipped_span("2026-01-01", "nonsense").unwrap_err(),
            "date_to is not ISO"
        );
        assert_eq!(
            clipped_span("2026-01-14", "2026-01-01").unwrap_err(),
            "span must be 0-42 days, date_from first"
        );
        assert_eq!(
            clipped_span("2026-01-01", "2026-03-01").unwrap_err(),
            "span must be 0-42 days, date_from first"
        );
        let ok = clipped_span("2026-01-01", "2026-01-01").expect("a one-day span is a span");
        assert_eq!(ok.from_day, ok.to_day);
        assert_eq!(
            (ok.from.as_str(), ok.to.as_str()),
            ("2026-01-01", "2026-01-01")
        );
    }
}

#[cfg(test)]
mod projection_trigger_tests {
    use super::*;
    use axum::http::Method;
    use axum::response::IntoResponse;

    /// A draft answers 200 to a POST and writes nothing. Before the marker, every
    /// draft cost a full vault projection.
    #[test]
    fn a_draft_response_does_not_trigger_a_projection() {
        let draft = not_a_write(response(StatusCode::OK, json!({ "draft": {} })));
        assert_eq!(draft.status(), StatusCode::OK);
        assert!(!completes_a_write(&Method::POST, &draft));
    }

    /// The control: an unmarked 2xx write still projects, and a 202 or a GET does not.
    #[test]
    fn an_unmarked_write_still_triggers_a_projection() {
        let write = response(StatusCode::CREATED, json!({})).into_response();
        assert!(completes_a_write(&Method::POST, &write));
        let ok = response(StatusCode::OK, json!({})).into_response();
        assert!(completes_a_write(&Method::PATCH, &ok));
        assert!(!completes_a_write(&Method::GET, &ok));
        let accepted = response(StatusCode::ACCEPTED, json!({})).into_response();
        assert!(!completes_a_write(&Method::POST, &accepted));
        let refused = response(StatusCode::BAD_REQUEST, json!({})).into_response();
        assert!(!completes_a_write(&Method::POST, &refused));
    }
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

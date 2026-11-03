//! Operator API over the router: plan a folder, run one import at a time in the
//! background, and read the ledger. No agent-token admission: a plan reads any
//! folder the caller names.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Ledger, Outcome};

const DEFAULT_PORT: u16 = 8107;

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/routes", "This manifest. Agent credentials are not admitted."),
    route_manifest::get("GET", "/api/import/plan", "Sort every file under folder (absolute path) without importing: files and bytes per route, files per skip reason."),
    route_manifest::get("GET", "/api/import/status", "Ledger counts per route, outcome and reason for every recorded file under folder (absolute path)."),
    route_manifest::get("GET", "/api/import/runs/current", "The running or last finished run: folder, files done of total, outcome counts, error. null before the first run."),
    route_manifest::Route {
        method: "POST",
        path: "/api/import/runs",
        request_schema: Some(route_manifest::schema_of::<RunRequest>),
        summary: "Start an import of folder (absolute path) in the background. Source files are only read. 202 with the run; 409 while another run is active.",
    },
];

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    folder: String,
}

#[derive(Deserialize)]
struct FolderQuery {
    folder: String,
}

/// One import run as the page polls it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Job {
    pub folder: String,
    pub total: usize,
    pub done: usize,
    pub unchanged: usize,
    pub outcomes: BTreeMap<&'static str, usize>,
    pub finished: bool,
    pub error: Option<String>,
}

struct AppState {
    ledger: Ledger,
    job: Mutex<Option<Job>>,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    eprintln!("import: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "import operation failed".into(),
    )
}

/// An absolute, existing directory, resolved through symlinks.
fn folder(raw: &str) -> Result<PathBuf, ApiError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "folder must be an absolute path".into(),
        ));
    }
    match path.canonicalize() {
        Ok(path) if path.is_dir() => Ok(path),
        _ => Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("{raw} is not a reachable folder"),
        )),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "import" }))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("import", ROUTES))
}

async fn plan(Query(query): Query<FolderQuery>) -> Result<Json<Value>, ApiError> {
    let root = folder(&query.folder)?;
    let shown = root.display().to_string();
    let entries =
        tokio::task::spawn_blocking(move || crate::plan(&root).map_err(|error| error.to_string()))
            .await
            .map_err(internal)?
            .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, error))?;
    let tally = crate::tally(&entries);
    let routes: Vec<_> = tally
        .routes
        .iter()
        .map(|(route, (files, bytes))| json!({ "route": route, "files": files, "bytes": bytes }))
        .collect();
    let skipped: Vec<_> = tally
        .skipped
        .iter()
        .map(|(reason, files)| json!({ "reason": reason, "files": files }))
        .collect();
    Ok(Json(
        json!({ "folder": shown, "files": entries.len(), "routes": routes, "skipped": skipped }),
    ))
}

async fn status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FolderQuery>,
) -> Result<Json<Value>, ApiError> {
    let root = folder(&query.folder)?;
    let shown = root.display().to_string();
    let rows: Vec<_> = state
        .ledger
        .status(&root)
        .map_err(internal)?
        .into_iter()
        .map(|(tally, files)| {
            json!({ "route": tally.route, "outcome": tally.outcome, "detail": tally.detail, "files": files })
        })
        .collect();
    Ok(Json(json!({ "folder": shown, "rows": rows })))
}

async fn current(State(state): State<Arc<AppState>>) -> Json<Value> {
    let job = state.job.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Json(json!({ "job": job }))
}

async fn start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RunRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let root = folder(&request.folder)?;
    let job = Job {
        folder: root.display().to_string(),
        ..Job::default()
    };
    {
        let mut slot = state.job.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|job| !job.finished) {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "an import is already running".into(),
            ));
        }
        *slot = Some(job.clone());
    }
    let worker = Arc::clone(&state);
    tokio::task::spawn_blocking(move || {
        let result = run_job(&worker, &root);
        let mut slot = worker.job.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = slot.as_mut() {
            job.finished = true;
            job.error = result.err().map(|error| error.to_string());
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": job }))))
}

fn run_job(state: &AppState, root: &std::path::Path) -> crate::Fallible<()> {
    let entries = crate::plan(root)?;
    let update = |change: &dyn Fn(&mut Job)| {
        if let Some(job) = state.job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            change(job);
        }
    };
    update(&|job| job.total = entries.len());
    // The blocking HTTP client must be built off the async threads.
    let documents = crate::DocumentsHttp::from_deployment()?;
    crate::run(&entries, &state.ledger, &documents, |_, outcome, _| {
        update(&|job| {
            job.done += 1;
            match outcome {
                Some(outcome) => *job.outcomes.entry(Outcome::name(outcome)).or_default() += 1,
                None => job.unchanged += 1,
            }
        });
    })?;
    Ok(())
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/import/plan", get(plan))
        .route("/api/import/status", get(status))
        .route("/api/import/runs", axum::routing::post(start))
        .route("/api/import/runs/current", get(current))
        .layer(middleware::from_fn_with_state(
            "import",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

/// Serve the import API through Sjel's loopback credential gate, without agent-token access.
///
/// # Errors
/// The ledger or the asynchronous runtime cannot be opened.
pub fn serve() -> crate::Fallible<()> {
    let ledger = Ledger::open(&sjel_config::database_path())?;
    let state = Arc::new(AppState {
        ledger,
        job: Mutex::new(None),
    });
    let port = sjel_config::resolve_port(None, None, DEFAULT_PORT);
    tokio::runtime::Runtime::new()?.block_on(sjel_server::serve(
        "import",
        sjel_server::Reach::Loopback,
        port,
        router(state),
        sjel_server::InboundAuth::from_deployment().require_credential(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_covers_routes_and_write_bodies() {
        assert!(route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    #[test]
    fn a_folder_must_be_absolute_and_reachable() {
        assert_eq!(
            folder("relative/path").err().map(|e| e.0),
            Some(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            folder("/no/such/folder/here").err().map(|e| e.0),
            Some(StatusCode::NOT_FOUND)
        );
        assert!(folder(&std::env::temp_dir().display().to_string()).is_ok());
    }
}

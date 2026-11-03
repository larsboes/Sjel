//! The read-only HTTP surface over the media ledger.
//!
//! The CLI answers questions about bytes on a disk that is mounted on this machine. This answers
//! the questions the dashboard can draw: how many digests and locations are indexed, which volumes
//! hold them, and what the recent ingest runs did with every path they considered — including the
//! refusals a count alone cannot name.
//!
//! Nothing here writes. Every handler calls a `Ledger` reader, so this surface cannot change the
//! ledger or a library byte, and the browser cannot reach a verb that moves a file.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use media::store::Ledger;

/// The port the dev proxy and the health poll reach. Overridden by `SJEL_PORT`.
pub const DEFAULT_PORT: u16 = 8102;

/// How many runs `GET /api/ingests` reports. One page of history, not the whole table: a run is
/// one row per file, and a panel wants the last few, not the archive.
const RECENT_RUNS: i64 = 20;

/// The manifest served at `GET /routes`.
pub const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/status",
        "Index totals and every volume the ledger holds, with its indexed path and byte counts.",
    ),
    route_manifest::get(
        "GET",
        "/api/ingests",
        "The 20 most recent ingest runs, newest first, with each run's disposition counts.",
    ),
    route_manifest::get(
        "GET",
        "/api/ingests/{id}",
        "One ingest run and the disposition of every path it considered.",
    ),
    route_manifest::get(
        "GET",
        "/api/perceptual",
        "Advisory perceptual near-duplicate candidates from media_phashes and their operator review decisions from media_reviews.",
    ),
    route_manifest::get(
        "GET",
        "/api/geotags",
        "Indexed EXIF geotags with latitude, longitude, and capture dates from media_geotags.",
    ),
];

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(error: String) -> Self {
        eprintln!("media: {error}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "media ledger operation failed".into(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "no ingest run with that id".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

fn status_json(ledger: &Ledger) -> Result<Value, String> {
    let (files, locations) = ledger.counts().map_err(|error| error.to_string())?;
    let volumes = ledger.volumes().map_err(|error| error.to_string())?;
    Ok(json!({ "files": files, "locations": locations, "volumes": volumes }))
}

fn ingests_json(ledger: &Ledger) -> Result<Value, String> {
    let runs = ledger
        .recent_ingests(RECENT_RUNS)
        .map_err(|error| error.to_string())?;
    Ok(json!({ "runs": runs }))
}

fn ingest_json(ledger: &Ledger, id: i64) -> Result<Option<Value>, String> {
    let Some(run) = ledger.ingest(id).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let items = ledger.ingest_items(id).map_err(|error| error.to_string())?;
    Ok(Some(json!({ "run": run, "items": items })))
}

fn perceptual_json(ledger: &Ledger) -> Result<Value, String> {
    let report = media::perceptual::propose_near_duplicates(
        ledger,
        None,
        media::perceptual::DEFAULT_MAX_DISTANCE,
    )
    .map_err(|error| error.to_string())?;
    serde_json::to_value(report).map_err(|error| error.to_string())
}

fn geotags_json(ledger: &Ledger) -> Result<Value, String> {
    let rows = ledger.geotags().map_err(|error| error.to_string())?;
    Ok(json!({ "geotags": rows }))
}

/// Run a blocking ledger read off the async runtime, so one slow query cannot stall the listener.
async fn blocking<T>(
    ledger: Arc<Ledger>,
    work: impl FnOnce(&Ledger) -> Result<T, String> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || work(&ledger))
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(ApiError::internal)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "media" }))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("media", ROUTES))
}

async fn status(State(ledger): State<Arc<Ledger>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(blocking(ledger, status_json).await?))
}

async fn ingests(State(ledger): State<Arc<Ledger>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(blocking(ledger, ingests_json).await?))
}

async fn ingest(
    State(ledger): State<Arc<Ledger>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    blocking(ledger, move |ledger| ingest_json(ledger, id))
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn perceptual(State(ledger): State<Arc<Ledger>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(blocking(ledger, perceptual_json).await?))
}

async fn geotags(State(ledger): State<Arc<Ledger>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(blocking(ledger, geotags_json).await?))
}

fn router(ledger: Arc<Ledger>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/status", get(status))
        .route("/api/ingests", get(ingests))
        .route("/api/ingests/{id}", get(ingest))
        .route("/api/perceptual", get(perceptual))
        .route("/api/geotags", get(geotags))
        .layer(middleware::from_fn_with_state(
            "media",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(ledger)
}

/// Open the shared ledger and serve until stopped. A ledger that cannot be opened is fatal: the
/// process would answer with numbers it did not read.
pub fn serve() {
    let path = sjel_config::database_path();
    let ledger = match Ledger::open(&path) {
        Ok(ledger) => ledger,
        Err(error) => {
            eprintln!("media: cannot open ledger at {}: {error}", path.display());
            std::process::exit(1);
        }
    };
    let port = sjel_config::resolve_port(None, None, DEFAULT_PORT);
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime could not start");
    runtime.block_on(sjel_server::serve_local(
        "media",
        port,
        router(Arc::new(ledger)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn scratch(name: &str, id: u64) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "media-server-{name}-{}-{id}.db",
            std::process::id()
        ))
    }

    #[test]
    fn manifest_covers_routes() {
        let source = include_str!("server.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    #[test]
    fn status_reports_what_the_ledger_holds() {
        let db = scratch("status", 0);
        let ledger = Ledger::open(&db).unwrap();
        ledger.register("VOL-1", "Library").unwrap();
        let body = status_json(&ledger).unwrap();
        assert_eq!(body["files"], json!(0));
        assert_eq!(body["locations"], json!(0));
        assert_eq!(body["volumes"][0]["label"], json!("Library"));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn an_unfinished_run_reports_null_finished_and_empty_items() {
        let db = scratch("ingest", 1);
        let ledger = Ledger::open(&db).unwrap();
        let id = ledger.start_ingest("/staging", "/staging", "[]").unwrap();
        let body = ingest_json(&ledger, id).unwrap().unwrap();
        assert_eq!(body["run"]["finished"], Value::Null);
        assert_eq!(body["items"], json!([]));
        assert!(ingest_json(&ledger, id + 1).unwrap().is_none());
        let _ = std::fs::remove_file(&db);
    }

    async fn body_of(uri: &str, id: u64) -> (StatusCode, Value) {
        let db = scratch("http", id);
        let ledger = Ledger::open(&db).unwrap();
        let response = router(Arc::new(ledger))
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let _ = std::fs::remove_file(&db);
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn every_read_route_answers_with_the_ledger_shape() {
        let (status, body) = body_of("/api/status", 2).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["files"], json!(0));
        assert!(body["volumes"].is_array());

        let (status, body) = body_of("/api/ingests", 3).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["runs"], json!([]));

        let (status, body) = body_of("/routes", 4).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["capability"], json!("media"));

        let (status, body) = body_of("/api/ingests/999", 5).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body["error"].is_string());
    }
}

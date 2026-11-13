//! Operator HTTP access to the ledger and local attachment intake.

mod content_review;

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use media::{attachments, store::Ledger};
pub use media_api::DEFAULT_MEDIA_PORT as DEFAULT_PORT;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// How many runs `GET /api/ingests` reports. One page of history, not the whole table: a run is
/// one row per file, and a panel wants the last few, not the archive.
const RECENT_RUNS: i64 = 20;

/// The manifest served at `GET /routes`.
pub const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/api/content/proposals", "Operator-only content review queue. Filters: state and after; at most 50 items and a next_after cursor. No previews in list rows."),
    route_manifest::get("GET", "/api/content/proposals/{id}", "Operator-only evidence, private PNG previews, raw candidates and approved metadata for one immutable proposal."),
    route_manifest::Route {
        method: "POST", path: "/api/content/proposals/{id}/decision",
        summary: "Persist explicit accepted tags/collections or rejection for the exact evidence fingerprint. Idempotent identical retries; conflicting decisions refuse. Originals never change.",
        request_schema: Some(route_manifest::schema_of::<media_api::ContentDecision>),
    },
    route_manifest::get(
        "GET",
        "/api/attachments/{sha256}",
        "Verified local attachment metadata and source filenames; 404 if unknown. Operator credential required.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/attachments",
        summary: "Store raw bytes (at most 128 MiB), with filename and sha256 query parameters. Returns attachment and created. Operator credential required; no extraction or cloud use.",
        request_schema: Some(raw_attachment_schema),
    },
    route_manifest::get("GET", media_api::ROUTE_MEDIA_HEALTH, "Liveness."),
    route_manifest::get("GET", media_api::ROUTE_MEDIA_ROUTES, "This manifest."),
    route_manifest::get(
        "GET",
        media_api::ROUTE_MEDIA_STATUS,
        "Index totals and every volume the ledger holds, with its indexed path and byte counts.",
    ),
    route_manifest::get(
        "GET",
        media_api::ROUTE_MEDIA_INGESTS,
        "The 20 most recent ingest runs, newest first, with each run's disposition counts.",
    ),
    route_manifest::get(
        "GET",
        media_api::ROUTE_MEDIA_INGEST_BY_ID,
        "One ingest run and the disposition of every path it considered.",
    ),
    route_manifest::get(
        "GET",
        media_api::ROUTE_MEDIA_PERCEPTUAL,
        "Advisory perceptual near-duplicate candidates from media_phashes and their operator review decisions from media_reviews.",
    ),
    route_manifest::get(
        "GET",
        media_api::ROUTE_MEDIA_GEOTAGS,
        "Indexed EXIF geotags with latitude, longitude, and capture dates from media_geotags.",
    ),
];

fn raw_attachment_schema() -> Value {
    json!({"type": "string", "format": "binary", "maxLength": attachments::MAX_BYTES})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    filename: String,
    sha256: String,
}

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

async fn attachment(
    State(library): State<Arc<attachments::Library>>,
    Path(digest): Path<String>,
) -> Result<Json<Value>, ApiError> {
    attachments::validate_digest(&digest).map_err(|e| invalid(e.to_string()))?;
    let held =
        tokio::task::spawn_blocking(move || library.holding(&digest).map_err(|e| e.to_string()))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(ApiError::internal)?;
    held.map(|attachment| Json(json!({"attachment": attachment})))
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "attachment not found".into(),
        })
}

fn invalid(message: String) -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        message,
    }
}

async fn upload_attachment(
    State(library): State<Arc<attachments::Library>>,
    Query(query): Query<UploadQuery>,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    attachments::validate_filename(&query.filename).map_err(|e| invalid(e.to_string()))?;
    attachments::validate_digest(&query.sha256).map_err(|e| invalid(e.to_string()))?;
    let (attachment, created) = tokio::task::spawn_blocking(move || {
        let actual: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != query.sha256 {
            return Err(invalid("upload SHA-256 mismatch".into()));
        }
        attachments::validate_content(&bytes).map_err(|e| invalid(e.to_string()))?;
        library
            .upload(&query.filename, &query.sha256, &bytes)
            .map_err(|e| ApiError::internal(e.to_string()))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({"attachment": attachment, "created": created})),
    ))
}

fn attachment_router(library: Arc<attachments::Library>) -> Router {
    Router::new()
        .route("/api/attachments", axum::routing::post(upload_attachment))
        .route("/api/attachments/{sha256}", get(attachment))
        .layer(DefaultBodyLimit::max(attachments::MAX_BYTES))
        .with_state(library)
}

fn router(ledger: Arc<Ledger>) -> Router {
    Router::new()
        .route(media_api::ROUTE_MEDIA_HEALTH, get(health))
        .route(media_api::ROUTE_MEDIA_ROUTES, get(routes))
        .route(media_api::ROUTE_MEDIA_STATUS, get(status))
        .route(media_api::ROUTE_MEDIA_INGESTS, get(ingests))
        .route(media_api::ROUTE_MEDIA_INGEST_BY_ID, get(ingest))
        .route(media_api::ROUTE_MEDIA_PERCEPTUAL, get(perceptual))
        .route(media_api::ROUTE_MEDIA_GEOTAGS, get(geotags))
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
    let ledger = Arc::new(ledger);
    let library = sjel_config::overlay_root()
        .ok_or_else(|| "media attachments require a private overlay".to_string())
        .and_then(|root| {
            attachments::Library::open(ledger.clone(), root.join("data/media/attachments"))
                .map_err(|e| e.to_string())
        });
    let library = match library {
        Ok(library) => Arc::new(library),
        Err(error) => {
            eprintln!("media: {error}");
            std::process::exit(1);
        }
    };
    let app = router(ledger.clone())
        .merge(content_review::router(ledger, library.clone()))
        .merge(attachment_router(library))
        .layer(middleware::from_fn_with_state(
            "media",
            sjel_server::origin::refuse_foreign_origins,
        ));
    let port = sjel_config::resolve_port(None, None, DEFAULT_PORT);
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime could not start");
    runtime.block_on(sjel_server::serve(
        "media",
        sjel_server::Reach::Loopback,
        port,
        app,
        sjel_server::InboundAuth::from_deployment().require_credential(),
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
        let source = concat!(
            include_str!("server.rs"),
            include_str!("server/content_review.rs")
        );
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
    async fn attachment_intake_requires_operator_auth_and_verifies_retries() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("media-http-attachments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ledger = Arc::new(Ledger::open(&root.join("ledger.db")).unwrap());
        let library =
            Arc::new(attachments::Library::open(ledger.clone(), root.join("originals")).unwrap());
        let inner = router(ledger).merge(attachment_router(library));
        let app = sjel_server::authenticated(
            inner.clone(),
            sjel_server::InboundAuth::with_token(Some("public-operator-fixture".into()))
                .require_credential(),
        );
        let digest: String = Sha256::digest(b"image")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let uri = format!("/api/attachments?filename=photo.jpg&sha256={digest}");
        let request = |method: &str, uri: &str, token: Option<&str>, bytes: Vec<u8>| {
            let mut builder = axum::http::Request::builder().method(method).uri(uri);
            if let Some(token) = token {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            builder.body(axum::body::Body::from(bytes)).unwrap()
        };
        for method in ["GET", "POST"] {
            let target = if method == "GET" {
                format!("/api/attachments/{digest}")
            } else {
                uri.clone()
            };
            for credential in [None, Some("public-agent-fixture")] {
                let response = app
                    .clone()
                    .oneshot(request(method, &target, credential, b"image".to_vec()))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        let closed = sjel_server::authenticated(
            inner,
            sjel_server::InboundAuth::with_token(None).require_credential(),
        );
        assert_eq!(
            closed
                .oneshot(request("POST", &uri, None, b"image".to_vec()))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        for expected in [StatusCode::CREATED, StatusCode::OK] {
            let response = app
                .clone()
                .oneshot(request(
                    "POST",
                    &uri,
                    Some("public-operator-fixture"),
                    b"image".to_vec(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["attachment"]["sha256"], digest);
            assert_eq!(value["created"], expected == StatusCode::CREATED);
        }
        let response = app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/api/attachments/{digest}"),
                Some("public-operator-fixture"),
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        for bad_uri in [
            format!("/api/attachments?filename=..%2Fx.jpg&sha256={digest}"),
            "/api/attachments?filename=x.jpg&sha256=invalid".to_owned(),
        ] {
            assert_eq!(
                app.clone()
                    .oneshot(request(
                        "POST",
                        &bad_uri,
                        Some("public-operator-fixture"),
                        b"image".to_vec()
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        let placeholder = b"bplist00synthetic-placeholder";
        let placeholder_digest: String = Sha256::digest(placeholder)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let placeholder_uri =
            format!("/api/attachments?filename=placeholder.mov&sha256={placeholder_digest}");
        let refused = app
            .clone()
            .oneshot(request(
                "POST",
                &placeholder_uri,
                Some("public-operator-fixture"),
                placeholder.to_vec(),
            ))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let refusal = axum::body::to_bytes(refused.into_body(), 4096)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&refusal)
            .unwrap()
            .contains("placeholder"));
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    &uri,
                    Some("public-operator-fixture"),
                    b"wrong".to_vec()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    &uri,
                    Some("public-operator-fixture"),
                    vec![0; attachments::MAX_BYTES + 1]
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            app.oneshot(request(
                "GET",
                &format!("/api/attachments/{}", "0".repeat(64)),
                Some("public-operator-fixture"),
                vec![]
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::NOT_FOUND
        );
        std::fs::remove_dir_all(root).unwrap();
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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use software::{discover, SoftwareStore};

/// How often the running service scans again. Sizes and last-used dates move over days, not
/// minutes; the review page can always ask for a scan now.
const RESCAN: Duration = Duration::from_secs(6 * 3_600);

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/apps",
        "Every app, leftover and tool a scan has seen, largest footprint first: kind, sizes, last used, whether it is still present, the current decision with its note, and why it is due review.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/apps/scan",
        summary: "Scan apps, ~/Library leftovers and package-manager tooling now. Body: {} or none. Answers when the scan is recorded, with the number of apps found. 409 while another scan runs.",
        request_schema: Some(route_manifest::schema_of::<ScanRequest>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/apps/{bundle_id}/decision",
        summary: "Record a decision about one app: use, remove, replace or rebuild, with a note. Appended; earlier decisions stay as history. 404 for a bundle id no scan has seen.",
        request_schema: Some(route_manifest::schema_of::<DecisionRequest>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/apps/{bundle_id}/uninstall",
        summary: "Uninstall a row whose current decision is remove: an app or leftover goes to ~/.Trash, a package through its manager's uninstall. Body: {} or none. 409 unless the decision is remove; answers with the steps taken, then rescans.",
        request_schema: Some(route_manifest::schema_of::<UninstallRequest>),
    },
];

pub use software_api::{DecisionRequest, ScanRequest, UninstallRequest};

/// A note is a sentence or two about why, not a document.
const MAX_NOTE_CHARS: usize = 2_000;

struct AppState {
    store: SoftwareStore,
    home: PathBuf,
    /// Held for the length of a scan or an uninstall, so none of them interleave.
    scanning: Mutex<()>,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    eprintln!("software: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "software operation failed".into(),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "software" }))
}

async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    state.store.ping().await.map_err(internal)?;
    Ok(Json(json!({ "status": "ready", "service": "software" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("software", ROUTES))
}

async fn list(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let apps = state.store.apps().await.map_err(internal)?;
    Ok(Json(json!({ "apps": apps })))
}

async fn scan_now(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let Ok(_held) = state.scanning.try_lock() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "a scan is already running".into(),
        ));
    };
    let found = scan(&state).await.map_err(internal)?;
    Ok(Json(json!({ "scanned": found })))
}

async fn decide(
    State(state): State<Arc<AppState>>,
    Path(bundle_id): Path<String>,
    Json(request): Json<DecisionRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.note.chars().count() > MAX_NOTE_CHARS {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("a note holds at most {MAX_NOTE_CHARS} characters"),
        ));
    }
    let known = state
        .store
        .decide(&bundle_id, request.decision, request.note.trim())
        .await
        .map_err(internal)?;
    if !known {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("no scan has seen '{bundle_id}'"),
        ));
    }
    Ok(Json(
        json!({ "bundle_id": bundle_id, "decision": request.decision.as_str() }),
    ))
}

async fn uninstall(
    State(state): State<Arc<AppState>>,
    Path(bundle_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let Ok(_held) = state.scanning.try_lock() else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "a scan or another uninstall is running".into(),
        ));
    };
    let Some(app) = state.store.get(&bundle_id).await.map_err(internal)? else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("no scan has seen '{bundle_id}'"),
        ));
    };
    if app.decision.as_deref() != Some("remove") || !app.present {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!("'{}' is not marked remove; decide first", app.name),
        ));
    }
    let home = state.home.clone();
    let outcome = tokio::task::spawn_blocking(move || software::uninstall::uninstall(&app, &home))
        .await
        .map_err(internal)?;
    let steps = match outcome {
        Ok(steps) => steps,
        Err(reason) => return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, reason)),
    };
    for step in &steps {
        eprintln!("software: {bundle_id}: {step}");
    }
    scan(&state).await.map_err(internal)?;
    Ok(Json(json!({ "bundle_id": bundle_id, "steps": steps })))
}

/// The scan shells out to `plutil`, `mdls`, `du` and `brew` per row, so it runs off the async
/// threads.
async fn scan(state: &AppState) -> software::store::Fallible<usize> {
    let home = state.home.clone();
    let found = tokio::task::spawn_blocking(move || discover::scan_all(&home)).await?;
    state.store.record_scan(&found).await?;
    Ok(found.len())
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/apps", get(list))
        .route("/api/apps/scan", post(scan_now))
        .route("/api/apps/{bundle_id}/decision", post(decide))
        .route("/api/apps/{bundle_id}/uninstall", post(uninstall))
        .layer(middleware::from_fn_with_state(
            "software",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

fn fatal(error: impl std::fmt::Display) -> ! {
    eprintln!("software: {error}");
    std::process::exit(1);
}

fn port() -> u16 {
    sjel_config::resolve_port(Some("SJEL_SOFTWARE_PORT"), None, 8106)
}

async fn serve() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| fatal("HOME is not set"));
    let path = sjel_config::env_var("SJEL_SOFTWARE_DB_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(|path| sjel_config::expand_tilde(&path))
        .unwrap_or_else(|| fatal("SJEL_SOFTWARE_DB_PATH must name the separate owner file"));
    let database = Arc::new(path);
    let store = {
        let database = Arc::clone(&database);
        tokio::task::spawn_blocking(move || SoftwareStore::open(&database))
            .await
            .unwrap_or_else(|error| fatal(error))
            .unwrap_or_else(|error| fatal(error))
    };
    let state = Arc::new(AppState {
        store,
        home,
        scanning: Mutex::new(()),
    });
    let timer = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            {
                let _held = timer.scanning.lock().await;
                match scan(&timer).await {
                    Ok(found) => eprintln!("software: scan recorded {found} apps"),
                    Err(error) => eprintln!("software: scan failed: {error}"),
                }
            }
            tokio::time::sleep(RESCAN).await;
        }
    });
    let snapshot_state = Arc::clone(&state);
    tokio::join!(
        sjel_server::serve_local("software", port(), router(state)),
        sjel_server::owner_snapshot::serve("software", database, move |database, target| {
            snapshot_state
                .store
                .snapshot(database, target)
                .map_err(|error| error.to_string())
        }),
    );
}

async fn cli_request<T: serde::de::DeserializeOwned>(
    method: reqwest::Method,
    route: &str,
) -> Result<T, reqwest::Error> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?;
    let mut request = client.request(method, format!("http://127.0.0.1:{}{route}", port()));
    if let Some(bearer) = sjel_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header(reqwest::header::AUTHORIZATION, bearer);
    }
    request.send().await?.error_for_status()?.json().await
}

async fn cli(command: &str) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        "snapshot" => sjel_server::owner_snapshot::submit("software").await?,
        "scan" => {
            #[derive(serde::Deserialize)]
            struct Response {
                scanned: usize,
            }
            let response: Response =
                cli_request(reqwest::Method::POST, software_api::ROUTE_APPS_SCAN).await?;
            println!("{} apps recorded", response.scanned);
        }
        _ => {
            #[derive(serde::Deserialize)]
            struct Response {
                apps: Vec<software::App>,
            }
            let response: Response =
                cli_request(reqwest::Method::GET, software_api::ROUTE_APPS).await?;
            for app in response.apps.iter().filter(|a| a.present) {
                println!(
                    "{:<8}  {:>9}  {:<10}  {:<9}  {}{}",
                    app.kind,
                    mb(app.app_bytes + app.data_bytes),
                    app.last_used
                        .as_deref()
                        .map_or("never", |d| &d[..10.min(d.len())]),
                    app.decision.as_deref().unwrap_or("-"),
                    app.name,
                    app.review
                        .as_deref()
                        .map(|r| format!("  [review: {r}]"))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_048_576.)
}

const USAGE: &str = "software: the apps and tools on this Mac, and what you decided about each.

  software serve   HTTP API (scans on start and every six hours)
  software scan    scan apps, ~/Library leftovers and package-manager tooling now
  software list    what is present: kind, size with data, last used, decision, review
  software snapshot  request the running owner's protected backup snapshot";

fn main() {
    let command = std::env::args().nth(1).unwrap_or_default();
    if !matches!(command.as_str(), "serve" | "scan" | "list" | "snapshot") {
        println!("{USAGE}");
        std::process::exit(if command.is_empty() || command == "-h" {
            0
        } else {
            2
        });
    }
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime could not start");
    if command == "serve" {
        runtime.block_on(serve());
    } else {
        runtime
            .block_on(cli(&command))
            .unwrap_or_else(|error| fatal(error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    #[test]
    fn every_contract_route_is_declared() {
        for route in software_api::ALL_ROUTES {
            assert!(
                ROUTES.iter().any(|declared| declared.path == route),
                "software-api names {route}, which the manifest does not declare"
            );
        }
    }

    #[test]
    fn manifest_covers_routes_and_write_schema() {
        let source = include_str!("main.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    #[tokio::test]
    async fn decision_routes_refuse_unknown_apps_and_long_notes() {
        let dir = std::env::temp_dir().join(format!("sjel-software-router-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = Arc::new(AppState {
            store: SoftwareStore::open(&dir.join("sjel.db")).unwrap(),
            home: dir.clone(),
            scanning: Mutex::new(()),
        });
        let app = router(state);
        let post = |body: Value| {
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/apps/never.seen/decision")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap()
        };
        let status = app
            .clone()
            .oneshot(post(json!({ "decision": "use", "note": "daily" })))
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::NOT_FOUND);
        let long = "x".repeat(MAX_NOTE_CHARS + 1);
        let status = app
            .clone()
            .oneshot(post(json!({ "decision": "use", "note": long })))
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let status = app
            .oneshot(post(json!({ "decision": "maybe" })))
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

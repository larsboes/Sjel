use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde_json::{json, Value};

use web_links::{
    store::Fallible, EditLink, HistoryDecision, HistoryDecisionRequest, HistoryReply, HistoryScan,
    LinksReply, LinksStore, WriteLink, ROUTE_HISTORY_CANDIDATES, ROUTE_HISTORY_SCAN, ROUTE_LINKS,
};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get("GET", ROUTE_LINKS, "Saved web destinations, grouped by site and ordered by pin and name."),
    route_manifest::Route { method: "POST", path: ROUTE_LINKS, summary: "Save an HTTP(S) destination. Repeated normalized URLs return the existing row.", request_schema: Some(route_manifest::schema_of::<WriteLink>) },
    route_manifest::Route { method: "PATCH", path: "/api/links/{id}", summary: "Edit title, group override or pin state for one saved destination.", request_schema: Some(route_manifest::schema_of::<EditLink>) },
    route_manifest::Route { method: "DELETE", path: "/api/links/{id}", summary: "Remove one saved destination.", request_schema: None },
    route_manifest::Route { method: "POST", path: "/api/links/{id}/open", summary: "Record an explicit open and return the destination.", request_schema: None },
    route_manifest::Route { method: "POST", path: ROUTE_HISTORY_SCAN, summary: "Read Chrome and Safari history locally and stage candidates; no candidate is saved without review.", request_schema: Some(route_manifest::schema_of::<HistoryScan>) },
    route_manifest::get("GET", ROUTE_HISTORY_CANDIDATES, "Pending browser-history candidates for review."),
    route_manifest::Route { method: "POST", path: "/api/history/candidates/{id}/decision", summary: "Keep or dismiss one pending browser-history candidate.", request_schema: Some(route_manifest::schema_of::<HistoryDecisionRequest>) },
];

#[derive(Clone)]
struct AppState {
    store: LinksStore,
    home: PathBuf,
}

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
fn internal(error: impl std::fmt::Display) -> ApiError {
    eprintln!("web-links: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "web-links operation failed".into(),
    )
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, error.to_string())
}

async fn blocking<T>(
    operation: impl FnOnce() -> Fallible<T> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || operation().map_err(|error| error.to_string()))
        .await
        .map_err(internal)?
        .map_err(internal)
}

async fn health() -> Json<Value> {
    Json(json!({"status":"ok", "service":"web-links"}))
}
async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    blocking(move || store.ping()).await?;
    Ok(Json(json!({"status":"ready", "service":"web-links"})))
}
async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("web-links", ROUTES))
}

async fn list(State(state): State<Arc<AppState>>) -> Result<Json<LinksReply>, ApiError> {
    let store = state.store.clone();
    Ok(Json(LinksReply {
        links: blocking(move || store.list()).await?,
    }))
}
async fn add(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WriteLink>,
) -> Result<Json<web_links::Link>, ApiError> {
    web_links::destination(&request.url).map_err(invalid)?;
    if request.title.chars().count() > 240
        || request
            .group
            .as_deref()
            .is_some_and(|group| group.chars().count() > 80)
    {
        return Err(invalid(
            "Title must be at most 240 characters and group at most 80.",
        ));
    }
    let store = state.store.clone();
    Ok(Json(blocking(move || store.add(&request)).await?))
}
async fn edit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<EditLink>,
) -> Result<Json<web_links::Link>, ApiError> {
    if request
        .title
        .as_deref()
        .is_some_and(|value| value.chars().count() > 240)
        || request
            .group
            .as_deref()
            .is_some_and(|value| value.chars().count() > 80)
    {
        return Err(invalid(
            "Title must be at most 240 characters and group at most 80.",
        ));
    }
    let store = state.store.clone();
    blocking(move || {
        store.edit(
            &id,
            request.title.as_deref(),
            request.group.as_deref(),
            request.pinned,
        )
    })
    .await?
    .map(Json)
    .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Destination not found.".into()))
}
async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = state.store.clone();
    if blocking(move || store.remove(&id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(
            StatusCode::NOT_FOUND,
            "Destination not found.".into(),
        ))
    }
}
async fn open_link(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<web_links::Link>, ApiError> {
    let store = state.store.clone();
    blocking(move || store.record_open(&id))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Destination not found.".into()))
}

async fn scan_history(
    State(state): State<Arc<AppState>>,
    Json(request): Json<HistoryScan>,
) -> Result<Json<HistoryReply>, ApiError> {
    if !(1..=365).contains(&request.days) || !(1..=100).contains(&request.min_visits) {
        return Err(invalid(
            "History scan bounds must be 1–365 days and 1–100 visits.",
        ));
    }
    let home = state.home.clone();
    let candidates = tokio::task::spawn_blocking(move || {
        web_links::collect_history(&home, request.days, request.min_visits)
    })
    .await
    .map_err(internal)?
    .map_err(invalid)?;
    let store = state.store.clone();
    let staged = blocking(move || store.stage(&candidates)).await?;
    let store = state.store.clone();
    let candidates = blocking(move || store.candidates()).await?;
    Ok(Json(HistoryReply { staged, candidates }))
}
async fn history_candidates(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<web_links::Candidate>>, ApiError> {
    let store = state.store.clone();
    Ok(Json(blocking(move || store.candidates()).await?))
}
async fn decide_candidate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<HistoryDecisionRequest>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let decision = request.decision;
    let result = blocking(move || store.decide_candidate(&id, decision)).await?;
    if !result.found {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Candidate is no longer pending.".into(),
        ));
    }
    match decision {
        HistoryDecision::Keep => result
            .saved
            .map(|link| Json(json!({"decision":"kept", "link":link})))
            .ok_or_else(|| internal("kept candidate has no saved link")),
        HistoryDecision::Dismiss => Ok(Json(json!({"decision":"dismissed"}))),
    }
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route(ROUTE_LINKS, get(list).post(add))
        .route(&format!("{ROUTE_LINKS}/{{id}}"), patch(edit).delete(remove))
        .route(&format!("{ROUTE_LINKS}/{{id}}/open"), post(open_link))
        .route(ROUTE_HISTORY_SCAN, post(scan_history))
        .route(ROUTE_HISTORY_CANDIDATES, get(history_candidates))
        .route(
            &format!("{ROUTE_HISTORY_CANDIDATES}/{{id}}/decision"),
            post(decide_candidate),
        )
        .layer(middleware::from_fn_with_state(
            "web-links",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

fn open() -> Result<Arc<AppState>, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let store =
        LinksStore::open(&sjel_config::database_path()).map_err(|error| error.to_string())?;
    Ok(Arc::new(AppState { store, home }))
}

async fn serve() {
    let state = match open() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("web-links: {error}");
            std::process::exit(1);
        }
    };
    let port = sjel_config::resolve_port(None, None, 8109);
    sjel_server::serve_local("web-links", port, router(state)).await;
}

#[tokio::main]
async fn main() {
    serve().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    fn test_router() -> Router {
        let path =
            std::env::temp_dir().join(format!("sjel-web-links-api-{}.db", std::process::id()));
        let store = LinksStore::open(&path).unwrap();
        router(Arc::new(AppState {
            store,
            home: std::env::temp_dir(),
        }))
    }

    #[tokio::test]
    async fn list_and_create_routes_use_api_namespace() {
        let app = test_router();
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/links")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"url":"https://reddit.com/r/rust","title":"Rust"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/links")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn manifest_covers_routes() {
        let source = include_str!("server.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
    }
}

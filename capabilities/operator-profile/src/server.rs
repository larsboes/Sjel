use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use operator_profile::{
    config::Config, model::ProfileInput, store::profile_json, OperatorProfileStore, PutOutcome,
};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/profile",
        "Read the canonical operator profile. The response contains only profile fields and revision metadata.",
    ),
    route_manifest::Route {
        method: "PUT",
        path: "/api/profile",
        summary: "Replace the canonical operator profile. Requires expected_revision; stale writes return 409 and write nothing.",
        request_schema: Some(route_manifest::schema_of::<PutProfileRequest>),
    },
];

#[derive(Deserialize, schemars::JsonSchema)]
struct PutProfileRequest {
    profile: ProfileInput,
    expected_revision: u64,
}

struct ApiError {
    status: StatusCode,
    body: Value,
}

impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: json!({ "error": message.into() }),
        }
    }
    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: json!({ "error": "profile operation failed" }),
        }
    }
    fn stale(current_revision: u64) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            body: json!({ "error": "profile changed since it was read", "code": "stale_profile", "current_revision": current_revision }),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "operator-profile" }))
}

async fn ready(State(store): State<Arc<OperatorProfileStore>>) -> Result<Json<Value>, ApiError> {
    tokio::task::spawn_blocking(move || store.ping().map_err(|error| error.to_string()))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?;
    Ok(Json(
        json!({ "status": "ready", "service": "operator-profile" }),
    ))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("operator-profile", ROUTES))
}

async fn get_profile(
    State(store): State<Arc<OperatorProfileStore>>,
) -> Result<Json<Value>, ApiError> {
    let profile =
        tokio::task::spawn_blocking(move || store.get().map_err(|error| error.to_string()))
            .await
            .map_err(|_| ApiError::internal())?
            .map_err(|_| ApiError::internal())?;
    let stored = profile.is_some();
    let body = profile_json(profile).map_err(|_| ApiError::internal())?;
    Ok(Json(json!({ "profile": body, "stored": stored })))
}

async fn put_profile(
    State(store): State<Arc<OperatorProfileStore>>,
    Json(request): Json<PutProfileRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    request.profile.validate().map_err(ApiError::bad)?;
    let input = request.profile;
    let expected_revision = request.expected_revision;
    let outcome = tokio::task::spawn_blocking(move || {
        store
            .put(&input, expected_revision)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|_| ApiError::internal())?;
    match outcome {
        PutOutcome::Stored(profile) => {
            let body = profile_json(Some(profile)).map_err(|_| ApiError::internal())?;
            Ok((
                StatusCode::OK,
                Json(json!({ "profile": body, "stored": true })),
            ))
        }
        PutOutcome::Stale { current_revision } => Err(ApiError::stale(current_revision)),
    }
}

fn router(store: Arc<OperatorProfileStore>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/profile", get(get_profile).put(put_profile))
        .layer(middleware::from_fn_with_state(
            "operator-profile",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(store)
}

pub async fn serve() {
    let config = Config::load();
    let store = match OperatorProfileStore::open(&config.database_path) {
        Ok(store) => store,
        Err(_) => {
            eprintln!("operator-profile: cannot open private profile store");
            std::process::exit(1);
        }
    };
    sjel_server::serve_local("operator-profile", config.port, router(Arc::new(store))).await;
}

fn main() {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime could not start")
        .block_on(serve());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_covers_routes_and_write_schema() {
        let source = include_str!("server.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    #[test]
    fn response_shape_has_no_unstored_profile_data() {
        let empty = profile_json(None).unwrap();
        assert_eq!(empty["revision"], json!(0));
        assert_eq!(empty["fields"], json!({}));
        let body = json!({ "profile": empty, "stored": false });
        assert_eq!(body["stored"], json!(false));
    }
}

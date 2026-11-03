use std::sync::Arc;

use devices::{auth, store};

use axum::{
    body::{to_bytes, Body},
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use store::{Device, DevicesStore, StoreError};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable device registry.",
    ),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::Route {
        method: "POST",
        path: "/api/pairing/challenges",
        summary: "Create a ten-minute, single-use pairing challenge for one new device.",
        request_schema: None,
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/pairing/claims",
        summary: "Claim one pairing challenge with a device label and Ed25519 public key.",
        request_schema: Some(route_manifest::schema_of::<ClaimRequest>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/pairing/requests",
        summary: "A device asks to join with its label and Ed25519 public key; the owner allows or denies it.",
        request_schema: Some(route_manifest::schema_of::<JoinRequest>),
    },
    route_manifest::get(
        "GET",
        "/api/pairing/requests",
        "Requests still waiting for the owner, each with the six-digit code of its key.",
    ),
    route_manifest::get(
        "GET",
        "/api/pairing/requests/{id}",
        "One request's state; the device polls it until it is approved, denied or expired.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/pairing/requests/{id}/approve",
        summary: "Allow one waiting device: register the key it asked with.",
        request_schema: None,
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/pairing/requests/{id}/deny",
        summary: "Deny one waiting device.",
        request_schema: None,
    },
    route_manifest::get(
        "GET",
        "/api/devices",
        "List registered devices, including revoked devices.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/devices/{id}/revoke",
        summary: "Revoke one active device. Revocation is idempotence-protected and visible.",
        request_schema: None,
    },
    route_manifest::get(
        "GET",
        "/api/devices/me",
        "Return the registered device represented by a signed request.",
    ),
];

#[derive(Debug, Deserialize, JsonSchema)]
struct ClaimRequest {
    challenge_id: String,
    code: String,
    label: String,
    platform: String,
    algorithm: String,
    /// Hex-encoded 32-byte Ed25519 public key. The private key never crosses this API.
    public_key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct JoinRequest {
    label: String,
    platform: String,
    algorithm: String,
    /// Hex-encoded 32-byte Ed25519 public key. The private key never crosses this API.
    public_key: String,
}

struct AppState {
    store: DevicesStore,
}

struct ApiError(StatusCode, Value);

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        let status = match &error {
            StoreError::Invalid(_) => StatusCode::BAD_REQUEST,
            StoreError::NotFound(_) => StatusCode::NOT_FOUND,
            StoreError::Conflict(_) => StatusCode::CONFLICT,
            StoreError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            StoreError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self(status, json!({ "error": error.to_string() }))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

type Reply = Result<Json<Value>, ApiError>;

async fn blocking<T: Send + 'static>(
    state: &Arc<AppState>,
    work: impl FnOnce(&AppState) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, ApiError> {
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || work(&state))
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": error.to_string() }),
            )
        })?
        .map_err(ApiError::from)
}

fn json_value<T: serde::Serialize>(value: T) -> Json<Value> {
    Json(serde_json::to_value(value).unwrap_or(Value::Null))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "devices" }))
}

async fn ready(State(state): State<Arc<AppState>>) -> Reply {
    blocking(&state, |state| state.store.ping()).await?;
    Ok(Json(json!({ "status": "ready", "service": "devices" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("devices", ROUTES))
}

async fn create_challenge(State(state): State<Arc<AppState>>) -> Response {
    match blocking(&state, |state| state.store.create_challenge()).await {
        Ok(challenge) => (StatusCode::CREATED, json_value(challenge)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn claim(State(state): State<Arc<AppState>>, Json(request): Json<ClaimRequest>) -> Response {
    let result = blocking(&state, move |state| {
        state.store.claim(
            &request.challenge_id,
            &request.code,
            request.label,
            request.platform,
            request.algorithm,
            request.public_key,
        )
    })
    .await;
    match result {
        Ok(device) => (StatusCode::CREATED, json_value(device)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn request_join(
    State(state): State<Arc<AppState>>,
    Json(request): Json<JoinRequest>,
) -> Response {
    let result = blocking(&state, move |state| {
        state.store.request_pairing(
            request.label,
            request.platform,
            request.algorithm,
            request.public_key,
        )
    })
    .await;
    match result {
        Ok(join) => (StatusCode::CREATED, json_value(join)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn pending_requests(State(state): State<Arc<AppState>>) -> Reply {
    let requests = blocking(&state, |state| state.store.pending_requests()).await?;
    Ok(json_value(json!({ "requests": requests })))
}

async fn join_status(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Reply {
    Ok(json_value(
        blocking(&state, move |state| state.store.pairing_request(&id)).await?,
    ))
}

async fn approve_join(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Reply {
    Ok(json_value(
        blocking(&state, move |state| state.store.decide_request(&id, true)).await?,
    ))
}

async fn deny_join(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Reply {
    Ok(json_value(
        blocking(&state, move |state| state.store.decide_request(&id, false)).await?,
    ))
}

async fn list_devices(State(state): State<Arc<AppState>>) -> Reply {
    let devices: Vec<Device> = blocking(&state, |state| state.store.list()).await?;
    Ok(json_value(json!({ "devices": devices })))
}

async fn revoke(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match blocking(&state, move |state| state.store.revoke(&id)).await {
        Ok(device) => json_value(device).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn device_me(Extension(device): Extension<Device>) -> Reply {
    Ok(json_value(device))
}

async fn signed_device_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: middleware::Next,
) -> Response {
    let signed = match auth::SignedRequest::from_headers(request.headers()) {
        Ok(signed) => signed,
        Err(error) => {
            return ApiError(StatusCode::UNAUTHORIZED, json!({ "error": error })).into_response()
        }
    };
    let method = request.method().as_str().to_string();
    let path_and_query = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_string(),
        |value| value.as_str().to_string(),
    );
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, 8 * 1024 * 1024).await {
        Ok(body) => body,
        Err(_) => {
            return ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({ "error": "request body is too large" }),
            )
            .into_response()
        }
    };
    let body_for_auth = body.to_vec();
    let signed_for_store = signed.clone();
    let device = match blocking(&state, move |state| {
        state
            .store
            .authenticate(&signed_for_store, &method, &path_and_query, &body_for_auth)
    })
    .await
    {
        Ok(device) => device,
        Err(error) => return error.into_response(),
    };
    let mut request = Request::from_parts(parts, Body::from(body));
    request.extensions_mut().insert(device);
    next.run(request).await
}

fn router(state: Arc<AppState>) -> Router {
    let signed = Router::new()
        .route("/api/devices/me", get(device_me))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            signed_device_auth,
        ));
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/pairing/challenges", post(create_challenge))
        .route("/api/pairing/claims", post(claim))
        .route(
            "/api/pairing/requests",
            post(request_join).get(pending_requests),
        )
        .route("/api/pairing/requests/{id}", get(join_status))
        .route("/api/pairing/requests/{id}/approve", post(approve_join))
        .route("/api/pairing/requests/{id}/deny", post(deny_join))
        .route("/api/devices", get(list_devices))
        .route("/api/devices/{id}/revoke", post(revoke))
        .merge(signed)
        .layer(middleware::from_fn_with_state(
            "devices",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let database_path = sjel_config::database_path();
    let port = sjel_config::resolve_port(None, None, 8098);
    let store = match DevicesStore::open(&database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("devices: cannot open store: {error}");
            std::process::exit(1);
        }
    };
    sjel_server::serve_local("devices", port, router(Arc::new(AppState { store }))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_manifest_contains_pairing_and_revocation() {
        assert!(ROUTES
            .iter()
            .any(|route| route.path == "/api/pairing/claims"));
        assert!(ROUTES
            .iter()
            .any(|route| route.path == "/api/devices/{id}/revoke"));
    }
}

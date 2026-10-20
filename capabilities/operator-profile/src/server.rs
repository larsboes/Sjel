use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use operator_profile::{
    config::Config,
    model::{Harness, ProfileInput},
    render::{self, SectionChange},
    store::profile_json,
    OperatorProfileStore, PutOutcome,
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
    route_manifest::Route {
        method: "POST",
        path: "/api/profile/export",
        summary: "Preview or apply the managed operator-profile section for a verified harness target (claude, codex, pi, opencode). When apply is true, expected_revision is required and must match the stored revision.",
        request_schema: Some(route_manifest::schema_of::<ExportProfileRequest>),
    },
];

#[derive(Deserialize, schemars::JsonSchema)]
struct PutProfileRequest {
    profile: ProfileInput,
    expected_revision: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ExportProfileRequest {
    harness: Harness,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    expected_revision: Option<u64>,
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

async fn export_profile(
    State(store): State<Arc<OperatorProfileStore>>,
    Json(request): Json<ExportProfileRequest>,
) -> Result<Json<Value>, ApiError> {
    let profile = tokio::task::spawn_blocking(move || store.get().map_err(|e| e.to_string()))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?
        .ok_or_else(|| ApiError::bad("no profile is stored yet"))?;

    if request.apply {
        let expected = request.expected_revision.ok_or_else(|| {
            ApiError::bad("export with apply=true requires expected_revision matching the reviewed profile revision")
        })?;
        if profile.revision != expected {
            return Err(ApiError::stale(profile.revision));
        }
    }

    let generated = render::render(&profile, request.harness).map_err(ApiError::bad)?;
    let home = std::env::var_os("SJEL_OPERATOR_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .ok_or_else(|| ApiError::bad("HOME is not set"))?;
    let target = render::target_path(&home, request.harness);
    let existing = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(ApiError::internal()),
    };
    let change = if request.apply {
        render::apply_document(&target, &existing, &generated).map_err(ApiError::bad)?
    } else {
        render::preview_document(&existing, &generated)
            .map_err(ApiError::bad)?
            .1
    };
    let change_label = match change {
        SectionChange::Create => "create",
        SectionChange::Replace => "replace",
        SectionChange::Unchanged => "unchanged",
    };
    let audit = render::audit(&profile, request.harness);
    let omitted: Vec<Value> = audit
        .omitted
        .into_iter()
        .map(|(id, reason)| json!({ "id": id, "reason": reason }))
        .collect();
    Ok(Json(json!({
        "harness": request.harness.as_str(),
        "target": target.display().to_string(),
        "profile_revision": profile.revision,
        "applied": request.apply,
        "change": change_label,
        "included": audit.included,
        "omitted": omitted,
        "generated": generated,
    })))
}

fn router(store: Arc<OperatorProfileStore>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/profile", get(get_profile).put(put_profile))
        .route("/api/profile/export", post(export_profile))
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

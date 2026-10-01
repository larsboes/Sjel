use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const ROLE: &str = "assistant";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_REPLY_TOKENS: u32 = 1024;
const DEFAULT_REPLY_TOKENS: u32 = 512;
const SYSTEM_INSTRUCTIONS: &str = "You are Sjel Assistant. Give concise, factual answers. Do not claim access to data or tools that the prompt does not provide.";

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct GenerateRequest {
    prompt: String,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct GenerateResponse {
    text: String,
    model: String,
    source: &'static str,
}

fn failure(status: StatusCode, message: &'static str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message })))
}

fn ready_role() -> Option<sjel_inference::ResolvedRole> {
    sjel_inference::InferenceConfig::load(sjel_config::overlay_config).role(ROLE)
}

fn role_is_local(role: &sjel_inference::ResolvedRole) -> bool {
    role.is_loopback()
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn ready() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = ready_role().filter(role_is_local).ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "the local assistant role is not configured",
        )
    })?;
    if role.max_input_tokens.is_none() {
        return Err(failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "the assistant role must declare max_input_tokens",
        ));
    }
    Ok(Json(json!({
        "status": "ready",
        "model": role.model,
        "max_input_bytes": role.max_input_tokens,
    })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest(
        "assistant",
        &[
            route_manifest::get("GET", "/health", "Liveness."),
            route_manifest::get(
                "GET",
                "/ready",
                "Configured local assistant model is available.",
            ),
            route_manifest::get("GET", "/routes", "This manifest."),
            route_manifest::Route {
                method: "POST",
                path: "/api/generate",
                summary: "Generate a bounded answer with the configured local assistant role.",
                request_schema: Some(route_manifest::schema_of::<GenerateRequest>),
            },
        ],
    ))
}

fn payload(role: &sjel_inference::ResolvedRole, request: &GenerateRequest) -> Value {
    let mut body = json!({
        "model": role.model,
        "messages": [
            { "role": "system", "content": match request.instructions.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(extra) => format!("{SYSTEM_INSTRUCTIONS}\n\n{extra}"),
                None => SYSTEM_INSTRUCTIONS.to_string(),
            } },
            { "role": "user", "content": request.prompt },
        ],
        "max_tokens": request.max_tokens.unwrap_or(DEFAULT_REPLY_TOKENS).min(MAX_REPLY_TOKENS),
    });
    let object = body.as_object_mut().expect("object literal");
    if let Some(kwargs) = role.chat_template_kwargs.as_ref() {
        object.insert("chat_template_kwargs".into(), kwargs.clone());
    }
    if let Some(overrides) = role.request_overrides.as_ref().and_then(Value::as_object) {
        for (key, value) in overrides {
            if !matches!(key.as_str(), "model" | "messages" | "max_tokens") {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    body
}

fn fits_role(
    role: &sjel_inference::ResolvedRole,
    prompt: &str,
    instructions: Option<&str>,
) -> bool {
    let Some(limit) = role.max_input_tokens else {
        return false;
    };
    let input_bytes = prompt
        .len()
        .saturating_add(instructions.unwrap_or_default().len())
        .saturating_add(SYSTEM_INSTRUCTIONS.len())
        .saturating_add(256);
    input_bytes <= usize::try_from(limit).unwrap_or(usize::MAX)
        && prompt
            .len()
            .saturating_add(instructions.unwrap_or_default().len())
            <= MAX_PROMPT_BYTES
}

async fn generate(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<Value>)> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "prompt must not be empty"));
    }
    let max_tokens = request.max_tokens.unwrap_or(DEFAULT_REPLY_TOKENS);
    if max_tokens == 0 || max_tokens > MAX_REPLY_TOKENS {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "max_tokens must be between 1 and 1024",
        ));
    }

    let role = ready_role().filter(role_is_local).ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "the local assistant role is not configured",
        )
    })?;
    if !fits_role(&role, prompt, request.instructions.as_deref()) {
        return Err(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "prompt does not fit the configured assistant input limit",
        ));
    }

    let request = GenerateRequest {
        prompt: prompt.to_string(),
        instructions: request.instructions,
        max_tokens: Some(max_tokens),
    };
    let mut call = state
        .client
        .post(role.chat_completions_endpoint())
        .json(&payload(&role, &request));
    if let Some(key) = role.bearer_key() {
        call = call.bearer_auth(key);
    }
    let response = call.send().await.map_err(|_| {
        failure(
            StatusCode::BAD_GATEWAY,
            "the configured local model did not answer",
        )
    })?;
    if !response.status().is_success() {
        return Err(failure(
            StatusCode::BAD_GATEWAY,
            "the configured local model rejected the request",
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| failure(StatusCode::BAD_GATEWAY, "the model returned invalid JSON"))?;
    let text = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| failure(StatusCode::BAD_GATEWAY, "the model returned no answer"))?;

    Ok(Json(GenerateResponse {
        text: text.to_string(),
        model: role.model,
        source: "local",
    }))
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/generate", post(generate))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("assistant HTTP client configuration is valid");
    let port = sjel_server::resolve_port(None, None, 8100);
    sjel_server::serve_local("assistant", port, router(Arc::new(AppState { client }))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role() -> sjel_inference::ResolvedRole {
        let config: sjel_inference::InferenceConfig = r#"{
            "backends": { "local": { "api": "ollama", "base_url": "http://127.0.0.1:11434" } },
            "roles": { "assistant": { "backend": "local", "model": "qwen3:4b", "max_input_tokens": 8192 } }
        }"#
        .parse()
        .unwrap();
        config.role("assistant").unwrap()
    }

    #[test]
    fn request_is_bounded_and_uses_the_resolved_local_model() {
        let role = role();
        let request = GenerateRequest {
            prompt: "What is local?".into(),
            instructions: None,
            max_tokens: Some(700),
        };
        let body = payload(&role, &request);
        assert_eq!(body["model"], "qwen3:4b");
        assert_eq!(body["max_tokens"], 700);
        assert_eq!(body["messages"][1]["content"], "What is local?");
        assert!(fits_role(&role, "What is local?", None));
        assert!(!fits_role(&role, &"x".repeat(33 * 1024), None));
    }

    #[test]
    fn request_limits_are_enforced() {
        assert_eq!(MAX_REPLY_TOKENS, 1024);
        let role = role();
        assert!(!fits_role(&role, &"x".repeat(40 * 1024), None));
    }
}

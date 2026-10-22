use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sjel_agent::{Agent, AgentInput, Event, Message, Tool};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ROLE: &str = "assistant";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_REPLY_TOKENS: u32 = 1024;
const DEFAULT_REPLY_TOKENS: u32 = 512;
const MAX_AGENT_TURNS: usize = 6;
const TOOL_OUTPUT_LIMIT: usize = 16_000;
const DEFAULT_SHELL_URL: &str = "http://127.0.0.1:8082";
const SYSTEM_INSTRUCTIONS: &str = "You are Sjel Assistant. Give concise, factual answers. Do not claim access to data or tools that the prompt does not provide.";
const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("assistant");

struct AppState {
    client: reqwest::Client,
    shell_url: String,
    ask_runs: Mutex<HashMap<u64, AskRun>>,
    next_ask_id: AtomicU64,
}

#[derive(Clone)]
struct AskRun {
    input: AgentInput,
    stop: Arc<AtomicBool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SteerRequest {
    message: String,
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

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AskTurn {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AskRequest {
    prompt: String,
    #[serde(default)]
    history: Vec<AskTurn>,
    #[serde(default)]
    instructions: Option<String>,
    /// Exact configured `cloud_*` role selected by the operator. Absent means local assistant.
    #[serde(default)]
    provider_role: Option<String>,
    /// Per-request acknowledgement that this prompt is public (c0).
    #[serde(default)]
    confirm_public_send: bool,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CloudProviderOption {
    role: String,
    provider: String,
    model: String,
    data_tier: String,
    available: bool,
    reason: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct AskResponse {
    text: String,
    model: String,
    source: &'static str,
    tools: Vec<String>,
}

/// Shared HTTP client for read-only Sjel capability calls through the local shell proxy.
#[derive(Clone)]
struct ShellBridge {
    base: String,
    bearer: Option<String>,
}

impl ShellBridge {
    fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            bearer: sjel_server::InboundAuth::from_deployment().bearer_header(),
        }
    }

    fn get(&self, path: &str) -> Result<String, String> {
        let client = sjel_http::client(PURPOSE, Duration::from_secs(10))
            .map_err(|e| format!("http client: {e}"))?;
        let url = format!("{}{path}", self.base);
        let mut req = client.get(&url);
        if let Some(auth) = &self.bearer {
            req = req.header(axum::http::header::AUTHORIZATION, auth);
        }
        let res = req.send().map_err(|e| format!("{path}: {e}"))?;
        let status = res.status();
        let body = res.text().map_err(|e| format!("{path}: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "{path} returned HTTP {}: {}",
                status.as_u16(),
                sjel_agent::tools::cap(&body, 512)
            ));
        }
        Ok(sjel_agent::tools::cap(&body, TOOL_OUTPUT_LIMIT))
    }
}

struct SjelHealthTool(ShellBridge);
impl Tool for SjelHealthTool {
    fn name(&self) -> &'static str {
        "sjel_health"
    }
    fn description(&self) -> &'static str {
        "Read the health, version, uptime, and liveness of all enabled Sjel capabilities on this Mac."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/api/sjel-status/health")
    }
}

struct CalendarEntriesTool(ShellBridge);
impl Tool for CalendarEntriesTool {
    fn name(&self) -> &'static str {
        "calendar_entries"
    }
    fn description(&self) -> &'static str {
        "Read calendar entries between `from` and `to` (inclusive, YYYY-MM-DD)."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["from", "to"],
            "properties": {
                "from": { "type": "string", "description": "Start date in YYYY-MM-DD format." },
                "to": { "type": "string", "description": "End date in YYYY-MM-DD format." }
            }
        })
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let from = args
            .get("from")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing string argument `from`".to_owned())?;
        let to = args
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing string argument `to`".to_owned())?;
        if !is_iso_date(from) || !is_iso_date(to) {
            return Err("`from` and `to` must be YYYY-MM-DD dates".into());
        }
        self.0
            .get(&format!("/calendar/api/entries?from={from}&to={to}"))
    }
}

struct ProjectsListTool(ShellBridge);
impl Tool for ProjectsListTool {
    fn name(&self) -> &'static str {
        "projects_list"
    }
    fn description(&self) -> &'static str {
        "List all Sjel projects and their note counts."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/projects/api/projects")
    }
}

struct ProjectRecordsTool(ShellBridge);
impl Tool for ProjectRecordsTool {
    fn name(&self) -> &'static str {
        "project_records"
    }
    fn description(&self) -> &'static str {
        "Read the tasks and notes inside one Sjel project by its name."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "description": "Project name from projects_list." }
            }
        })
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing string argument `name`".to_owned())?;
        let mut url = reqwest::Url::parse("http://127.0.0.1/projects/api/project")
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("name", name);
        let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        self.0.get(&path)
    }
}

struct TripsPlansTool(ShellBridge);
impl Tool for TripsPlansTool {
    fn name(&self) -> &'static str {
        "trips_plans"
    }
    fn description(&self) -> &'static str {
        "Read saved and upcoming trip plans from the trips capability."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/trips/api/plans")
    }
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Read-only Sjel capability tools for the assistant loop. Never includes `bash` or filesystem tools.
fn sjel_tools(shell_url: &str) -> Vec<Box<dyn Tool>> {
    let bridge = ShellBridge::new(shell_url);
    vec![
        Box::new(SjelHealthTool(bridge.clone())),
        Box::new(CalendarEntriesTool(bridge.clone())),
        Box::new(ProjectsListTool(bridge.clone())),
        Box::new(ProjectRecordsTool(bridge.clone())),
        Box::new(TripsPlansTool(bridge)),
    ]
}

fn failure(status: StatusCode, message: impl AsRef<str>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.as_ref() })))
}

fn ready_role() -> Option<sjel_inference::ResolvedRole> {
    sjel_inference::InferenceConfig::load(sjel_config::overlay_config).role(ROLE)
}

fn ask_role(
    provider_role: Option<&str>,
    confirm_public_send: bool,
) -> Result<sjel_inference::ResolvedRole, String> {
    let Some(name) = provider_role else {
        return ready_role()
            .filter(role_is_local)
            .ok_or_else(|| "the local assistant role is not configured".into());
    };
    if !confirm_public_send {
        return Err(
            "selecting a cloud model requires confirming that this prompt is public (c0)".into(),
        );
    }
    if !name.starts_with("cloud_assistant_") {
        return Err("cloud Ask accepts only a configured cloud_assistant_* role".into());
    }
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(name)
        .filter(|role| role.is_cloud_endpoint())
        .ok_or_else(|| "the selected cloud role is not configured".to_owned())?;
    if !role.dispatch_ready() {
        return Err("the selected cloud role is missing its provider policy or credential".into());
    }
    Ok(role)
}

fn validate_cloud_context(
    provider_role: Option<&str>,
    history: &[AskTurn],
    instructions: Option<&str>,
) -> Result<(), &'static str> {
    if provider_role.is_none() {
        return Ok(());
    }
    if !history.is_empty() {
        return Err("cloud Ask sends only the current prompt; conversation history is excluded");
    }
    if instructions.is_some_and(|text| !text.trim().is_empty()) {
        return Err("cloud Ask does not accept custom instructions outside the visible prompt");
    }
    Ok(())
}

fn cloud_providers() -> Vec<CloudProviderOption> {
    let config = sjel_inference::InferenceConfig::load(sjel_config::overlay_config);
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let reviewed = sjel_inference::ReviewedProvidersList::load();
    cloud_provider_options(&config, &reviewed, &date)
}

fn cloud_provider_options(
    config: &sjel_inference::InferenceConfig,
    reviewed: &sjel_inference::ReviewedProvidersList,
    date: &str,
) -> Vec<CloudProviderOption> {
    config
        .roles_with_prefix("cloud_assistant_")
        .into_iter()
        .filter_map(|(name, role)| {
            if !role.is_cloud_endpoint() {
                return None;
            }
            let provider = role.provider_name.clone().unwrap_or_default();
            let reason = if !role.has_cloud_policy() {
                Some("provider policy is incomplete".to_owned())
            } else if !role.billing_active_on(date) {
                Some("billing authorization is inactive".to_owned())
            } else if let Err(error) = reviewed.check_admission(&provider, "c0", date) {
                Some(error.to_string())
            } else if !role.credential_ready() {
                Some("credential is not configured".to_owned())
            } else {
                None
            };
            Some(CloudProviderOption {
                role: name,
                provider,
                model: role.model,
                data_tier: role
                    .cloud_data_tier
                    .map(|tier| tier.as_str().to_owned())
                    .unwrap_or_else(|| "unconfigured".into()),
                available: reason.is_none(),
                reason,
            })
        })
        .collect()
}

fn role_is_local(role: &sjel_inference::ResolvedRole) -> bool {
    role.is_loopback()
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn ask_providers() -> Json<Vec<CloudProviderOption>> {
    Json(cloud_providers())
}

async fn ready() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = ready_role().filter(role_is_local).ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "the local assistant role is not configured",
        )
    })?;
    let checked = role.clone();
    tokio::task::spawn_blocking(move || checked.runtime_admission())
        .await
        .map_err(|error| failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .map_err(|reason| failure(StatusCode::CONFLICT, reason.to_string()))?;
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
            route_manifest::get(
                "GET",
                "/api/ask/providers",
                "Discover cloud model roles without exposing endpoints or credential paths.",
            ),
            route_manifest::get("GET", "/routes", "This manifest."),
            route_manifest::Route {
                method: "POST",
                path: "/api/generate",
                summary: "Generate a bounded answer with the configured local assistant role.",
                request_schema: Some(route_manifest::schema_of::<GenerateRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/ask",
                summary: "Run a multi-turn assistant session with read-only Sjel capability tools and no bash.",
                request_schema: Some(route_manifest::schema_of::<AskRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/ask/stream",
                summary: "Run an assistant session and stream text and read-only tool activity as server-sent events.",
                request_schema: Some(route_manifest::schema_of::<AskRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/ask/{run_id}/steer",
                summary: "Queue an instruction for the next assistant model request during an active streamed session.",
                request_schema: Some(route_manifest::schema_of::<SteerRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/ask/{run_id}/stop",
                summary: "Request cooperative cancellation of an active streamed assistant session.",
                request_schema: None,
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

async fn admit_request(
    role: &sjel_inference::ResolvedRole,
    body: &Value,
) -> Result<(), (StatusCode, Json<Value>)> {
    let role = role.clone();
    let body = body.clone();
    tokio::task::spawn_blocking(move || role.admit_chat_request(&body))
        .await
        .map_err(|error| failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .map_err(|reason| failure(StatusCode::CONFLICT, reason))
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
    let body = payload(&role, &request);
    admit_request(&role, &body).await?;
    let mut call = state
        .client
        .post(role.chat_completions_endpoint())
        .json(&body);
    if let Some(key) = role.bearer_key() {
        call = call.bearer_auth(key);
    }
    admit_request(&role, &body).await?;
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

fn run_ask(
    role: sjel_inference::ResolvedRole,
    shell_url: &str,
    request: AskRequest,
) -> Result<AskResponse, (StatusCode, String)> {
    run_ask_with_input(
        role,
        shell_url,
        request,
        AgentInput::default(),
        Arc::default(),
        |_| {},
    )
}

fn run_ask_with_input(
    role: sjel_inference::ResolvedRole,
    shell_url: &str,
    request: AskRequest,
    input: AgentInput,
    stop: Arc<AtomicBool>,
    mut emit: impl FnMut(Value),
) -> Result<AskResponse, (StatusCode, String)> {
    let cloud = role.is_cloud_endpoint();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let instructions = if cloud {
        "You are Sjel Assistant. Answer only from the public prompt. You have no access to Sjel data or tools."
    } else {
        SYSTEM_INSTRUCTIONS
    };
    let tools_instruction = if cloud {
        ""
    } else {
        " Use the read-only Sjel tools to check live capabilities, calendar entries, projects, or trips before answering."
    };
    let system = if cloud {
        instructions.to_owned()
    } else {
        match request
            .instructions
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            Some(extra) => {
                format!("{instructions}\nToday is {today}.{tools_instruction}\n\n{extra}")
            }
            None => format!("{instructions}\nToday is {today}.{tools_instruction}"),
        }
    };
    let mut messages = Vec::with_capacity(request.history.len() + 2);
    messages.push(Message::System { content: system });
    for turn in request.history {
        let content = turn.content.trim();
        if content.is_empty() {
            continue;
        }
        match turn.role.as_str() {
            "user" => messages.push(Message::User {
                content: content.to_owned(),
            }),
            "assistant" => messages.push(Message::Assistant {
                content: Some(content.to_owned()),
                tool_calls: None,
            }),
            _ => {}
        }
    }
    messages.push(Message::User {
        content: request.prompt,
    });

    let tools_for_agent = if cloud {
        Vec::new()
    } else {
        sjel_tools(shell_url)
    };
    let mut agent = Agent::new(role.clone(), tools_for_agent, [], stop)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    agent.max_turns = MAX_AGENT_TURNS;
    agent.set_input_queue(input);

    let mut tools = Vec::new();
    let mut streamed = String::new();
    let on_event = |event: Event<'_>| match event {
        Event::ToolCall(call) => {
            tools.push(call.function.name.clone());
            emit(json!({
                "type": "tool_call",
                "name": call.function.name,
                "arguments": call.function.arguments,
            }));
        }
        Event::Text(chunk) => {
            streamed.push_str(chunk);
            emit(json!({"type": "text", "content": chunk}));
        }
        Event::ToolResult { call, content } => emit(json!({
            "type": "tool_result",
            "name": call.function.name,
            "content": content,
        })),
        Event::Reasoning(_) => {}
    };
    let result = if cloud {
        agent.run_public(&mut messages, on_event)
    } else {
        agent.run(&mut messages, on_event)
    };
    result.map_err(|err| match err {
        sjel_agent::AgentError::RequestRefused(reason) => (StatusCode::CONFLICT, reason),
        sjel_agent::AgentError::Stopped => (StatusCode::CONFLICT, "stopped by the user".into()),
        other => (StatusCode::BAD_GATEWAY, other.to_string()),
    })?;

    let text = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant {
                content: Some(c), ..
            } if !c.trim().is_empty() => Some(c.trim().to_owned()),
            _ => None,
        })
        .or_else(|| {
            let trimmed = streamed.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "the model returned no answer".to_owned(),
            )
        })?;

    Ok(AskResponse {
        text,
        model: role.model,
        source: if cloud { "cloud" } else { "local" },
        tools,
    })
}

async fn ask(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AskRequest>,
) -> Result<Json<AskResponse>, (StatusCode, Json<Value>)> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "prompt must not be empty"));
    }
    validate_cloud_context(
        request.provider_role.as_deref(),
        &request.history,
        request.instructions.as_deref(),
    )
    .map_err(|reason| failure(StatusCode::CONFLICT, reason))?;
    let role = ask_role(
        request.provider_role.as_deref(),
        request.confirm_public_send,
    )
    .map_err(|reason| {
        let status = if request.provider_role.is_some() {
            StatusCode::CONFLICT
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        failure(status, reason)
    })?;
    let history_bytes: usize = request.history.iter().map(|t| t.content.len()).sum();
    let combined_len = prompt.len().saturating_add(history_bytes);
    if combined_len > MAX_PROMPT_BYTES
        || !fits_role(
            &role,
            &"x".repeat(combined_len),
            request.instructions.as_deref(),
        )
    {
        return Err(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "prompt and history do not fit the configured assistant input limit",
        ));
    }
    let shell_url = state.shell_url.clone();
    let req = AskRequest {
        prompt: prompt.to_owned(),
        history: request.history,
        instructions: request.instructions,
        provider_role: request.provider_role,
        confirm_public_send: request.confirm_public_send,
    };
    let response = tokio::task::spawn_blocking(move || run_ask(role, &shell_url, req))
        .await
        .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|(status, msg)| failure(status, msg))?;
    Ok(Json(response))
}

async fn ask_stream(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AskRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "prompt must not be empty"));
    }
    validate_cloud_context(
        request.provider_role.as_deref(),
        &request.history,
        request.instructions.as_deref(),
    )
    .map_err(|reason| failure(StatusCode::CONFLICT, reason))?;
    let role = ask_role(
        request.provider_role.as_deref(),
        request.confirm_public_send,
    )
    .map_err(|reason| {
        let status = if request.provider_role.is_some() {
            StatusCode::CONFLICT
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        failure(status, reason)
    })?;
    let history_bytes: usize = request.history.iter().map(|turn| turn.content.len()).sum();
    let combined_len = prompt.len().saturating_add(history_bytes);
    if combined_len > MAX_PROMPT_BYTES
        || !fits_role(
            &role,
            &"x".repeat(combined_len),
            request.instructions.as_deref(),
        )
    {
        return Err(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "prompt and history do not fit the configured assistant input limit",
        ));
    }

    let run_id = state.next_ask_id.fetch_add(1, Ordering::Relaxed);
    let input = AgentInput::default();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut runs = state.ask_runs.lock().map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ask sessions unavailable",
            )
        })?;
        if runs.len() >= 8 {
            return Err(failure(
                StatusCode::TOO_MANY_REQUESTS,
                "too many assistant sessions are active",
            ));
        }
        runs.insert(
            run_id,
            AskRun {
                input: input.clone(),
                stop: Arc::clone(&stop),
            },
        );
    }

    let (events_tx, events_rx) = tokio::sync::mpsc::channel::<String>(64);
    let shell_url = state.shell_url.clone();
    let request = AskRequest {
        prompt: prompt.to_owned(),
        history: request.history,
        instructions: request.instructions,
        provider_role: request.provider_role,
        confirm_public_send: request.confirm_public_send,
    };
    let sse = Sse::new(stream::unfold(events_rx, |mut rx| async move {
        rx.recv().await.map(|data| {
            (
                Ok::<_, std::convert::Infallible>(SseEvent::default().data(data)),
                rx,
            )
        })
    }))
    .keep_alive(KeepAlive::default());

    let state_for_task = state.clone();
    tokio::spawn(async move {
        let worker_tx = events_tx.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_ask_with_input(role, &shell_url, request, input, stop, |event| {
                if let Ok(encoded) = serde_json::to_string(&event) {
                    let _ = worker_tx.blocking_send(encoded);
                }
            })
        })
        .await;
        match result {
            Ok(Ok(reply)) => {
                let _ = events_tx
                    .send(json!({"type":"done", "text":reply.text, "model":reply.model, "source":reply.source, "tools":reply.tools}).to_string())
                    .await;
            }
            Ok(Err((StatusCode::CONFLICT, error))) if error == "stopped by the user" => {
                let _ = events_tx.send(json!({"type":"stopped"}).to_string()).await;
            }
            Ok(Err((status, error))) => {
                let _ = events_tx
                    .send(
                        json!({"type":"error", "status":status.as_u16(), "message":error})
                            .to_string(),
                    )
                    .await;
            }
            Err(error) => {
                let _ = events_tx
                    .send(json!({"type":"error", "message":error.to_string()}).to_string())
                    .await;
            }
        }
        if let Ok(mut runs) = state_for_task.ask_runs.lock() {
            runs.remove(&run_id);
        }
    });

    let response = sse.into_response();
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        "x-sjel-ask-run",
        HeaderValue::from_str(&run_id.to_string()).expect("numeric run id is a header value"),
    );
    Ok(axum::response::Response::from_parts(parts, body))
}

async fn steer_ask(
    Path(run_id): Path<u64>,
    State(state): State<Arc<AppState>>,
    Json(request): Json<SteerRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let message = request.message.trim();
    if message.is_empty() || message.len() > 4_000 {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "steering message must contain 1 to 4000 bytes",
        ));
    }
    let run = state
        .ask_runs
        .lock()
        .map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ask sessions unavailable",
            )
        })?
        .get(&run_id)
        .cloned()
        .ok_or_else(|| {
            failure(
                StatusCode::NOT_FOUND,
                "assistant session is no longer active",
            )
        })?;
    run.input.steer(message).map_err(|error| {
        let status = match error {
            sjel_agent::InputQueueError::Full => StatusCode::TOO_MANY_REQUESTS,
            sjel_agent::InputQueueError::Poisoned => StatusCode::INTERNAL_SERVER_ERROR,
        };
        failure(status, error.to_string())
    })?;
    Ok(Json(json!({"queued": true})))
}

async fn stop_ask(
    Path(run_id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let run = state
        .ask_runs
        .lock()
        .map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ask sessions unavailable",
            )
        })?
        .get(&run_id)
        .cloned()
        .ok_or_else(|| {
            failure(
                StatusCode::NOT_FOUND,
                "assistant session is no longer active",
            )
        })?;
    run.stop.store(true, Ordering::Relaxed);
    Ok(Json(json!({"stopping": true})))
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/ask/providers", get(ask_providers))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/generate", post(generate))
        .route("/api/ask", post(ask))
        .route("/api/ask/stream", post(ask_stream))
        .route("/api/ask/{run_id}/steer", post(steer_ask))
        .route("/api/ask/{run_id}/stop", post(stop_ask))
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
    let shell_url =
        std::env::var("SJEL_STATUS_URL").unwrap_or_else(|_| DEFAULT_SHELL_URL.to_owned());
    let port = sjel_server::resolve_port(None, None, 8100);
    sjel_server::serve_local(
        "assistant",
        port,
        router(Arc::new(AppState {
            client,
            shell_url,
            ask_runs: Mutex::new(HashMap::new()),
            next_ask_id: AtomicU64::new(1),
        })),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_role_selection_requires_public_consent_and_chat_role_name() {
        assert!(ask_role(Some("cloud_assistant_example"), false).is_err());
        assert!(ask_role(Some("cloud_embedding_example"), true).is_err());
    }

    #[test]
    fn cloud_context_rejects_hidden_history_and_instructions() {
        let prompt = Some("cloud_assistant_nim");
        assert!(validate_cloud_context(prompt, &[], None).is_ok());
        assert!(validate_cloud_context(prompt, &[], Some("hidden system prompt")).is_err());
        assert!(validate_cloud_context(
            prompt,
            &[AskTurn {
                role: "user".into(),
                content: "prior text".into(),
            }],
            None,
        )
        .is_err());
        assert!(validate_cloud_context(None, &[], Some("local instructions")).is_ok());
    }

    #[test]
    fn provider_discovery_shape_contains_no_endpoint_or_credential_path() {
        let response = serde_json::to_value(CloudProviderOption {
            role: "cloud_assistant_demo".into(),
            provider: "demo".into(),
            model: "demo-chat".into(),
            data_tier: "public".into(),
            available: false,
            reason: Some("credential is not configured".into()),
        })
        .unwrap();
        assert!(response.get("endpoint").is_none());
        assert!(response.get("api_key_file").is_none());
        assert!(response.get("credential").is_none());
    }

    #[test]
    fn provider_discovery_lists_reviewed_profiles_and_disables_unready_roles() {
        let mut config: sjel_inference::InferenceConfig =
            include_str!("../../../libs/inference/inference.config.example.json")
                .parse()
                .expect("shared inference example parses");
        for backend in [
            "assistant-openai",
            "assistant-nvidia-nim",
            "assistant-google-gemini",
        ] {
            config
                .backends
                .get_mut(backend)
                .expect("example backend exists")
                .api_key_file = None;
        }
        let reviewed = sjel_inference::ReviewedProvidersList::parse_toml(include_str!(
            "../../../providers.toml"
        ))
        .expect("provider reviews parse");

        let options = cloud_provider_options(&config, &reviewed, "2026-10-22");

        assert_eq!(
            options
                .iter()
                .map(|option| option.role.as_str())
                .collect::<Vec<_>>(),
            [
                "cloud_assistant_google",
                "cloud_assistant_nvidia_nim",
                "cloud_assistant_openai"
            ]
        );
        for option in &options {
            assert!(!option.available);
            let encoded = serde_json::to_value(option).unwrap();
            for private_field in ["endpoint", "base_url", "api_key_file", "credential"] {
                assert!(encoded.get(private_field).is_none());
            }
        }
        assert!(options[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("credential")));
        assert!(options[1]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("credential")));
        assert!(options[2]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("billing")));
    }
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

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

    #[test]
    fn assistant_tools_are_read_only_sjel_tools_and_never_bash() {
        let tools = sjel_tools("http://127.0.0.1:8082");
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec![
                "sjel_health",
                "calendar_entries",
                "projects_list",
                "project_records",
                "trips_plans"
            ]
        );
        assert!(!names.contains(&"bash"));
        assert!(!names.contains(&"write"));
        assert!(!names.contains(&"edit"));
        assert!(tools.iter().all(|t| t.read_only()));
    }

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            client: reqwest::Client::new(),
            shell_url: DEFAULT_SHELL_URL.into(),
            ask_runs: Mutex::new(HashMap::new()),
            next_ask_id: AtomicU64::new(1),
        })
    }

    #[tokio::test]
    async fn steering_route_queues_only_for_an_active_run() {
        let state = test_state();
        state.ask_runs.lock().unwrap().insert(
            23,
            AskRun {
                input: AgentInput::default(),
                stop: Arc::new(AtomicBool::new(false)),
            },
        );
        let reply = steer_ask(
            Path(23),
            State(state.clone()),
            Json(SteerRequest {
                message: "Focus on the dates.".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(reply.0["queued"], true);

        let error = steer_ask(
            Path(99),
            State(state),
            Json(SteerRequest {
                message: "Too late.".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn steering_route_reports_a_full_agent_queue() {
        let state = test_state();
        let input = AgentInput::default();
        while input.steer("queued").is_ok() {}
        state.ask_runs.lock().unwrap().insert(
            23,
            AskRun {
                input,
                stop: Arc::new(AtomicBool::new(false)),
            },
        );
        let error = steer_ask(
            Path(23),
            State(state),
            Json(SteerRequest {
                message: "One more.".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn stop_route_sets_the_active_run_flag() {
        let state = test_state();
        let stop = Arc::new(AtomicBool::new(false));
        state.ask_runs.lock().unwrap().insert(
            23,
            AskRun {
                input: AgentInput::default(),
                stop: Arc::clone(&stop),
            },
        );

        let reply = stop_ask(Path(23), State(state.clone())).await.unwrap();
        assert_eq!(reply.0["stopping"], true);
        assert!(stop.load(Ordering::Relaxed));

        let error = stop_ask(Path(99), State(state)).await.unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[test]
    fn ask_runs_agent_loop_with_sjel_tools() {
        let shell_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let shell_port = shell_listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = shell_listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = r#"{"ok":true,"version":"v1","uptime_seconds":120,"capabilities":{"calendar":{"up":true}}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let llm_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let llm_port = llm_listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let turns = [
                vec![json!({
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_1",
                                "type": "function",
                                "function": { "name": "sjel_health", "arguments": "{}" }
                            }]
                        }
                    }]
                })],
                vec![json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "content": "Calendar is up on v1." }
                    }]
                })],
            ];
            for chunks in turns {
                let (stream, _) = llm_listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut req = vec![0u8; length];
                reader.read_exact(&mut req).unwrap();
                let mut sse: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                sse.push_str("data: [DONE]\n\n");
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
                    sse.len()
                )
                .unwrap();
            }
        });

        let config: sjel_inference::InferenceConfig = format!(
            r#"{{
                "backends": {{ "local": {{ "api": "openai", "base_url": "http://127.0.0.1:{llm_port}/v1" }} }},
                "roles": {{ "assistant": {{ "backend": "local", "model": "qwen3:4b", "max_input_tokens": 8192 }} }}
            }}"#
        )
        .parse()
        .unwrap();
        let role = config.role("assistant").unwrap();
        let mut activity = Vec::new();
        let reply = run_ask_with_input(
            role,
            &format!("http://127.0.0.1:{shell_port}"),
            AskRequest {
                prompt: "Is calendar up?".into(),
                history: vec![],
                instructions: None,
                provider_role: None,
                confirm_public_send: false,
            },
            AgentInput::default(),
            Arc::default(),
            |event| match event["type"].as_str() {
                Some("tool_call") => activity.push(format!(
                    "call:{}",
                    event["name"].as_str().unwrap_or_default()
                )),
                Some("tool_result") => activity.push(format!(
                    "result:{}",
                    event["name"].as_str().unwrap_or_default()
                )),
                Some("text") => activity.push(format!(
                    "text:{}",
                    event["content"].as_str().unwrap_or_default()
                )),
                _ => {}
            },
        )
        .unwrap();
        assert_eq!(reply.text, "Calendar is up on v1.");
        assert_eq!(reply.tools, vec!["sjel_health"]);
        assert_eq!(
            activity,
            [
                "call:sjel_health",
                "result:sjel_health",
                "text:Calendar is up on v1."
            ]
        );
    }
}

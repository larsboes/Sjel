use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sjel_agent::{Agent, Event, Message, Tool};
use std::sync::Arc;
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

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    shell_url: String,
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
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let system = match request
        .instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(extra) => format!(
            "{SYSTEM_INSTRUCTIONS}\nToday is {today}. Use the read-only Sjel tools to check live capabilities, calendar entries, projects, or trips before answering.\n\n{extra}"
        ),
        None => format!(
            "{SYSTEM_INSTRUCTIONS}\nToday is {today}. Use the read-only Sjel tools to check live capabilities, calendar entries, projects, or trips before answering."
        ),
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

    let mut agent = Agent::new(role.clone(), sjel_tools(shell_url), [], Arc::default())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    agent.max_turns = MAX_AGENT_TURNS;

    let mut tools = Vec::new();
    let mut streamed = String::new();
    agent
        .run(&mut messages, |event| match event {
            Event::ToolCall(call) => tools.push(call.function.name.clone()),
            Event::Text(chunk) => streamed.push_str(chunk),
            _ => {}
        })
        .map_err(|err| match err {
            sjel_agent::AgentError::RequestRefused(reason) => (StatusCode::CONFLICT, reason),
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
        source: "local",
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
    let role = ready_role().filter(role_is_local).ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "the local assistant role is not configured",
        )
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
    };
    let response = tokio::task::spawn_blocking(move || run_ask(role, &shell_url, req))
        .await
        .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|(status, msg)| failure(status, msg))?;
    Ok(Json(response))
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/generate", post(generate))
        .route("/api/ask", post(ask))
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
        router(Arc::new(AppState { client, shell_url })),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let reply = run_ask(
            role,
            &format!("http://127.0.0.1:{shell_port}"),
            AskRequest {
                prompt: "Is calendar up?".into(),
                history: vec![],
                instructions: None,
            },
        )
        .unwrap();
        assert_eq!(reply.text, "Calendar is up on v1.");
        assert_eq!(reply.tools, vec!["sjel_health"]);
    }
}

//! A coding agent on this machine, for the app and for the phone.
//!
//! The loop is `libs/agent`'s; this file is the part that makes it a capability: sessions that
//! outlive a request, a stream a second screen can attach to, and the owner's permission in front
//! of every call that writes or runs.
//!
//! Nothing here is reachable from another machine. The listener is loopback, as every capability's
//! is (`sjel_server::serve_local_with`), and the phone arrives the way the rest of Sjel does —
//! `tailscale serve` identity or a paired device. A session holds file contents and command
//! output, so it never reaches a cloud model: `Agent::run` refuses a remote role outright, and
//! `Agent::run_public` refuses tools.

mod diff;
mod gate;
mod plugins;
mod session;
mod sql;
mod terminal;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use base64::Engine as _;
use futures::stream;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use sjel_agent::{Agent, AgentInput, Event};
use sjel_server::agent_policy::{AgentRoutes, PolicyFiles};

use crate::session::{Active, Session};
use crate::terminal::{ReadOutcome, Terminal, Terminals};

/// The inference role a run uses. An overlay that wants another names it here.
const ROLE: &str = "coding";
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_STEER_BYTES: usize = 4_000;
/// How many model requests one turn may make before it stops asking for tools. A run that keeps
/// calling tools forever is a run burning a local model's time.
const MAX_TURNS: usize = 40;
const PORT: u16 = 8108;

/// The routes the agent token never reaches, whatever the owner's mode says.
///
/// A terminal is a shell: a keystroke is a command, so there is no argument for the gate to
/// check and no card a human could read first. Closing them here is what makes "an agent cannot
/// get a shell" a property of the gate rather than of each handler remembering.
const AGENT_ROUTES: AgentRoutes = AgentRoutes {
    confirm: &[],
    get_writes: &[],
    closed: &[
        ("POST", "/api/agent/terminals"),
        ("GET", "/api/agent/terminals"),
        ("GET", "/api/agent/terminals/options"),
        ("GET", "/api/agent/terminals/{id}/stream"),
        ("POST", "/api/agent/terminals/{id}/input"),
        ("POST", "/api/agent/terminals/{id}/resize"),
        ("DELETE", "/api/agent/terminals/{id}"),
    ],
};

struct AppState {
    /// Live sessions: the ones this process has a conversation for.
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Where sessions are filed: `<overlay>/data/agent/sessions`.
    dir: PathBuf,
    /// The shells held for the app and the phone.
    terminals: Arc<Terminals>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateSession {
    /// Absolute path the run works in. Absent means the checkout this capability runs from.
    #[serde(default)]
    root: Option<String>,
    /// Plugin names. Absent means `<overlay>/config/agent.toml`, then the default set.
    #[serde(default)]
    plugins: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PromptRequest {
    prompt: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SteerRequest {
    message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ApproveRequest {
    approval_id: String,
    allow: bool,
    /// `once`, or `session` to allow this exact call for the rest of the session. Anything else
    /// is refused rather than read as one of them.
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StreamQuery {
    /// The last sequence the caller already has. A reconnect sends what it saw last.
    #[serde(default)]
    from: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenTerminal {
    /// A preset name from `<overlay>/config/agent.toml`, or one of the five defaults. An empty
    /// string is the login shell.
    preset: String,
    /// An existing directory under `$HOME`. Absent means `$HOME`.
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TerminalInput {
    /// Bytes to write to the pty, base64. At most 64 KiB, before decoding.
    data: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TerminalResize {
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TerminalStreamQuery {
    /// The byte offset the caller already has. Absent means the oldest output still held.
    #[serde(default)]
    from: Option<u64>,
}

fn failure(status: StatusCode, message: impl AsRef<str>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.as_ref() })))
}

/// The session for `id`, from memory or from disk, so a phone can read yesterday's run.
fn with_session(state: &AppState, id: &str) -> Result<Arc<Session>, (StatusCode, Json<Value>)> {
    {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "sessions unavailable"))?;
        if let Some(session) = sessions.get(id) {
            return Ok(Arc::clone(session));
        }
    }
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "sessions unavailable"))?;
    let found = session::root_dirs(&state.dir)
        .into_iter()
        .find_map(|dir| Session::load(&dir, id))
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "no such session"))?;
    let session = Arc::new(found);
    sessions.insert(id.to_owned(), Arc::clone(&session));
    Ok(session)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "capability": "agent" }))
}

/// Ready when a local role resolves and the session directory is writable. A coding run is
/// pointed at a local model by construction, so a missing role is the one thing that stops it.
async fn ready() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config).role(ROLE);
    match role {
        Some(role) => Ok(Json(json!({
            "status": "ready",
            "role": ROLE,
            "model": role.model,
            "cloud": role.is_cloud_endpoint(),
        }))),
        None => Err(failure(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("no `{ROLE}` inference role is configured"),
        )),
    }
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest(
        "agent",
        &[
            route_manifest::get("GET", "/health", "Liveness."),
            route_manifest::get(
                "GET",
                "/ready",
                "The local coding role resolves, so a run can start.",
            ),
            route_manifest::get("GET", "/routes", "This manifest."),
            route_manifest::get(
                "GET",
                "/api/agent/sessions",
                "Every session filed on this machine, newest first.",
            ),
            route_manifest::get(
                "GET",
                "/api/agent/sessions/{id}",
                "One session's meta: root, plugins, turn count, and whether a run is active.",
            ),
            route_manifest::get(
                "GET",
                "/api/agent/sessions/{id}/transcript",
                "One session's turns, as stored.",
            ),
            route_manifest::get(
                "GET",
                "/api/agent/sessions/{id}/stream",
                "Server-sent events for the session. `?from=<seq>` resumes where a reader left off, \
                 and the response keeps alive while a tool call waits for the owner.",
            ),
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/sessions",
                summary: "Start a session in a root with a plugin set.",
                request_schema: Some(route_manifest::schema_of::<CreateSession>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/sessions/{id}/prompt",
                summary: "Send one prompt and start the run. One run per session at a time.",
                request_schema: Some(route_manifest::schema_of::<PromptRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/sessions/{id}/approve",
                summary: "Allow or deny the tool call waiting on an approval id.",
                request_schema: Some(route_manifest::schema_of::<ApproveRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/sessions/{id}/steer",
                summary: "Queue a message for the next model request in the run.",
                request_schema: Some(route_manifest::schema_of::<SteerRequest>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/sessions/{id}/stop",
                summary: "Stop the turn and keep the conversation.",
                request_schema: None,
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/terminals",
                summary: "Open a pty on this Mac: a preset from the overlay, a directory under $HOME, \
                 and the size the client draws.",
                request_schema: Some(route_manifest::schema_of::<OpenTerminal>),
            },
            route_manifest::get(
                "GET",
                "/api/agent/terminals",
                "Every terminal held on this Mac, newest first.",
            ),
            route_manifest::get(
                "GET",
                "/api/agent/terminals/options",
                "What a new terminal can be: the preset names, and the folders it is offered to \
                 start in (home, then ~/Developer's folders, newest first).",
            ),
            route_manifest::get(
                "GET",
                "/api/agent/terminals/{id}/stream",
                "Server-sent events for one terminal: `output` frames with their byte offset, \
                 `truncated` with the oldest offset still held, and one `exit`.",
            ),
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/terminals/{id}/input",
                summary: "Write base64 bytes to the pty, as if typed.",
                request_schema: Some(route_manifest::schema_of::<TerminalInput>),
            },
            route_manifest::Route {
                method: "POST",
                path: "/api/agent/terminals/{id}/resize",
                summary: "Change the pty's size. The last resize wins.",
                request_schema: Some(route_manifest::schema_of::<TerminalResize>),
            },
            route_manifest::Route {
                method: "DELETE",
                path: "/api/agent/terminals/{id}",
                summary: "End the terminal: SIGHUP to its process group, SIGKILL after five seconds.",
                request_schema: None,
            },
        ],
    ))
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSession>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = match request
        .root
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        Some(given) => {
            let expanded = expand(given);
            let resolved = std::fs::canonicalize(&expanded)
                .map_err(|e| failure(StatusCode::BAD_REQUEST, format!("{given}: {e}")))?;
            if !resolved.is_dir() {
                return Err(failure(
                    StatusCode::BAD_REQUEST,
                    format!("{given} is not a directory"),
                ));
            }
            resolved
        }
        None => std::env::current_dir()
            .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
    };
    let plugins = request.plugins.unwrap_or_else(plugins::configured);
    // Assembling once here turns a typo in a plugin name into a 400 at creation, not a failed run
    // the operator reads a minute later.
    plugins::assemble(
        &plugins,
        &root,
        &Arc::new(AtomicBool::new(false)),
        Arc::new(session::EventLog::new()),
        session::Grants::new(),
        None,
    )
    .map_err(|reason| failure(StatusCode::BAD_REQUEST, reason))?;

    let dir = state.dir.join(session::root_slug(&root));
    let session = Session::create(&dir, &root, plugins)
        .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let meta = session.meta();
    state
        .sessions
        .lock()
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "sessions unavailable"))?
        .insert(session.id.clone(), Arc::new(session));
    Ok(Json(meta))
}

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut listed = Session::list(&state.dir);
    // A live session knows whether a run is active; the file on disk cannot say that.
    if let Ok(sessions) = state.sessions.lock() {
        for meta in &mut listed {
            if let Some(id) = meta["id"].as_str() {
                if let Some(live) = sessions.get(id) {
                    *meta = live.meta();
                }
            }
        }
    }
    Json(json!({ "sessions": listed }))
}

async fn session_meta(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    Ok(Json(with_session(&state, &id)?.meta()))
}

async fn transcript(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    Ok(Json(with_session(&state, &id)?.transcript()))
}

async fn prompt(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<PromptRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let text = request.prompt.trim();
    if text.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "prompt must not be empty"));
    }
    if text.len() > MAX_PROMPT_BYTES {
        return Err(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("prompt is longer than {MAX_PROMPT_BYTES} bytes"),
        ));
    }
    let session = with_session(&state, &id)?;

    let input = AgentInput::default();
    let stop = Arc::new(AtomicBool::new(false));
    session
        .begin(Active {
            input: input.clone(),
            stop: Arc::clone(&stop),
        })
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "a run is already active in this session; stop it or wait for it",
            )
        })?;
    // A new turn starts a new log, so `from=0` is always the whole of this run.
    session.log.clear();
    session.push_user(text.to_owned());
    session
        .log
        .push(json!({"type": "turn", "role": "user", "content": text}));

    let state_for_task = Arc::clone(&state);
    let session_for_task = Arc::clone(&session);
    tokio::spawn(async move {
        let worker_state = Arc::clone(&state_for_task);
        let worker_session = Arc::clone(&session_for_task);
        let result = tokio::task::spawn_blocking(move || {
            run_turn(&worker_state, &worker_session, input, stop)
        })
        .await;
        let log = session_for_task.log.clone();
        match result {
            Ok(Ok(())) => {
                log.push(json!({"type": "done"}));
            }
            Ok(Err(reason)) => {
                log.push(json!({"type": "error", "message": reason}));
            }
            Err(error) => {
                log.push(json!({"type": "error", "message": error.to_string()}));
            }
        }
        if let Err(error) = session_for_task.persist() {
            log.push(json!({"type": "error", "message": format!("session file: {error}")}));
        }
        session_for_task.end();
        log.finish();
    });

    Ok(Json(json!({ "started": true, "session": session.id })))
}

/// The blocking half: build the set, run the loop, push every event into the session's log.
fn run_turn(
    _state: &AppState,
    session: &Session,
    input: AgentInput,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(ROLE)
        .ok_or_else(|| format!("no `{ROLE}` inference role is configured"))?;
    if role.is_cloud_endpoint() {
        return Err(format!(
            "the `{ROLE}` role points at a cloud endpoint. A coding session holds file contents \
             and command output, so it runs on a local model only."
        ));
    }

    let set = plugins::assemble(
        session.plugins(),
        &session.root,
        &stop,
        Arc::clone(&session.log),
        Arc::clone(&session.grants),
        role.max_input_tokens,
    )?;

    let mut agent = Agent::new(role, set.tools, set.extensions, Arc::clone(&stop))
        .map_err(|e| e.to_string())?;
    agent.max_turns = MAX_TURNS;
    agent.set_input_queue(input);

    let mut messages = session
        .messages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let log = &session.log;
    let on_event = |event: Event<'_>| match event {
        Event::Text(chunk) => {
            log.push(json!({"type": "text", "content": chunk}));
        }
        Event::Reasoning(chunk) => {
            log.push(json!({"type": "reasoning", "content": chunk}));
        }
        Event::ToolCall(call) => {
            log.push(json!({
                "type": "tool_call",
                "id": call.id,
                "name": call.function.name,
                "arguments": call.function.arguments,
            }));
        }
        Event::ToolResult { call, content } => {
            log.push(json!({
                "type": "tool_result",
                "id": call.id,
                "name": call.function.name,
                "content": sjel_agent::tools::cap(content, 8_000),
            }));
        }
        Event::Usage {
            input_tokens,
            output_tokens,
        } => {
            log.push(json!({
                "type": "usage",
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            }));
        }
    };
    let outcome = agent.run(&mut messages, on_event);
    drop(messages);
    match outcome {
        Ok(()) => Ok(()),
        Err(sjel_agent::AgentError::Stopped) => {
            session.log.push(json!({"type": "stopped"}));
            Ok(())
        }
        Err(sjel_agent::AgentError::RequestRefused(reason)) => Err(reason),
        Err(other) => Err(other.to_string()),
    }
}

async fn stream(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<StreamQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let session = with_session(&state, &id)?;
    // Subscribe before reading, so an event that lands in between is a wake this reader has not
    // missed rather than one that waits for the next.
    let receiver = session.log.subscribe();
    let log = Arc::clone(&session.log);
    let cursor = query.from.unwrap_or(0);

    let events = stream::unfold(
        (cursor, receiver, log),
        |(mut cursor, mut receiver, log)| async move {
            loop {
                let next = log.after(cursor);
                if next.truncated {
                    // The events this reader asked for are gone. It is told once and the stream
                    // stops: a reader resumed mid-turn would draw half a tool call as a whole one.
                    return Some((
                        Ok::<_, std::convert::Infallible>(
                            SseEvent::default().event("truncated").data(
                                json!({"from": cursor, "first_seq": next.first_seq}).to_string(),
                            ),
                        ),
                        (u64::MAX, receiver, log),
                    ));
                }
                if let Some(event) = next.event {
                    cursor = event["seq"].as_u64().unwrap_or(cursor);
                    return Some((
                        Ok(SseEvent::default().data(event.to_string())),
                        (cursor, receiver, log),
                    ));
                }
                if next.done {
                    return None;
                }
                // A wake that never comes means every reader of the log is gone.
                receiver.recv().await?;
            }
        },
    );

    // The keep-alive is what holds the response open while a tool call waits for the owner. A
    // stream that went quiet for fifteen minutes would be dropped by a phone's radio long before
    // a human got to the prompt.
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn approve(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ApproveRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = with_session(&state, &id)?;
    let scope = request.scope.as_deref().unwrap_or("once");
    let for_session = match scope {
        "once" => false,
        "session" => true,
        other => {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                format!("scope is `once` or `session`, not `{other}`"),
            ))
        }
    };
    if for_session && !request.allow {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "a session-wide grant needs `allow`; deny applies to one call",
        ));
    }
    let policy = PolicyFiles::from_deployment().ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "no overlay is configured, so there is no approval to decide",
        )
    })?;
    let record = policy
        .decide(&request.approval_id, request.allow)
        .map_err(|reason| failure(StatusCode::CONFLICT, reason))?;
    // The grant is remembered only once the record says allowed: the file is the decision, and
    // the session list is a convenience on top of it.
    let remembered = request.allow && session.grants.allow_for_session(&request.approval_id);
    if !request.allow {
        session.grants.forget(&request.approval_id);
    }
    Ok(Json(json!({
        "approval": record,
        "for_session": remembered,
        "requested_scope": scope,
        "waiting": session.active().is_some(),
    })))
}

async fn steer(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<SteerRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let message = request.message.trim();
    if message.is_empty() || message.len() > MAX_STEER_BYTES {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            format!("a steering message is 1 to {MAX_STEER_BYTES} bytes"),
        ));
    }
    let session = with_session(&state, &id)?;
    let active = session.active().ok_or_else(|| {
        failure(
            StatusCode::CONFLICT,
            "no run is active in this session, so there is nothing to steer",
        )
    })?;
    active.input.steer(message).map_err(|error| {
        let status = match error {
            sjel_agent::InputQueueError::Full => StatusCode::TOO_MANY_REQUESTS,
            sjel_agent::InputQueueError::Poisoned => StatusCode::INTERNAL_SERVER_ERROR,
        };
        failure(status, error.to_string())
    })?;
    Ok(Json(json!({ "queued": true })))
}

async fn stop(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let session = with_session(&state, &id)?;
    let active = session
        .active()
        .ok_or_else(|| failure(StatusCode::CONFLICT, "no run is active in this session"))?;
    active.stop.store(true, Ordering::Relaxed);
    Ok(Json(json!({ "stopping": true })))
}

/// Who asked for a terminal: the paired device's own name, or this machine's loopback.
fn client_of(device: Option<Extension<sjel_server::AdmittedDevice>>) -> String {
    device
        .map(|Extension(device)| device.0)
        .unwrap_or_else(|| "loopback".to_owned())
}

fn status_of(refusal: &terminal::Refusal) -> StatusCode {
    match refusal {
        terminal::Refusal::BadRequest(_) => StatusCode::BAD_REQUEST,
        terminal::Refusal::TooMany(_) => StatusCode::CONFLICT,
        terminal::Refusal::Missing(_) => StatusCode::NOT_FOUND,
        terminal::Refusal::Failed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn open_terminal(
    State(state): State<Arc<AppState>>,
    device: Option<Extension<sjel_server::AdmittedDevice>>,
    Json(request): Json<OpenTerminal>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let client = client_of(device);
    let terminal = state
        .terminals
        .open(terminal::Open {
            preset: &request.preset,
            cwd: request.cwd.as_deref(),
            cols: request.cols.unwrap_or(80),
            rows: request.rows.unwrap_or(24),
            client: &client,
        })
        .map_err(|refusal| failure(status_of(&refusal), refusal.message()))?;
    // The id alone: everything else about a new terminal is its list row, and a client that
    // wants it asks for the list.
    Ok(Json(json!({ "id": terminal.id })))
}

async fn list_terminals(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(Value::Array(state.terminals.list()))
}

async fn terminal_options(State(state): State<Arc<AppState>>) -> Json<Value> {
    let folders = std::env::var_os("HOME")
        .map(|home| terminal::folders(std::path::Path::new(&home)))
        .unwrap_or_default();
    Json(json!({ "presets": state.terminals.preset_names(), "folders": folders }))
}

async fn terminal_stream(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TerminalStreamQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let terminal = state
        .terminals
        .get(&id)
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "no such terminal"))?;
    // Subscribe before the first read, so output that lands in between is a wake this reader
    // has not missed.
    let receiver = terminal.subscribe();
    terminal.attach();
    let cursor = query.from.unwrap_or_else(|| terminal.first_offset());

    // The attachment lives in the stream's own state, so the count the list reports falls back
    // when the reader goes away, however it goes away.
    let attached = Attached(Arc::clone(&terminal));
    let events = stream::unfold(
        (cursor, receiver, attached, false),
        |(mut cursor, mut receiver, attached, finished)| async move {
            if finished {
                return None;
            }
            loop {
                match attached.0.read(cursor, terminal::MAX_FRAME_BYTES) {
                    ReadOutcome::Truncated { first_offset } => {
                        // The stream stops here rather than continuing: a reader that kept drawing
                        // on a grid it never cleared would show a hole as if it were the start.
                        return Some((
                            Ok::<_, std::convert::Infallible>(
                                SseEvent::default()
                                    .event("truncated")
                                    .data(json!({ "first_offset": first_offset }).to_string()),
                            ),
                            (first_offset, receiver, attached, true),
                        ));
                    }
                    ReadOutcome::Output { offset, bytes } if !bytes.is_empty() => {
                        cursor = offset + bytes.len() as u64;
                        let data = json!({
                            "offset": offset,
                            "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                        });
                        return Some((
                            Ok(SseEvent::default().event("output").data(data.to_string())),
                            (cursor, receiver, attached, false),
                        ));
                    }
                    ReadOutcome::Output { .. } => {}
                }
                if !attached.0.alive() {
                    let exit = terminal::exit_event(&attached.0);
                    return Some((
                        Ok(SseEvent::default().event("exit").data(exit.to_string())),
                        (cursor, receiver, attached, true),
                    ));
                }
                // A wake that never comes means every reader of the log is gone.
                receiver.recv().await?;
            }
        },
    );

    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Holds one attachment for as long as a reader is attached, and lets go when it is not.
struct Attached(Arc<Terminal>);

impl Drop for Attached {
    fn drop(&mut self) {
        self.0.detach();
    }
}

async fn terminal_input(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<TerminalInput>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    if request.data.len() > terminal::MAX_INPUT_BYTES {
        return Err(failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "input is at most {} bytes of base64",
                terminal::MAX_INPUT_BYTES
            ),
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(request.data.as_bytes())
        .map_err(|error| failure(StatusCode::BAD_REQUEST, format!("data is base64: {error}")))?;
    let terminal = state
        .terminals
        .get(&id)
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "no such terminal"))?;
    terminal.write(&bytes).map_err(|reason| {
        let status = if terminal.alive() {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::CONFLICT
        };
        failure(status, reason)
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn terminal_resize(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<TerminalResize>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let terminal = state
        .terminals
        .get(&id)
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "no such terminal"))?;
    terminal
        .resize(request.cols, request.rows)
        .map_err(|reason| failure(StatusCode::CONFLICT, reason))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn close_terminal(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    state
        .terminals
        .close(&id)
        .await
        .map_err(|refusal| failure(status_of(&refusal), refusal.message()))?;
    Ok(StatusCode::NO_CONTENT)
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route(
            "/api/agent/sessions",
            get(list_sessions).post(create_session),
        )
        .route("/api/agent/sessions/{id}", get(session_meta))
        .route("/api/agent/sessions/{id}/transcript", get(transcript))
        .route("/api/agent/sessions/{id}/stream", get(stream))
        .route("/api/agent/sessions/{id}/prompt", post(prompt))
        .route("/api/agent/sessions/{id}/approve", post(approve))
        .route("/api/agent/sessions/{id}/steer", post(steer))
        .route("/api/agent/sessions/{id}/stop", post(stop))
        .route(
            "/api/agent/terminals",
            get(list_terminals).post(open_terminal),
        )
        .route("/api/agent/terminals/options", get(terminal_options))
        .route("/api/agent/terminals/{id}", delete(close_terminal))
        .route("/api/agent/terminals/{id}/stream", get(terminal_stream))
        .route("/api/agent/terminals/{id}/input", post(terminal_input))
        .route("/api/agent/terminals/{id}/resize", post(terminal_resize))
        .with_state(state)
}

/// `~` and `~/…`, so a root can be typed the way it is written.
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn sessions_dir() -> PathBuf {
    sjel_config::overlay_data_dir("agent")
        .unwrap_or_else(|| std::env::temp_dir().join("sjel-agent-sessions"))
        .join("sessions")
}

/// The one line per terminal start and end: no input, no output.
fn terminals_audit() -> PathBuf {
    sjel_config::overlay_data_dir("agent")
        .unwrap_or_else(|| std::env::temp_dir().join("sjel-agent-data"))
        .join("terminals.jsonl")
}

#[tokio::main]
async fn main() {
    let dir = sessions_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("agent: cannot create {}: {error}", dir.display());
    }
    let terminals = Arc::new(Terminals::new(
        terminals_audit(),
        terminal::Presets::configured(),
    ));
    tokio::spawn(Terminals::reap_forever(Arc::clone(&terminals)));
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        dir,
        terminals,
    });
    let port = sjel_server::resolve_port(None, None, PORT);
    sjel_server::serve_local_with("agent", port, router(state), AGENT_ROUTES).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TestOverlay {
        path: PathBuf,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestOverlay {
        fn new(endpoint: &str) -> Self {
            let lock = ENV_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let path = std::env::temp_dir().join(format!(
                "sjel-agent-integration-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(path.join("config")).unwrap();
            std::fs::create_dir_all(path.join("secrets/agent/approvals")).unwrap();
            std::fs::create_dir_all(path.join("data/agent/sessions")).unwrap();
            std::fs::write(
                path.join("config/inference.json"),
                format!(
                    r#"{{"backends":{{"local":{{"api":"openai","base_url":"{endpoint}"}}}},"roles":{{"coding":{{"backend":"local","model":"test-coder","max_input_tokens":8192}}}}}}"#
                ),
            )
            .unwrap();
            let previous = std::env::var_os("SJEL_PERSONAL_ROOT");
            std::env::set_var("SJEL_PERSONAL_ROOT", &path);
            Self {
                path,
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for TestOverlay {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var("SJEL_PERSONAL_ROOT", previous);
            } else {
                std::env::remove_var("SJEL_PERSONAL_ROOT");
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn fake_model() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut second_request = None;
            for (index, response) in [
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_write","type":"function","function":{"name":"write","arguments":r#"{"path":"result.txt","content":"written by tool"}"#}}]}}]}),
                json!({"choices":[{"index":0,"delta":{"content":"The file was written."}}]}),
            ]
            .into_iter()
            .enumerate()
            {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if line == "\r\n" { break; }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if index == 1 { second_request = Some(serde_json::from_slice::<Value>(&body).unwrap()); }
                let sse = format!("data: {response}\n\ndata: [DONE]\n\n");
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}", sse.len()).unwrap();
            }
            let request =
                second_request.expect("model received the follow-up after tool execution");
            assert!(
                request["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "tool"
                            && message["content"]
                                .as_str()
                                .is_some_and(|content| content.contains("wrote 15 bytes to"))
                    })
                }),
                "tool result was not sent to the model: {request}"
            );
        });
        (endpoint, thread)
    }

    fn session_for(overlay: &TestOverlay) -> Session {
        let root = overlay.path.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        Session::create(
            &overlay.path.join("data/agent/sessions").join("fixture"),
            &root,
            vec!["fs".into()],
        )
        .unwrap()
    }

    fn wait_for_approval(session: &Session) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut cursor = 0;
            while let Some(event) = session.log.after(cursor).event {
                cursor = event["seq"].as_u64().unwrap();
                if event["type"] == "needs_approval" {
                    return event["approval_id"].as_str().unwrap().to_owned();
                }
            }
            assert!(
                Instant::now() < deadline,
                "tool never reached the approval gate"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn event_types(session: &Session) -> Vec<String> {
        let mut cursor = 0;
        let mut result = Vec::new();
        loop {
            let next = session.log.after(cursor);
            let Some(event) = next.event else { break };
            cursor = event["seq"].as_u64().unwrap();
            result.push(event["type"].as_str().unwrap().to_owned());
        }
        result
    }

    fn start_turn(session: Arc<Session>) -> std::thread::JoinHandle<()> {
        session.push_user("write result.txt".into());
        session
            .log
            .push(json!({"type":"turn","role":"user","content":"write result.txt"}));
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AppState {
            sessions: Mutex::new(HashMap::new()),
            dir: PathBuf::new(),
            terminals: Arc::new(Terminals::new(
                std::env::temp_dir().join("sjel-agent-test-terminals.jsonl"),
                terminal::Presets::defaults(),
            )),
        });
        // `run_turn` is the same blocking worker the capability starts after accepting a prompt.
        std::thread::spawn(move || {
            run_turn(&state, &session, AgentInput::default(), stop).unwrap();
            session.log.push(json!({"type":"done"}));
            session.persist().unwrap();
            session.log.finish();
        })
    }

    #[tokio::test]
    async fn capability_run_gates_a_tool_and_session_load_restores_the_turns() {
        let (endpoint, model) = fake_model();
        let overlay = TestOverlay::new(&endpoint);
        let session = Arc::new(session_for(&overlay));
        let worker = start_turn(Arc::clone(&session));
        let approval_id = wait_for_approval(&session);
        let state = Arc::new(AppState {
            sessions: Mutex::new(HashMap::from([(session.id.clone(), Arc::clone(&session))])),
            dir: overlay.path.join("data/agent/sessions"),
            terminals: Arc::new(Terminals::new(
                overlay.path.join("data/agent/terminals.jsonl"),
                terminal::Presets::defaults(),
            )),
        });
        let _ = approve(
            State(state),
            AxumPath(session.id.clone()),
            Json(ApproveRequest {
                approval_id,
                allow: true,
                scope: Some("once".into()),
            }),
        )
        .await
        .unwrap();
        worker.join().unwrap();
        model.join().unwrap();

        assert_eq!(
            event_types(&session),
            [
                "turn",
                "tool_call",
                "needs_approval",
                "approved",
                "tool_result",
                "text",
                "done"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(session.root.join("result.txt")).unwrap(),
            "written by tool"
        );
        let sessions_dir = overlay.path.join("data/agent/sessions").join("fixture");
        let turns_file = sessions_dir.join(format!("{}.jsonl", session.id));
        assert!(std::fs::read_to_string(&turns_file)
            .unwrap()
            .contains("The file was written."));
        let restored =
            Session::load(&sessions_dir, &session.id).expect("session is readable after restart");
        assert!(restored
            .transcript()
            .to_string()
            .contains("The file was written."));
    }

    #[test]
    fn policy_decision_from_another_thread_wakes_the_approval_gate() {
        let (endpoint, model) = fake_model();
        let overlay = TestOverlay::new(&endpoint);
        let session = Arc::new(session_for(&overlay));
        let worker = start_turn(Arc::clone(&session));
        let approval_id = wait_for_approval(&session);
        let policy_root = overlay.path.clone();
        std::thread::spawn(move || {
            PolicyFiles::new(policy_root)
                .decide(&approval_id, true)
                .unwrap();
        })
        .join()
        .unwrap();
        worker.join().unwrap();
        model.join().unwrap();

        assert!(
            session.root.join("result.txt").exists(),
            "events: {:?}",
            event_types(&session)
        );
        let events = event_types(&session);
        assert!(events
            .windows(3)
            .any(|window| window == ["needs_approval", "approved", "tool_result"]));
        assert_eq!(events.last().map(String::as_str), Some("done"));
    }

    #[test]
    fn a_root_is_expanded_but_a_relative_path_is_not_guessed_at() {
        let home = std::env::var_os("HOME").expect("HOME is set in this environment");
        assert_eq!(
            expand("~/Developer/Sjel"),
            PathBuf::from(&home).join("Developer/Sjel")
        );
        assert_eq!(expand("/tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(expand("relative/x"), PathBuf::from("relative/x"));
        // Bare `~` is not a path this expands: guessing at it would be worse than failing.
        assert_eq!(expand("~"), PathBuf::from("~"));
    }

    #[test]
    fn the_manifest_names_every_route_it_serves() {
        let manifest = futures::executor::block_on(routes()).0;
        let paths: Vec<&str> = manifest["routes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|route| route["path"].as_str().unwrap())
            .collect();
        for path in [
            "/health",
            "/ready",
            "/routes",
            "/api/agent/sessions",
            "/api/agent/sessions/{id}",
            "/api/agent/sessions/{id}/transcript",
            "/api/agent/sessions/{id}/stream",
            "/api/agent/sessions/{id}/prompt",
            "/api/agent/sessions/{id}/approve",
            "/api/agent/sessions/{id}/steer",
            "/api/agent/sessions/{id}/stop",
            "/api/agent/terminals",
            "/api/agent/terminals/{id}",
            "/api/agent/terminals/{id}/stream",
            "/api/agent/terminals/{id}/input",
            "/api/agent/terminals/{id}/resize",
        ] {
            assert!(
                paths.contains(&path),
                "{path} is not in the manifest: {paths:?}"
            );
        }
    }

    /// The route layer around the pty: open, list, type into it, resize it, end it.
    #[tokio::test]
    async fn a_terminal_route_opens_types_resizes_and_closes() {
        let presets =
            terminal::Presets::parse("[terminal.presets]\nprobe = \"printf probe-ok\\n\"\n");
        let state = Arc::new(AppState {
            sessions: Mutex::new(HashMap::new()),
            dir: std::env::temp_dir().join("sjel-agent-route-test"),
            terminals: Arc::new(Terminals::new(
                std::env::temp_dir().join(format!(
                    "sjel-agent-route-audit-{}.jsonl",
                    std::process::id()
                )),
                presets,
            )),
        });

        let refused = open_terminal(
            State(Arc::clone(&state)),
            None,
            Json(OpenTerminal {
                preset: "nope".into(),
                cwd: None,
                cols: None,
                rows: None,
            }),
        )
        .await
        .expect_err("an unknown preset is refused");
        assert_eq!(refused.0, StatusCode::BAD_REQUEST);

        let opened = open_terminal(
            State(Arc::clone(&state)),
            None,
            Json(OpenTerminal {
                preset: "probe".into(),
                cwd: None,
                cols: Some(90),
                rows: Some(30),
            }),
        )
        .await
        .unwrap();
        let id = opened.0["id"].as_str().unwrap().to_owned();

        let listed = list_terminals(State(Arc::clone(&state))).await.0;
        let rows = listed.as_array().unwrap();
        assert_eq!(rows.len(), 1, "{listed}");
        assert_eq!(rows[0]["preset"], "probe");
        assert_eq!(rows[0]["cwd"], std::env::var("HOME").unwrap());

        // A terminal that runs one command ends on its own, and its output is at the offset a
        // client would read from.
        let terminal = state.terminals.get(&id).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = String::new();
        let mut cursor = terminal.first_offset();
        while std::time::Instant::now() < deadline {
            if let ReadOutcome::Output { offset, bytes } = terminal.read(cursor, 4096) {
                cursor = offset + bytes.len() as u64;
                seen.push_str(&String::from_utf8_lossy(&bytes));
            }
            if seen.contains("probe-ok") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(seen.contains("probe-ok"), "{seen}");

        let bad = terminal_input(
            State(Arc::clone(&state)),
            AxumPath(id.clone()),
            Json(TerminalInput {
                data: "not base64!!".into(),
            }),
        )
        .await
        .expect_err("a body that is not base64 is refused");
        assert_eq!(bad.0, StatusCode::BAD_REQUEST);

        terminal_resize(
            State(Arc::clone(&state)),
            AxumPath(id.clone()),
            Json(TerminalResize {
                cols: 100,
                rows: 40,
            }),
        )
        .await
        .unwrap();

        let missing = close_terminal(State(Arc::clone(&state)), AxumPath("nope".into()))
            .await
            .expect_err("an unknown id is a 404");
        assert_eq!(missing.0, StatusCode::NOT_FOUND);
        close_terminal(State(Arc::clone(&state)), AxumPath(id.clone()))
            .await
            .unwrap();
        assert!(state.terminals.get(&id).is_none());
    }

    /// A terminal is a shell, so no mode may admit an agent token to one. The list is checked
    /// against the manifest rather than restated, so a terminal route added without closing it
    /// fails here instead of opening a shell quietly.
    #[test]
    fn every_terminal_route_is_closed_to_an_agent_token() {
        let manifest = futures::executor::block_on(routes()).0;
        let mut checked = 0;
        for route in manifest["routes"].as_array().unwrap() {
            let path = route["path"].as_str().unwrap();
            if !path.starts_with("/api/agent/terminals") {
                continue;
            }
            let method = route["method"].as_str().unwrap();
            checked += 1;
            assert!(
                AGENT_ROUTES.closes(method, path),
                "{method} {path} would hand an agent token a shell"
            );
        }
        assert_eq!(checked, 7, "the manifest lost a terminal route: {checked}");

        // The rule is narrower than the capability: a session route stays reachable.
        assert!(!AGENT_ROUTES.closes("POST", "/api/agent/sessions/{id}/prompt"));
    }
}

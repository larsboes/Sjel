//! Sjel's capabilities as MCP tools over stdio (ISA ISC-40, F10) — `tools/sjel-mcp.ts` ported on
//! 2026-10-04.
//!
//! One stdio MCP server over every capability whose gate admits the agent. The tools are built
//! from each capability's own `GET /routes`, so a new route is a new tool with no list to keep in
//! step. What the server offers follows the owner's mode for that capability (set on the Systems
//! page; read here from the copy in `<overlay>/data/agent-modes.json`): `off` offers nothing,
//! `read-only` offers the reads, `ask` and `auto` offer the writes too. The gate enforces the same
//! mode on every call, so this list is a convenience, not the boundary.
//!
//! Every call carries the agent token, read from the login Keychain through
//! `tools/capability-auth` and held in this process only. Answers come back pseudonymized by the
//! gate (ISA F9). A write that waits for the owner answers 202; this server repeats it with
//! `X-Sjel-Approval` until the owner decides in the menu-bar app or on the Systems page.
//!
//! MCP's stdio transport is newline-delimited JSON-RPC. The three methods an agent needs
//! (`initialize`, `tools/list`, `tools/call`) are implemented here directly, so the server adds no
//! protocol dependency.
//!
//! One thread per request, as the TypeScript's un-awaited `handle()` was: an ask-mode write sleeps
//! up to 120 s between polls, and a `ping` or a second `tools/call` must not queue behind it. The
//! read loop joins them before it returns, so a handler that is still waiting for an owner's Allow
//! when stdin closes still gets to answer — which is exactly the window `sjel mcp register`'s
//! handshake closes stdin in.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use sjel_http::Purpose;

use crate::tools::{call_url, tools_for, Gate, Mode, Route, Tool};

/// How long a write may keep answering 202 before the server hands the decision back to the
/// agent, and how often it asks in the meantime.
const APPROVAL_WAIT: Duration = Duration::from_secs(120);
const APPROVAL_POLL: Duration = Duration::from_secs(3);

/// The per-request timeout the client carries. A capability that has not answered in a minute is
/// not going to.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

const INSTRUCTIONS: &str = "Sjel's capabilities on this machine. Answers are pseudonymized: pass \
     tokens back unchanged. Writes follow the owner's per-capability mode and may wait for their \
     Allow.";

/// One capability that answered `/routes`, and the tools it offers under its mode.
struct Registered {
    base: String,
    tools: Vec<Tool>,
}

type Registry = Arc<Mutex<BTreeMap<String, Registered>>>;
type Output = Arc<Mutex<io::Stdout>>;

/// The standard input loop. Returns when stdin ends, after every in-flight request has answered.
pub fn serve() {
    let registered: Registry = Arc::new(Mutex::new(BTreeMap::new()));
    let out: Output = Arc::new(Mutex::new(io::stdout()));
    let mut handlers: Vec<JoinHandle<()>> = Vec::new();

    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let registered = Arc::clone(&registered);
        let out = Arc::clone(&out);
        handlers.push(thread::spawn(move || {
            match serde_json::from_str::<Value>(&line) {
                Ok(message) => handle(&message, &registered, &out),
                // A line that is not JSON is answerable, so it is answered rather than dropped.
                Err(_) => send(
                    &out,
                    json!({"jsonrpc": "2.0", "id": null,
                       "error": {"code": -32700, "message": "parse error"}}),
                ),
            }
        }));
    }

    for handler in handlers {
        let _ = handler.join();
    }
}

fn send(out: &Output, message: Value) {
    let mut stdout = out.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = writeln!(stdout, "{message}");
    // Flushed per message: a harness reads a reply when it arrives, not when a buffer fills.
    let _ = stdout.flush();
}

/// Why a request could not be answered. The two codes are JSON-RPC's own.
enum RpcError {
    MethodNotFound(String),
    Internal(String),
}

/// One request. `id`, `method` and `params` are the only fields MCP uses; everything else in the
/// object is ignored, as JSON-RPC says it may be.
fn handle(message: &Value, registered: &Registry, out: &Output) {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    match dispatch(&method, &params, registered) {
        Ok(result) => {
            if let Some(id) = id {
                send(out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
        }
        // A write that failed is a tool result an agent can read, not a transport error, and a
        // notification has no id to fail against either.
        Err(error) => {
            let Some(id) = id else { return };
            match error {
                RpcError::MethodNotFound(method) => send(
                    out,
                    json!({"jsonrpc": "2.0", "id": id,
                           "error": {"code": -32601, "message": format!("method not found: {method}")}}),
                ),
                RpcError::Internal(message) if method == "tools/call" => send(
                    out,
                    json!({"jsonrpc": "2.0", "id": id,
                           "result": {"content": [{"type": "text", "text": message}], "isError": true}}),
                ),
                RpcError::Internal(message) => send(
                    out,
                    json!({"jsonrpc": "2.0", "id": id,
                           "error": {"code": -32603, "message": message}}),
                ),
            }
        }
    }
}

fn dispatch(method: &str, params: &Value, registered: &Registry) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18"),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "sjel", "version": "0.1.0"},
            "instructions": INSTRUCTIONS,
        })),
        "tools/list" => {
            let found = discover().map_err(RpcError::Internal)?;
            let answer = json!({
                "tools": found
                    .values()
                    .flat_map(|entry| entry.tools.iter())
                    .map(|tool| json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                    }))
                    .collect::<Vec<_>>()
            });
            *lock(registered) = found;
            Ok(answer)
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args: Map<String, Value> = match params.get("arguments") {
                Some(Value::Object(map)) => map.clone(),
                _ => Map::new(),
            };
            if lock(registered).is_empty() {
                // A client that calls before it lists: ask the capabilities now rather than
                // answering "no such tool" for a tool that exists.
                let found = discover().map_err(RpcError::Internal)?;
                *lock(registered) = found;
            }
            let hit = {
                let registry = lock(registered);
                registry.values().find_map(|entry| {
                    entry
                        .tools
                        .iter()
                        .find(|tool| tool.name == name)
                        .map(|tool| (entry.base.clone(), tool.clone()))
                })
            };
            match hit {
                Some((base, tool)) => {
                    let text = call(&tool, &base, &args).map_err(RpcError::Internal)?;
                    Ok(json!({"content": [{"type": "text", "text": text}]}))
                }
                None => Ok(json!({
                    "content": [{"type": "text", "text": format!("No tool named {name}.")}],
                    "isError": true
                })),
            }
        }
        "ping" => Ok(json!({})),
        other => Err(RpcError::MethodNotFound(other.to_string())),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// --- the capability side ------------------------------------------------------------------

fn root() -> Result<PathBuf, String> {
    match std::env::var("SJEL_ROOT") {
        Ok(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err("SJEL_ROOT is unset — run `sjel mcp` through its launcher".to_string()),
    }
}

/// The overlay the launcher resolved in `tools/lib/paths.sh`. This crate does not re-derive the
/// order: that file owns it, and one resolution is the point of the launcher source line.
fn overlay() -> Result<PathBuf, String> {
    match std::env::var("SJEL_OVERLAY_ROOT") {
        Ok(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err("no overlay is configured".to_string()),
    }
}

/// The `Authorization: Bearer …` line for the agent token, read from the login Keychain.
fn authorization() -> Result<String, String> {
    let binary = root()?.join("tools/capability-auth/capability-auth");
    let output = Command::new(&binary)
        .arg("--agent")
        .output()
        .map_err(|error| format!("could not run {}: {error}", binary.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --agent failed: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let value = strip_authorization_prefix(&line);
    if !value.starts_with("Bearer ") {
        return Err("the agent is not enrolled: run `sjel agent enroll`".to_string());
    }
    Ok(value)
}

/// `/^Authorization:\s*/i`, which `tools/capability-auth` may or may not have printed.
fn strip_authorization_prefix(line: &str) -> String {
    const PREFIX: &str = "authorization:";
    if line.len() >= PREFIX.len() && line[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        line[PREFIX.len()..].trim_start().to_string()
    } else {
        line.to_string()
    }
}

/// One session id per process, sent as `X-Sjel-Agent-Session`. It scopes the gate's pseudonym
/// tokens to this conversation: the same value comes back as the same token, and a token this
/// session never issued is refused. 16 random bytes, so two sessions collide with probability
/// nothing cares about.
fn session_id() -> &'static str {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| {
        let mut bytes = [0u8; 16];
        if let Ok(mut random) = fs::File::open("/dev/urandom") {
            if random.read_exact(&mut bytes).is_ok() {
                return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            }
        }
        // No `/dev/urandom` is not a thing on any machine this runs on, but a fallback that is
        // only less random than the real one beats refusing to serve.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        format!("{nanos:x}{:x}", std::process::id())
    })
}

/// The readable copy of the modes. The real policy is under `secrets/`, which an agent session
/// cannot read; this copy only decides which tools are offered, and the gate decides the rest.
fn modes() -> Result<BTreeMap<String, Mode>, String> {
    let file = overlay()?.join("data/agent-modes.json");
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let parsed: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let mut modes = BTreeMap::new();
    if let Some(Value::Object(map)) = parsed.get("modes") {
        for (capability, value) in map {
            if let Value::String(raw) = value {
                modes.insert(capability.clone(), Mode::parse(raw));
            }
        }
    }
    Ok(modes)
}

/// Every registered gate, in filename order.
///
/// The TypeScript took `readdirSync` order, which is the filesystem's and differs between
/// machines; the set is what matters, so a sorted read is the one reproducibility this port adds.
fn gates() -> Result<Vec<Gate>, String> {
    let dir = overlay()?.join("data/agent-gates");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    let mut gates = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).map_err(|error| error.to_string())?;
        gates.push(
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", file.display()))?,
        );
    }
    Ok(gates)
}

/// Where each registered capability answers, from `tools/capability.sh registry` — the one
/// resolver of the registry, called as the launcher the other tools call.
fn bases() -> Result<BTreeMap<String, String>, String> {
    let script = root()?.join("tools/capability.sh");
    let output = Command::new(&script)
        .arg("registry")
        .output()
        .map_err(|error| format!("could not run {}: {error}", script.display()))?;
    let raw = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<Value> = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let mut bases = BTreeMap::new();
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let field = |key: &str| {
            entry
                .get(key)
                .map(crate::tools::value_as_text)
                .filter(|value| !value.is_empty())
        };
        if let Some(endpoint) = field("endpoint") {
            bases.insert(name.to_string(), endpoint);
        } else if let Some(port) = field("port") {
            bases.insert(name.to_string(), format!("http://127.0.0.1:{port}"));
        }
    }
    Ok(bases)
}

/// One request to a capability, carrying the agent's identity.
fn fetch(
    url: &str,
    method: &str,
    body: Option<&str>,
    approval: Option<&str>,
) -> Result<reqwest::blocking::Response, String> {
    let client = sjel_http::client(Purpose::new("sjel-mcp"), REQUEST_TIMEOUT)
        .map_err(|error| error.to_string())?;
    let mut request = match method {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        "HEAD" => client.head(url),
        _ => client.get(url),
    };
    request = request
        .header("Authorization", authorization()?)
        .header("X-Sjel-Agent-Session", session_id());
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/json")
            .body(body.to_string());
    }
    if let Some(approval) = approval {
        request = request.header("X-Sjel-Approval", approval);
    }
    request.send().map_err(|error| error.to_string())
}

/// Ask every gated capability what it offers. A capability that is not running offers no tools
/// now; the next `tools/list` asks again.
fn discover() -> Result<BTreeMap<String, Registered>, String> {
    let policy = modes()?;
    let where_ = bases()?;
    let mut found = BTreeMap::new();
    for gate in gates()? {
        let Some(base) = where_.get(&gate.capability) else {
            continue;
        };
        let mode = policy.get(&gate.capability).copied().unwrap_or(Mode::Auto);
        if mode == Mode::Off {
            continue;
        }
        let Ok(response) = fetch(&format!("{base}/routes"), "GET", None, None) else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(text) = response.text() else { continue };
        let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
            continue;
        };
        found.insert(
            gate.capability.clone(),
            Registered {
                base: base.clone(),
                tools: tools_for(
                    &gate.capability,
                    mode,
                    manifest.routes.as_deref().unwrap_or(&[]),
                    &gate,
                ),
            },
        );
    }
    Ok(found)
}

/// The part of a capability's `/routes` answer this server reads.
#[derive(serde::Deserialize)]
struct Manifest {
    routes: Option<Vec<Route>>,
}

/// One tool call, including the wait for an owner's Allow.
fn call(tool: &Tool, base: &str, args: &Map<String, Value>) -> Result<String, String> {
    let url = call_url(base, &tool.path, args)?;
    let read = matches!(tool.method.as_str(), "GET" | "HEAD");
    let body = if read {
        None
    } else {
        Some(
            args.get("body")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| json!({}))
                .to_string(),
        )
    };

    let mut response = fetch(&url, &tool.method, body.as_deref(), None)?;
    let started = Instant::now();
    loop {
        let status = response.status();
        let text = response.text().map_err(|error| error.to_string())?;
        if status.as_u16() != 202 {
            return Ok(if status.is_success() {
                text
            } else {
                format!("HTTP {}: {text}", status.as_u16())
            });
        }
        // 202: the gate is holding the write for the owner's Allow, and named the approval.
        let ask: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let Some(approval) = ask
            .get("approval")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            // No id to repeat with, so the body is the answer. The gate always sets one, so this
            // is a capability answering 202 for some other reason.
            return Ok(text);
        };
        if started.elapsed() > APPROVAL_WAIT {
            return Ok(format!(
                "Waiting for the owner to allow this write (approval {approval}). Ask them to \
                 decide in the Sjel menu-bar app, then call this tool again."
            ));
        }
        thread::sleep(APPROVAL_POLL);
        response = fetch(&url, &tool.method, body.as_deref(), Some(&approval))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_authorization_line_is_stripped_whatever_its_case() {
        assert_eq!(
            strip_authorization_prefix("Authorization: Bearer x"),
            "Bearer x"
        );
        assert_eq!(
            strip_authorization_prefix("authorization:   Bearer y"),
            "Bearer y"
        );
        assert_eq!(strip_authorization_prefix("Bearer z"), "Bearer z");
    }

    #[test]
    fn initialize_echoes_the_protocol_and_names_the_server() {
        let registered: Registry = Arc::new(Mutex::new(BTreeMap::new()));
        let answer = dispatch(
            "initialize",
            &json!({"protocolVersion": "2024-11-05"}),
            &registered,
        )
        .ok()
        .unwrap();
        assert_eq!(answer["protocolVersion"], json!("2024-11-05"));
        assert_eq!(answer["serverInfo"]["name"], json!("sjel"));
        let defaulted = dispatch("initialize", &json!({}), &registered)
            .ok()
            .unwrap();
        assert_eq!(defaulted["protocolVersion"], json!("2025-06-18"));
    }

    #[test]
    fn an_unknown_method_is_json_rpcs_own_error() {
        let registered: Registry = Arc::new(Mutex::new(BTreeMap::new()));
        let error = dispatch("bogus/method", &json!({}), &registered).err();
        assert!(matches!(error, Some(RpcError::MethodNotFound(m)) if m == "bogus/method"));
    }

    #[test]
    fn ping_answers_with_an_empty_object() {
        let registered: Registry = Arc::new(Mutex::new(BTreeMap::new()));
        assert_eq!(
            dispatch("ping", &json!({}), &registered).ok().unwrap(),
            json!({})
        );
    }
}

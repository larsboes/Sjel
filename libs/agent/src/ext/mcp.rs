//! MCP client (F4, D10): speaks JSON-RPC 2.0 over stdio and HTTP.
//!
//! Exposes tools discovered from external MCP servers as `<server>__<tool>` (AGT-14).
//! Blocking, in-process, without async runtimes or third-party SDK dependencies (C3, D10).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use sjel_agent::{Extension, Tool};

/// The name `agent.toml` and `--ext` use.
pub const NAME: &str = "mcp";

/// Default timeout in seconds for an MCP call (AGT-15).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Configuration for an MCP server in `agent.toml`.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct ServerConfig {
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// JSON-RPC transport for talking to an MCP server.
pub trait McpTransport: Send + Sync {
    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<Value, String>;
}

/// Child process stdio transport.
pub struct StdioTransport {
    inner: Mutex<StdioInner>,
}

struct StdioInner {
    #[allow(dead_code)]
    child: Child,
    stdin: BufWriter<ChildStdin>,
    rx: mpsc::Receiver<std::io::Result<String>>,
    next_id: u64,
}

impl StdioTransport {
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn `{command}`: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to capture child stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture child stdout".to_string())?;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let is_err = line.is_err();
                if tx.send(line).is_err() || is_err {
                    break;
                }
            }
        });

        Ok(Self {
            inner: Mutex::new(StdioInner {
                child,
                stdin: BufWriter::new(stdin),
                rx,
                next_id: 1,
            }),
        })
    }
}

impl McpTransport for StdioTransport {
    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<Value, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "poisoned stdio transport lock".to_string())?;

        let id = inner.next_id;
        inner.next_id += 1;

        let mut req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            req["params"] = p;
        }

        let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        writeln!(inner.stdin, "{line}").map_err(|e| format!("failed to write to child: {e}"))?;
        inner
            .stdin
            .flush()
            .map_err(|e| format!("failed to flush child stdin: {e}"))?;

        let start = Instant::now();
        while start.elapsed() < timeout {
            if stop.load(Ordering::Relaxed) {
                return Err("stopped by user".to_string());
            }

            match inner.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(raw_line)) => {
                    let trimmed = raw_line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let val: Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if val.get("id").and_then(Value::as_u64) == Some(id) {
                        if let Some(err) = val.get("error") {
                            let msg = err
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("MCP error");
                            return Err(msg.to_string());
                        }
                        return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
                Ok(Err(e)) => return Err(format!("child stdout error: {e}")),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("child closed stdout unexpectedly".to_string())
                }
            }
        }

        Err(format!("MCP call `{method}` timed out after {timeout:?}"))
    }
}

/// HTTP JSON-RPC 2.0 transport.
pub struct HttpTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::blocking::Client,
    next_id: AtomicU64,
}

impl HttpTransport {
    pub fn new(
        url: String,
        headers: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<Self, String> {
        const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("agent-mcp");
        let client = sjel_http::client(PURPOSE, timeout)
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Self {
            url,
            headers,
            client,
            next_id: AtomicU64::new(1),
        })
    }
}

impl McpTransport for HttpTransport {
    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            payload["params"] = p;
        }

        let client = self.client.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut req = client.post(&url).json(&payload);
            for (k, v) in headers {
                req = req.header(&k, &v);
            }
            let res = req.send().and_then(|r| r.json::<Value>());
            let _ = tx.send(res);
        });

        let start = Instant::now();
        while start.elapsed() < timeout {
            if stop.load(Ordering::Relaxed) {
                return Err("stopped by user".to_string());
            }

            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(val)) => {
                    if let Some(err) = val.get("error") {
                        let msg = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("MCP error");
                        return Err(msg.to_string());
                    }
                    return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(Err(e)) => return Err(format!("HTTP request error: {e}")),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("HTTP transport thread ended unexpectedly".to_string())
                }
            }
        }

        Err(format!("MCP call `{method}` timed out after {timeout:?}"))
    }
}

/// Dynamic tool representing an MCP tool exposed by a server.
pub struct McpTool {
    scoped_name: String,
    raw_name: String,
    description: String,
    schema: Value,
    transport: Arc<dyn McpTransport>,
    timeout: Duration,
    stop: Arc<AtomicBool>,
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.scoped_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.schema.clone()
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let params = json!({
            "name": self.raw_name,
            "arguments": args,
        });

        let result = self
            .transport
            .call("tools/call", Some(params), self.timeout, &self.stop)?;

        // Format MCP CallToolResult
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            let mut text_parts: Vec<String> = Vec::new();
            for item in content {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(txt) = item.get("text").and_then(Value::as_str) {
                        text_parts.push(txt.to_string());
                    }
                } else {
                    text_parts.push(item.to_string());
                }
            }
            let text = text_parts.join("\n");
            if result.get("isError").and_then(Value::as_bool) == Some(true) {
                return Err(text);
            }
            return Ok(text);
        }

        if let Some(text) = result.as_str() {
            return Ok(text.to_string());
        }

        Ok(result.to_string())
    }
}

/// The MCP extension holding registered tools from all enabled MCP servers.
pub struct Mcp {
    tools: Vec<McpTool>,
}

impl Mcp {
    pub fn load(
        servers: &HashMap<String, ServerConfig>,
        stop: &Arc<AtomicBool>,
    ) -> (Self, Vec<String>) {
        let mut tools: Vec<McpTool> = Vec::new();
        let mut problems: Vec<String> = Vec::new();

        for (name, config) in servers {
            let timeout = Duration::from_secs(config.timeout);
            let transport: Result<Arc<dyn McpTransport>, String> =
                if let Some(ref cmd) = config.command {
                    StdioTransport::spawn(cmd, &config.args, &config.env)
                        .map(|t| Arc::new(t) as Arc<dyn McpTransport>)
                } else if let Some(ref url) = config.url {
                    HttpTransport::new(url.clone(), config.headers.clone(), timeout)
                        .map(|t| Arc::new(t) as Arc<dyn McpTransport>)
                } else {
                    Err(format!(
                        "server `{name}` must define either `command` or `url`"
                    ))
                };

            let transport = match transport {
                Ok(t) => t,
                Err(e) => {
                    problems.push(format!("{name}: {e}"));
                    continue;
                }
            };

            // Initialize handshake
            let init_params = json!({
                "protocolVersion": "2024-11-05",
                "clientInfo": { "name": "sjel-agent", "version": "0.1.0" },
                "capabilities": {}
            });
            if let Err(e) = transport.call("initialize", Some(init_params), timeout, stop) {
                problems.push(format!("{name} initialize failed: {e}"));
                continue;
            }

            // Discover tools
            let list_res = match transport.call("tools/list", Some(json!({})), timeout, stop) {
                Ok(v) => v,
                Err(e) => {
                    problems.push(format!("{name} tools/list failed: {e}"));
                    continue;
                }
            };

            let Some(tool_list) = list_res.get("tools").and_then(Value::as_array) else {
                problems.push(format!("{name}: missing `tools` array in tools/list"));
                continue;
            };

            for t in tool_list {
                let Some(tool_name) = t.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let scoped_name = format!("{name}__{tool_name}");
                let description = t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let schema = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"}));

                tools.push(McpTool {
                    scoped_name,
                    raw_name: tool_name.to_string(),
                    description,
                    schema,
                    transport: Arc::clone(&transport),
                    timeout,
                    stop: Arc::clone(stop),
                });
            }
        }

        (Self { tools }, problems)
    }
}

impl Extension for Mcp {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools
            .iter()
            .cloned()
            .map(|t| Box::new(t) as Box<dyn Tool>)
            .collect()
    }
}

impl Clone for McpTool {
    fn clone(&self) -> Self {
        Self {
            scoped_name: self.scoped_name.clone(),
            raw_name: self.raw_name.clone(),
            description: self.description.clone(),
            schema: self.schema.clone(),
            transport: Arc::clone(&self.transport),
            timeout: self.timeout,
            stop: Arc::clone(&self.stop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOCK_SERVER_SCRIPT: &str = r#"
import sys, json, time

for line in sys.stdin:
    if not line.strip():
        continue
    req = json.loads(line)
    req_id = req.get("id")
    method = req.get("method")
    
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "mock", "version": "1.0"}
            }
        }), flush=True)
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echoes argument",
                        "inputSchema": {"type": "object", "properties": {"msg": {"type": "string"}}}
                    },
                    {
                        "name": "slow",
                        "description": "Sleeps then responds",
                        "inputSchema": {"type": "object"}
                    }
                ]
            }
        }), flush=True)
    elif method == "tools/call":
        params = req.get("params", {})
        tool_name = params.get("name")
        args = params.get("arguments", {})
        if tool_name == "slow":
            time.sleep(3)
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"content": [{"type": "text", "text": "slow done"}]}
            }), flush=True)
        else:
            msg = args.get("msg", "default")
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"content": [{"type": "text", "text": f"echo: {msg}"}]}
            }), flush=True)
"#;

    #[test]
    fn stdio_mcp_discovers_tools_and_calls_them() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut servers = HashMap::new();
        servers.insert(
            "test".to_string(),
            ServerConfig {
                command: Some("python3".to_string()),
                args: vec![
                    "-u".to_string(),
                    "-c".to_string(),
                    MOCK_SERVER_SCRIPT.to_string(),
                ],
                timeout: 10,
                ..Default::default()
            },
        );

        let (mcp, problems) = Mcp::load(&servers, &stop);
        assert!(problems.is_empty(), "problems: {problems:?}");

        let tools = mcp.tools();
        assert_eq!(tools.len(), 2, "must discover 2 tools");

        let echo_tool = tools
            .iter()
            .find(|t| t.name() == "test__echo")
            .expect("test__echo exists");
        assert_eq!(echo_tool.description(), "Echoes argument");

        let res = echo_tool
            .run(&json!({"msg": "hello"}))
            .expect("tool call succeeds");
        assert_eq!(res, "echo: hello");
    }

    #[test]
    fn stdio_mcp_times_out_when_call_exceeds_timeout() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut servers = HashMap::new();
        servers.insert(
            "test_slow".to_string(),
            ServerConfig {
                command: Some("python3".to_string()),
                args: vec![
                    "-u".to_string(),
                    "-c".to_string(),
                    MOCK_SERVER_SCRIPT.to_string(),
                ],
                timeout: 1, // 1 second timeout
                ..Default::default()
            },
        );

        let (mcp, problems) = Mcp::load(&servers, &stop);
        assert!(problems.is_empty(), "problems: {problems:?}");

        let tools = mcp.tools();
        let slow_tool = tools
            .iter()
            .find(|t| t.name() == "test_slow__slow")
            .expect("slow tool exists");

        let start = Instant::now();
        let res = slow_tool.run(&json!({}));
        let elapsed = start.elapsed();

        assert!(res.is_err(), "should time out");
        let err = res.unwrap_err();
        assert!(err.contains("timed out"), "err was: {err}");
        assert!(
            elapsed < Duration::from_secs(3),
            "timed out around 1s, took {elapsed:?}"
        );
    }

    #[test]
    fn stdio_mcp_stop_flag_interrupts_within_one_second_and_leaves_server_alive() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut servers = HashMap::new();
        servers.insert(
            "test_stop".to_string(),
            ServerConfig {
                command: Some("python3".to_string()),
                args: vec![
                    "-u".to_string(),
                    "-c".to_string(),
                    MOCK_SERVER_SCRIPT.to_string(),
                ],
                timeout: 10,
                ..Default::default()
            },
        );

        let (mcp, problems) = Mcp::load(&servers, &stop);
        assert!(problems.is_empty(), "problems: {problems:?}");

        let tools = mcp.tools();
        let slow_tool = tools
            .iter()
            .find(|t| t.name() == "test_stop__slow")
            .expect("slow tool exists");
        let echo_tool = tools
            .iter()
            .find(|t| t.name() == "test_stop__echo")
            .expect("echo tool exists");

        // Set stop flag 100ms after starting slow call
        let stop_clone = Arc::clone(&stop);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            stop_clone.store(true, Ordering::Relaxed);
        });

        let start = Instant::now();
        let res = slow_tool.run(&json!({}));
        let elapsed = start.elapsed();

        assert!(res.is_err(), "call should be stopped");
        assert_eq!(res.unwrap_err(), "stopped by user");
        assert!(
            elapsed < Duration::from_secs(1),
            "stop must respond in < 1 s (AGT-16), took {elapsed:?}"
        );

        // Now clear stop flag and call echo_tool: the server must still be alive!
        stop.store(false, Ordering::Relaxed);
        let echo_res = echo_tool
            .run(&json!({"msg": "still alive"}))
            .expect("second call succeeds");
        assert_eq!(echo_res, "echo: still alive");
    }

    #[test]
    fn http_mcp_roundtrip() {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind tcp");
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.read(&mut buf) else { break };
                let req_str = String::from_utf8_lossy(&buf[..n]);

                // Parse body
                if let Some(body_start) = req_str.find("\r\n\r\n") {
                    let body = &req_str[body_start + 4..];
                    if let Ok(json_body) = serde_json::from_str::<Value>(body) {
                        let id = json_body.get("id").and_then(Value::as_u64).unwrap_or(1);
                        let method = json_body
                            .get("method")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let resp_body = match method {
                            "initialize" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {"protocolVersion": "2024-11-05", "capabilities": {}}
                            }),
                            "tools/list" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {"tools": [{"name": "ping", "description": "pong", "inputSchema": {}}]}
                            }),
                            "tools/call" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {"content": [{"type": "text", "text": "pong"}]}
                            }),
                            _ => {
                                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not found"}})
                            }
                        };
                        let body_bytes = serde_json::to_vec(&resp_body).unwrap();
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body_bytes.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&body_bytes);
                    }
                }
            }
        });

        let stop = Arc::new(AtomicBool::new(false));
        let mut servers = HashMap::new();
        servers.insert(
            "remote".to_string(),
            ServerConfig {
                url: Some(format!("http://127.0.0.1:{port}")),
                timeout: 5,
                ..Default::default()
            },
        );

        let (mcp, problems) = Mcp::load(&servers, &stop);
        assert!(problems.is_empty(), "problems: {problems:?}");

        let tools = mcp.tools();
        assert_eq!(tools.len(), 1);
        let ping_tool = &tools[0];
        assert_eq!(ping_tool.name(), "remote__ping");
        let res = ping_tool.run(&json!({})).unwrap();
        assert_eq!(res, "pong");
    }
}

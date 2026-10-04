//! One agent loop, shared by every front end that lets a model call tools.
//!
//! The loop is the part pi, Claude Code and the Sjel assistant all have in common: send the
//! conversation, run the tool calls the model asks for, append their results, and repeat
//! until the model answers without a tool call. What differs between front ends is the tool
//! list, so that is the argument. The coding CLI (`src/main.rs`) passes [`tools::coding`].
//! The assistant capability passes tools that reach Sjel's own APIs, and never `bash`.
//!
//! The wire format is streamed OpenAI chat completions with `tools`, which every backend in
//! `libs/inference` speaks (oMLX, Ollama's `/v1` shim, and the hosted providers). The model
//! and endpoint come from a [`ResolvedRole`], so moving the agent between a local and a
//! hosted model is an overlay edit, not a code change.

pub mod session;
pub mod stream;
pub mod tools;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sjel_inference::ResolvedRole;

const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("agent");
/// The whole streamed response, not the first byte. A local 26B model writing a long file
/// needs minutes.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// How much of an error body reaches the error message. Provider errors can echo the request.
const ERROR_BODY_LIMIT: usize = 2_000;

/// One chat-completions message. Unknown fields such as `reasoning_content` are dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        content: Option<String>,
        /// `Option`, not a defaulted `Vec`: some providers send `"tool_calls": null`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

fn function_kind() -> String {
    "function".into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// A JSON document in a string, as the API sends it. Parsed only when the call runs.
    pub arguments: String,
}

/// What a front end sees while a run is in progress.
#[derive(Debug)]
pub enum Event<'a> {
    /// A fragment of the answer, as the model streams it.
    Text(&'a str),
    /// A fragment of `reasoning_content`, from models that stream their thinking separately.
    Reasoning(&'a str),
    /// A call, complete, before it runs.
    ToolCall(&'a ToolCall),
    ToolResult {
        call: &'a ToolCall,
        content: &'a str,
    },
}

/// A capability the model can call.
///
/// `run` returns `Err` with text for the model, not for the operator: a missing file or a
/// non-unique edit is something the model reads and corrects on its next turn. Nothing a
/// tool returns stops the loop.
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON Schema for the arguments object.
    fn parameters(&self) -> Value;
    fn run(&self, args: &Value) -> Result<String, String>;
    /// True when the tool changes nothing, so calls to it can run at the same time.
    /// A turn runs its calls in parallel only when every call in it says yes.
    fn read_only(&self) -> bool {
        false
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("the model endpoint did not answer: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the model stream broke: {0}")]
    Io(#[from] std::io::Error),
    #[error("the model endpoint returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the model endpoint returned no usable message: {0}")]
    Malformed(String),
    #[error("the model still called tools after {0} turns")]
    TurnLimit(usize),
    #[error("stopped by the user")]
    Stopped,
}

pub struct Agent {
    role: ResolvedRole,
    client: reqwest::blocking::Client,
    tools: Vec<Box<dyn Tool>>,
    /// Set from outside (a Ctrl-C handler) to end the current run. The caller clears it.
    stop: Arc<AtomicBool>,
    /// One turn is one completion request. Stops a model that calls tools forever.
    pub max_turns: usize,
}

impl Agent {
    /// `stop` is shared with whatever ends a run early, and with tools that must end with it
    /// (see [`tools::coding`]).
    pub fn new(
        role: ResolvedRole,
        tools: Vec<Box<dyn Tool>>,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, AgentError> {
        Ok(Self {
            role,
            client: sjel_http::client(PURPOSE, REQUEST_TIMEOUT)?,
            tools,
            stop,
            max_turns: 50,
        })
    }

    /// Relaxed: the flag carries no data, only "stop", and every check rereads it.
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn model(&self) -> &str {
        &self.role.model
    }

    /// Runs the conversation until the model answers without a tool call.
    ///
    /// On `Err`, `messages` holds every complete message up to the failure. A front end that
    /// wants to retry the turn truncates back to the length it had before the call.
    ///
    /// On [`AgentError::Stopped`], `messages` stays valid to send again: text streamed before
    /// the stop is kept, half-streamed tool calls are dropped, and a call that never ran gets
    /// a result that says so. The conversation can continue with the next prompt.
    pub fn run(
        &self,
        messages: &mut Vec<Message>,
        mut on: impl FnMut(Event<'_>),
    ) -> Result<(), AgentError> {
        for _ in 0..self.max_turns {
            let reply = self.complete(messages, &mut on)?;
            if self.stopped() {
                if let Message::Assistant {
                    content: Some(text),
                    ..
                } = reply
                {
                    messages.push(Message::Assistant {
                        content: Some(text),
                        tool_calls: None,
                    });
                }
                return Err(AgentError::Stopped);
            }
            let calls = match &reply {
                Message::Assistant {
                    tool_calls: Some(calls),
                    ..
                } if !calls.is_empty() => calls.clone(),
                _ => {
                    messages.push(reply);
                    return Ok(());
                }
            };
            messages.push(reply);
            for call in &calls {
                on(Event::ToolCall(call));
            }
            for (call, content) in calls.iter().zip(self.call_all(&calls)) {
                on(Event::ToolResult {
                    call,
                    content: &content,
                });
                messages.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
            }
            if self.stopped() {
                return Err(AgentError::Stopped);
            }
        }
        Err(AgentError::TurnLimit(self.max_turns))
    }

    /// Results in call order. Read-only batches run on scoped threads: a model that asks for
    /// five files at once waits for the slowest read, not for the sum of them.
    fn call_all(&self, calls: &[ToolCall]) -> Vec<String> {
        let parallel = calls.len() > 1
            && calls
                .iter()
                .all(|c| self.tool(&c.function.name).is_some_and(|t| t.read_only()));
        if !parallel {
            return calls.iter().map(|c| self.call(c)).collect();
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = calls
                .iter()
                .map(|c| scope.spawn(move || self.call(c)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| "error: the tool panicked".into())
                })
                .collect()
        })
    }

    fn tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    fn call(&self, call: &ToolCall) -> String {
        if self.stopped() {
            return "error: the user stopped the turn before this call ran".into();
        }
        let Some(tool) = self.tool(&call.function.name) else {
            return format!("error: there is no tool named `{}`", call.function.name);
        };
        // Some models send "" for a call with no arguments.
        let raw = call.function.arguments.trim();
        let args = if raw.is_empty() {
            Ok(json!({}))
        } else {
            serde_json::from_str(raw)
        };
        match args {
            Ok(args) => tool.run(&args).unwrap_or_else(|e| format!("error: {e}")),
            Err(e) => format!("error: the arguments are not valid JSON: {e}"),
        }
    }

    fn request_body(&self, messages: &[Message]) -> Value {
        let mut body = json!({ "model": self.role.model, "messages": messages, "stream": true });
        let Some(object) = body.as_object_mut() else {
            unreachable!("json! object literal")
        };
        if !self.tools.is_empty() {
            let tools: Vec<Value> = self
                .tools
                .iter()
                .map(|t| {
                    json!({ "type": "function", "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters(),
                    }})
                })
                .collect();
            object.insert("tools".into(), Value::Array(tools));
        }
        // Same pass-through as capabilities/assistant: the role decides template arguments and
        // extra fields, but never the fields this loop owns.
        if let Some(kwargs) = &self.role.chat_template_kwargs {
            object.insert("chat_template_kwargs".into(), kwargs.clone());
        }
        if let Some(overrides) = self
            .role
            .request_overrides
            .as_ref()
            .and_then(Value::as_object)
        {
            for (key, value) in overrides {
                if !matches!(key.as_str(), "model" | "messages" | "tools" | "stream") {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
        body
    }

    fn complete(
        &self,
        messages: &[Message],
        on: &mut impl FnMut(Event<'_>),
    ) -> Result<Message, AgentError> {
        let mut request = self
            .client
            .post(self.role.chat_completions_endpoint())
            .json(&self.request_body(messages));
        if let Some(key) = self.role.bearer_key() {
            request = request.bearer_auth(key);
        }
        let response = request.send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(AgentError::Status {
                status: status.as_u16(),
                body: tools::cap(&response.text()?, ERROR_BODY_LIMIT),
            });
        }
        stream::fold(std::io::BufReader::new(response), &self.stop, on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    struct Echo;
    impl Tool for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn description(&self) -> &'static str {
            "Returns its text argument."
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object", "properties": { "text": { "type": "string" } } })
        }
        fn run(&self, args: &Value) -> Result<String, String> {
            args["text"]
                .as_str()
                .map(str::to_owned)
                .ok_or("no text".into())
        }
        fn read_only(&self) -> bool {
            true
        }
    }

    /// Streams each canned list of chunks to the next connection and hands back each request.
    fn serve(turns: Vec<Vec<Value>>) -> (u16, std::sync::mpsc::Receiver<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for chunks in turns {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0;
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
                let mut request = vec![0; length];
                reader.read_exact(&mut request).unwrap();
                seen.send(serde_json::from_slice(&request).unwrap())
                    .unwrap();
                let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
                body.push_str("data: [DONE]\n\n");
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (port, requests)
    }

    fn role(port: u16) -> ResolvedRole {
        let config: sjel_inference::InferenceConfig = format!(
            r#"{{ "backends": {{ "local": {{ "api": "openai", "base_url": "http://127.0.0.1:{port}/v1" }} }},
                 "roles": {{ "coding": {{ "backend": "local", "model": "m" }} }} }}"#
        )
        .parse()
        .unwrap();
        config.role("coding").unwrap()
    }

    fn delta(d: Value) -> Value {
        json!({ "choices": [{ "index": 0, "delta": d }] })
    }

    #[test]
    fn loop_runs_parallel_tool_calls_and_streams_the_final_answer() {
        let call = |i: u32, id: &str, text: &str| {
            json!({ "tool_calls": [{ "index": i, "id": id, "type": "function",
                "function": { "name": "echo", "arguments": format!("{{\"text\":\"{text}\"}}") } }] })
        };
        let (port, requests) = serve(vec![
            vec![delta(call(0, "c1", "a")), delta(call(1, "c2", "b"))],
            vec![
                delta(json!({ "content": "do" })),
                delta(json!({ "content": "ne" })),
            ],
        ]);
        let agent = Agent::new(role(port), vec![Box::new(Echo)], Arc::default()).unwrap();
        let mut messages = vec![Message::User {
            content: "ping".into(),
        }];
        let mut streamed = String::new();
        agent
            .run(&mut messages, |e| {
                if let Event::Text(t) = e {
                    streamed.push_str(t)
                }
            })
            .unwrap();

        assert_eq!(streamed, "done");
        let results = messages[2..4].to_vec();
        assert_eq!(
            results,
            [
                Message::Tool {
                    tool_call_id: "c1".into(),
                    content: "a".into()
                },
                Message::Tool {
                    tool_call_id: "c2".into(),
                    content: "b".into()
                },
            ]
        );
        assert_eq!(
            messages[4],
            Message::Assistant {
                content: Some("done".into()),
                tool_calls: None
            }
        );
        let first = requests.recv().unwrap();
        assert_eq!(first["stream"], true);
        assert_eq!(first["tools"][0]["function"]["name"], "echo");
        let second = requests.recv().unwrap();
        assert_eq!(second["messages"][1]["tool_calls"][1]["id"], "c2");
        assert_eq!(second["messages"][3]["content"], "b");
    }

    /// Sets the stop flag when it runs, as a Ctrl-C during a tool call would.
    struct Halt(Arc<AtomicBool>);
    impl Tool for Halt {
        fn name(&self) -> &'static str {
            "halt"
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object" })
        }
        fn run(&self, _: &Value) -> Result<String, String> {
            self.0.store(true, Ordering::Relaxed);
            Ok("halted".into())
        }
    }

    #[test]
    fn a_stop_leaves_a_conversation_that_can_be_sent_again() {
        let call = |i: u32, id: &str, name: &str| json!({ "tool_calls": [{ "index": i, "id": id, "function": { "name": name, "arguments": "{}" } }] });
        let (port, _requests) = serve(vec![vec![
            delta(call(0, "h", "halt")),
            delta(call(1, "e", "echo")),
        ]]);
        let stop = Arc::new(AtomicBool::new(false));
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(Halt(Arc::clone(&stop))), Box::new(Echo)];
        let agent = Agent::new(role(port), tools, stop).unwrap();
        let mut messages = vec![Message::User {
            content: "go".into(),
        }];
        let err = agent.run(&mut messages, |_| {}).unwrap_err();

        assert!(matches!(err, AgentError::Stopped));
        // Every call has a result, so the next request is valid.
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[2],
            Message::Tool {
                tool_call_id: "h".into(),
                content: "halted".into()
            }
        );
        let Message::Tool { content, .. } = &messages[3] else {
            panic!()
        };
        assert!(content.contains("stopped the turn before this call ran"));
    }

    #[test]
    fn a_bad_call_becomes_text_for_the_model() {
        let agent = Agent::new(role(1), vec![Box::new(Echo)], Arc::default()).unwrap();
        let call = |name: &str, arguments: &str| ToolCall {
            id: "c".into(),
            kind: function_kind(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        };
        assert!(agent
            .call(&call("nope", "{}"))
            .contains("no tool named `nope`"));
        assert!(agent.call(&call("echo", "{bad")).contains("not valid JSON"));
        assert_eq!(agent.call(&call("echo", "")), "error: no text");
    }
}

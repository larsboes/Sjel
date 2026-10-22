//! One agent loop, shared by every front end that lets a model call tools.
//!
//! The loop is the part pi, Claude Code and the Sjel assistant all have in common: send the
//! conversation, run the tool calls the model asks for, append their results, and repeat
//! until the model answers without a tool call. What differs between front ends is the tool
//! list, so that is the argument. The coding CLI (`src/main.rs`) passes [`tools::coding`].
//! The assistant capability passes tools that reach Sjel's own APIs, and never `bash`.
//!
//! The default wire format is streamed OpenAI chat completions with `tools`, which every backend in
//! `libs/inference` speaks (oMLX, Ollama's `/v1` shim, and the hosted providers). The model
//! and endpoint come from a [`ResolvedRole`], so moving the agent between a local and a
//! hosted model is an overlay edit, not a code change.

pub mod cloud;
pub mod extension;
pub mod session;
pub mod stream;
pub mod tools;

pub use extension::{Extension, Verdict};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
const MAX_PARALLEL_TOOL_CALLS: usize = 8;
/// Bounds messages waiting for an active agent run.
const INPUT_QUEUE_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Steering,
    FollowUp,
}

struct QueuedInput {
    kind: InputKind,
    content: String,
}

/// The bounded input queue shared with a running agent.
///
/// Steering is delivered after the current batch of tool calls and before the next model
/// request. Follow-ups are delivered after the current prompt receives its final answer.
/// Steering messages take priority over follow-ups; messages of each kind retain queue order.
#[derive(Clone, Default)]
pub struct AgentInput {
    pending: Arc<Mutex<VecDeque<QueuedInput>>>,
}

/// Why a message could not be added to an [`AgentInput`] queue.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InputQueueError {
    #[error("the agent input queue is full (capacity {INPUT_QUEUE_CAPACITY})")]
    Full,
    #[error("the agent input queue is unavailable because its lock was poisoned")]
    Poisoned,
}

impl AgentInput {
    /// Queue a message for the next model request after the current tool batch.
    pub fn steer(&self, content: impl Into<String>) -> Result<(), InputQueueError> {
        self.push(InputKind::Steering, content.into())
    }

    /// Queue a message for after the current prompt receives its final answer.
    pub fn follow_up(&self, content: impl Into<String>) -> Result<(), InputQueueError> {
        self.push(InputKind::FollowUp, content.into())
    }

    fn push(&self, kind: InputKind, content: String) -> Result<(), InputQueueError> {
        let mut pending = self.pending.lock().map_err(|_| InputQueueError::Poisoned)?;
        if pending.len() >= INPUT_QUEUE_CAPACITY {
            return Err(InputQueueError::Full);
        }
        pending.push_back(QueuedInput { kind, content });
        Ok(())
    }

    fn take(&self, kind: InputKind) -> Result<Option<String>, InputQueueError> {
        let mut pending = self.pending.lock().map_err(|_| InputQueueError::Poisoned)?;
        let Some(index) = pending.iter().position(|input| input.kind == kind) else {
            return Ok(None);
        };
        Ok(pending.remove(index).map(|input| input.content))
    }

    fn take_next(&self) -> Result<Option<String>, InputQueueError> {
        if let Some(content) = self.take(InputKind::Steering)? {
            Ok(Some(content))
        } else {
            self.take(InputKind::FollowUp)
        }
    }
}

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
    fn name(&self) -> &str;
    fn description(&self) -> &str;
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
    #[error("two tools are named `{name}`: {first} and {second}")]
    DuplicateTool {
        name: String,
        first: String,
        second: String,
    },
    #[error("the model still called tools after {0} turns")]
    TurnLimit(usize),
    #[error("{0}")]
    RequestRefused(String),
    #[error("stopped by the user")]
    Stopped,
    #[error(transparent)]
    InputQueue(#[from] InputQueueError),
}

pub struct Agent {
    role: ResolvedRole,
    client: reqwest::blocking::Client,
    tools: Vec<Box<dyn Tool>>,
    extensions: Vec<Box<dyn Extension>>,
    input: AgentInput,
    /// Set from outside (a Ctrl-C handler) to end the current run. The caller clears it.
    stop: Arc<AtomicBool>,
    /// One turn is one completion request. Stops a model that calls tools forever.
    pub max_turns: usize,
}

impl Agent {
    /// `stop` is shared with whatever ends a run early, and with tools that must end with it
    /// (see [`tools::coding`]).
    ///
    /// The extensions' tools join the core tools here, and two tools may not share a name: one
    /// of them would never run, and which one depends on the order they were collected in.
    /// The error names both owners.
    pub fn new(
        role: ResolvedRole,
        mut tools: Vec<Box<dyn Tool>>,
        extensions: impl IntoIterator<Item = Box<dyn Extension>>,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, AgentError> {
        let extensions: Vec<Box<dyn Extension>> = extensions.into_iter().collect();
        // One owner string per tool, parallel to `tools`, so a collision can name both sides.
        let mut owners = vec!["the core tools".to_owned(); tools.len()];
        for extension in &extensions {
            let added = extension.tools();
            owners.extend(std::iter::repeat_n(
                format!("the extension `{}`", extension.name()),
                added.len(),
            ));
            tools.extend(added);
        }
        let mut seen: Vec<(&str, &String)> = Vec::new();
        for (tool, owner) in tools.iter().zip(&owners) {
            if let Some((_, first)) = seen.iter().find(|(name, _)| *name == tool.name()) {
                return Err(AgentError::DuplicateTool {
                    name: tool.name().to_owned(),
                    first: (*first).clone(),
                    second: owner.clone(),
                });
            }
            seen.push((tool.name(), owner));
        }
        Ok(Self {
            role,
            client: sjel_http::client(PURPOSE, REQUEST_TIMEOUT)?,
            tools,
            extensions,
            input: AgentInput::default(),
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

    /// A cloneable handle for steering this agent or queueing a follow-up while it runs.
    pub fn input_queue(&self) -> AgentInput {
        self.input.clone()
    }

    /// Replace the run's input queue before it starts, so a front end can publish a handle
    /// for steering while the agent works on another thread.
    pub fn set_input_queue(&mut self, input: AgentInput) {
        self.input = input;
    }

    /// Runs until the model answers without a tool call or queued input.
    ///
    /// On `Err`, `messages` holds every complete message up to the failure. A front end that
    /// wants to retry the turn truncates back to the length it had before the call.
    ///
    /// Inputs queued through [`Agent::input_queue`] are added as user messages. Steering is
    /// delivered between tool batches and model requests; follow-ups are delivered after a
    /// prompt receives its final answer. The queue holds at most `INPUT_QUEUE_CAPACITY` messages.
    ///
    /// On [`AgentError::Stopped`], `messages` stays valid to send again: text streamed before
    /// the stop is kept, half-streamed tool calls are dropped, and a call that never ran gets
    /// a result that says so. The conversation can continue with the next prompt.
    pub fn run(
        &self,
        messages: &mut Vec<Message>,
        on: impl FnMut(Event<'_>),
    ) -> Result<(), AgentError> {
        self.run_with_class(messages, on, None)
    }

    /// Run a conversation explicitly attested as public (c0) with a cloud role.
    ///
    /// Cloud agent mode is intentionally narrower than local mode: tool schemas/results and
    /// extensions do not yet carry per-value provenance, so this path refuses both. It reserves
    /// each provider request against the durable local daily ceiling before sending it.
    pub fn run_public(
        &self,
        messages: &mut Vec<Message>,
        on: impl FnMut(Event<'_>),
    ) -> Result<(), AgentError> {
        self.run_with_class(messages, on, Some("c0"))
    }

    fn run_with_class(
        &self,
        messages: &mut Vec<Message>,
        mut on: impl FnMut(Event<'_>),
        data_class: Option<&str>,
    ) -> Result<(), AgentError> {
        // This loop accepts arbitrary strings from the caller and from tools, but carries no
        // provenance or data-class label for either. A cloud role would therefore be able to
        // receive local/private content before its provider policy could be checked. Keep the
        // shared agent local until callers can supply a classified request and a durable
        // provider budget; the assistant capability has its own local-role check as well.
        let external = !self.role.trusted_for_every_class();
        let cloud = self.role.is_cloud_endpoint();
        if external && !cloud {
            return Err(AgentError::RequestRefused(
                "remote agent backends must use a reviewed HTTPS cloud role".into(),
            ));
        }
        if cloud {
            if data_class != Some("c0") {
                return Err(AgentError::RequestRefused(
                    "cloud agent requests require explicit public-data mode".into(),
                ));
            }
            if !self.tools.is_empty() || !self.extensions.is_empty() {
                return Err(AgentError::RequestRefused(
                    "cloud agent requests currently require no tools or extensions because their data egress is not classified".into(),
                ));
            }
        }
        // The system prompt is the front end's text, and this is the one place an extension adds
        // to it: once per run, never per request. The front end opens the conversation with it,
        // and a resumed session is given the current one, so an edited `AGENTS.md` takes effect.
        if let Some(Message::System { content }) = messages.first_mut() {
            for extension in &self.extensions {
                extension.system(content);
            }
        }
        for _ in 0..self.max_turns {
            let reply = self.complete(messages, &mut on, data_class)?;
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
                    if let Some(content) = self.input.take_next()? {
                        if self.role.is_cloud_endpoint() {
                            return Err(AgentError::RequestRefused(
                                "cloud Ask currently accepts one public prompt and cannot send queued steering or follow-up text".into(),
                            ));
                        }
                        messages.push(Message::User { content });
                        continue;
                    }
                    return Ok(());
                }
            };
            if self.role.is_cloud_endpoint() {
                return Err(AgentError::RequestRefused(
                    "cloud model requested a tool, but cloud Ask has no tools; no call or follow-up was run".into(),
                ));
            }
            messages.push(reply);
            for call in &calls {
                on(Event::ToolCall(call));
            }
            for (call, content) in calls.iter().zip(self.call_all(&calls)) {
                on(Event::ToolResult {
                    call,
                    content: &content,
                });
                // The model's copy is the one an extension may rewrite: the event above is the
                // operator's own terminal, and the session file holds what is pushed here.
                let mut content = content;
                for extension in &self.extensions {
                    extension.tool_result(call, &mut content);
                }
                messages.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
            }
            if self.stopped() {
                return Err(AgentError::Stopped);
            }
            if let Some(content) = self.input.take(InputKind::Steering)? {
                if self.role.is_cloud_endpoint() {
                    return Err(AgentError::RequestRefused(
                        "cloud Ask currently accepts one public prompt and cannot send queued steering text".into(),
                    ));
                }
                messages.push(Message::User { content });
            }
        }
        Err(AgentError::TurnLimit(self.max_turns))
    }

    /// Results stay in call order. Read-only calls run in batches capped at
    /// `min(available_parallelism, MAX_PARALLEL_TOOL_CALLS)` live threads.
    fn call_all(&self, calls: &[ToolCall]) -> Vec<String> {
        let parallel = calls.len() > 1
            && calls
                .iter()
                .all(|c| self.tool(&c.function.name).is_some_and(|t| t.read_only()));
        if !parallel {
            return calls.iter().map(|c| self.call(c)).collect();
        }

        let workers = std::thread::available_parallelism()
            .map_or(1, |count| count.get())
            .min(MAX_PARALLEL_TOOL_CALLS);
        if workers == 1 {
            return calls.iter().map(|c| self.call(c)).collect();
        }

        let mut results = Vec::with_capacity(calls.len());
        for batch in calls.chunks(workers) {
            let batch_results = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|call| scope.spawn(move || self.call(call)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| "error: the tool panicked".into())
                    })
                    .collect::<Vec<_>>()
            });
            results.extend(batch_results);
        }
        results
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
        // The gate runs before the arguments are parsed: a guard blocks the call the model
        // asked for, and a call whose arguments are unparseable is still a call it can name.
        for extension in &self.extensions {
            if let Verdict::Deny(reason) = extension.tool_call(call) {
                return if reason.is_empty() {
                    "error: the call was denied".into()
                } else {
                    reason
                };
            }
        }
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
        if self.role.uses_responses_api() {
            return self.responses_request_body(messages);
        }
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

    fn responses_request_body(&self, messages: &[Message]) -> Value {
        let mut input = Vec::new();
        for message in messages {
            match message {
                Message::System { content } => {
                    input.push(json!({"role":"developer","content":content}))
                }
                Message::User { content } => input.push(json!({"role":"user","content":content})),
                Message::Assistant {
                    content,
                    tool_calls,
                } => {
                    if let Some(content) = content {
                        input.push(json!({"role":"assistant","content":content}));
                    }
                    for call in tool_calls.iter().flatten() {
                        input.push(json!({"type":"function_call","call_id":call.id,"name":call.function.name,"arguments":call.function.arguments}));
                    }
                }
                Message::Tool {
                    tool_call_id,
                    content,
                } => input.push(
                    json!({"type":"function_call_output","call_id":tool_call_id,"output":content}),
                ),
            }
        }
        let mut body = json!({"model":self.role.model,"input":input,"stream":true,"store":false,"max_output_tokens":1024});
        if !self.tools.is_empty() {
            body["tools"] = Value::Array(
                self.tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type":"function", "name":tool.name(), "description":tool.description(),
                            "parameters":tool.parameters(), "strict":false
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = json!("auto");
        }
        if let Some(overrides) = self
            .role
            .request_overrides
            .as_ref()
            .and_then(Value::as_object)
        {
            for (key, value) in overrides {
                if !matches!(
                    key.as_str(),
                    "model"
                        | "input"
                        | "stream"
                        | "tools"
                        | "tool_choice"
                        | "store"
                        | "max_output_tokens"
                ) {
                    body[key] = value.clone();
                }
            }
        }
        body
    }

    fn complete(
        &self,
        messages: &mut Vec<Message>,
        on: &mut impl FnMut(Event<'_>),
        data_class: Option<&str>,
    ) -> Result<Message, AgentError> {
        // Every extension sees the conversation just before it is sent, and may rewrite it.
        for extension in &self.extensions {
            extension.before_request(messages);
        }
        let mut body = self.request_body(messages);
        if self.role.is_afm() || self.role.is_cloud_endpoint() {
            body[if self.role.uses_responses_api() {
                "max_output_tokens"
            } else {
                "max_tokens"
            }] = json!(1024);
        }
        self.role
            .admit_chat_request(&body)
            .map_err(AgentError::RequestRefused)?;
        let mut request = self
            .client
            .post(self.role.chat_completions_endpoint())
            .json(&body);
        if let Some(key) = self.role.bearer_key() {
            request = request.bearer_auth(key);
        }
        self.role
            .admit_chat_request(&body)
            .map_err(AgentError::RequestRefused)?;
        if self.role.is_cloud_endpoint() {
            cloud::admit_and_reserve(&self.role, &body, data_class.unwrap_or("unknown"))
                .map_err(AgentError::RequestRefused)?;
        }
        let response = request.send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(AgentError::Status {
                status: status.as_u16(),
                body: tools::cap(&response.text()?, ERROR_BODY_LIMIT),
            });
        }
        if self.role.uses_responses_api() {
            stream::fold_responses(std::io::BufReader::new(response), &self.stop, on)
        } else {
            stream::fold(std::io::BufReader::new(response), &self.stop, on)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

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

    #[test]
    fn responses_api_request_uses_native_input_and_function_shapes() {
        let mut agent = Agent::new(role(1), vec![Box::new(Echo)], [], Arc::default()).unwrap();
        agent.role.backend.api = sjel_inference::Api::OpenAiResponses;
        agent.role.request_overrides = Some(json!({
            "reasoning": { "effort": "low" }, "store": true, "model": "override"
        }));
        let body = agent.request_body(&[
            Message::System {
                content: "rules".into(),
            },
            Message::User {
                content: "hello".into(),
            },
            Message::Assistant {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "echo".into(),
                        arguments: "{}".into(),
                    },
                }]),
            },
            Message::Tool {
                tool_call_id: "call_1".into(),
                content: "ok".into(),
            },
        ]);
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][2]["type"], "function_call");
        assert_eq!(body["input"][3]["type"], "function_call_output");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["model"], "m");
        assert!(body.get("messages").is_none());
    }

    #[test]
    fn cloud_role_refuses_unclassified_conversation_before_model_or_tool_egress() {
        let mut cloud = role(1);
        cloud.backend.base_url = "https://provider.example/v1".into();
        cloud.provider_name = Some("provider-example".into());
        cloud.cloud_data_tier = Some(sjel_inference::CloudDataTier::Public);
        cloud.billing_mode = Some(sjel_inference::BillingMode::FreeOnly);
        cloud.max_requests_per_day = Some(10);
        cloud.max_input_tokens = Some(4096);

        let agent = Agent::new(cloud, vec![Box::new(Echo)], [], Arc::default()).unwrap();
        let mut messages = vec![Message::User {
            content: "private note".into(),
        }];

        let error = agent.run(&mut messages, |_| {}).unwrap_err();

        assert!(
            matches!(error, AgentError::RequestRefused(reason) if reason.contains("explicit public-data mode"))
        );
        assert_eq!(
            messages.len(),
            1,
            "rejected conversation must stay unchanged"
        );
    }

    #[test]
    fn public_cloud_mode_refuses_unclassified_tool_surfaces() {
        let mut cloud = role(1);
        cloud.backend.base_url = "https://provider.example/v1".into();
        let agent = Agent::new(cloud, vec![Box::new(Echo)], [], Arc::default()).unwrap();
        let mut messages = vec![Message::User {
            content: "public question".into(),
        }];

        let error = agent.run_public(&mut messages, |_| {}).unwrap_err();

        assert!(
            matches!(error, AgentError::RequestRefused(reason) if reason.contains("no tools or extensions"))
        );
        assert_eq!(
            messages.len(),
            1,
            "rejected conversation must stay unchanged"
        );
    }

    #[test]
    fn unencrypted_remote_backend_is_refused_before_model_egress() {
        let mut remote = role(1);
        remote.backend.base_url = "http://provider.example/v1".into();
        let agent = Agent::new(remote, vec![], [], Arc::default()).unwrap();
        let mut messages = vec![Message::User {
            content: "question".into(),
        }];

        let error = agent.run(&mut messages, |_| {}).unwrap_err();

        assert!(
            matches!(error, AgentError::RequestRefused(reason) if reason.contains("reviewed HTTPS cloud role"))
        );
        assert_eq!(messages.len(), 1);
    }

    fn delta(d: Value) -> Value {
        json!({ "choices": [{ "index": 0, "delta": d }] })
    }

    #[test]
    fn queued_steering_enters_the_next_model_request_after_tool_results() {
        struct Pause {
            started: std::sync::mpsc::Sender<()>,
            release: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
        }
        impl Tool for Pause {
            fn name(&self) -> &str {
                "pause"
            }
            fn description(&self) -> &str {
                "Waits for the test to release it."
            }
            fn parameters(&self) -> Value {
                json!({ "type": "object", "properties": {} })
            }
            fn run(&self, _args: &Value) -> Result<String, String> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok("tool finished".into())
            }
        }

        let (port, requests) = serve(vec![
            vec![delta(json!({ "tool_calls": [{
                "index": 0,
                "id": "pause-1",
                "type": "function",
                "function": { "name": "pause", "arguments": "{}" }
            }] }))],
            vec![delta(json!({ "content": "Updated answer." }))],
        ]);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let agent = Agent::new(
            role(port),
            vec![Box::new(Pause {
                started: started_tx,
                release: Arc::new(Mutex::new(release_rx)),
            })],
            [],
            Arc::default(),
        )
        .unwrap();
        let input = agent.input_queue();
        let mut agent = agent;
        agent.set_input_queue(input.clone());
        let run = std::thread::spawn(move || {
            let mut messages = vec![Message::User {
                content: "Initial question.".into(),
            }];
            agent.run(&mut messages, |_| {}).unwrap();
            messages
        });

        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("tool call should start");
        input.steer("Focus on the current week.").unwrap();
        release_tx.send(()).unwrap();

        let first = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            first["messages"].as_array().unwrap().last().unwrap()["content"],
            "Initial question."
        );
        assert_eq!(
            second["messages"].as_array().unwrap().last().unwrap()["content"],
            "Focus on the current week."
        );
        assert!(
            matches!(run.join().unwrap().last(), Some(Message::Assistant { content: Some(text), .. }) if text == "Updated answer.")
        );
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
        let agent = Agent::new(role(port), vec![Box::new(Echo)], [], Arc::default()).unwrap();
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

    struct ConcurrentEcho {
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl Tool for ConcurrentEcho {
        fn name(&self) -> &'static str {
            "concurrent_echo"
        }
        fn description(&self) -> &'static str {
            "Echoes a value after a short delay."
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object", "required": ["value"] })
        }
        fn run(&self, args: &Value) -> Result<String, String> {
            let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
            self.peak.fetch_max(active, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(5));
            self.active.fetch_sub(1, Ordering::Relaxed);
            args["value"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "missing value".into())
        }
        fn read_only(&self) -> bool {
            true
        }
    }

    #[test]
    fn read_only_tool_batches_bound_parallelism_and_preserve_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let agent = Agent::new(
            role(1),
            vec![Box::new(ConcurrentEcho {
                active,
                peak: Arc::clone(&peak),
            })],
            [],
            Arc::default(),
        )
        .unwrap();
        let calls: Vec<_> = (0..MAX_PARALLEL_TOOL_CALLS * 3 + 1)
            .map(|i| ToolCall {
                id: format!("call-{i}"),
                kind: function_kind(),
                function: FunctionCall {
                    name: "concurrent_echo".into(),
                    arguments: json!({"value": i.to_string()}).to_string(),
                },
            })
            .collect();
        let worker_limit = std::thread::available_parallelism()
            .map_or(1, |count| count.get())
            .min(MAX_PARALLEL_TOOL_CALLS);

        let results = agent.call_all(&calls);

        assert_eq!(
            results,
            (0..calls.len()).map(|i| i.to_string()).collect::<Vec<_>>()
        );
        let peak = peak.load(Ordering::Relaxed);
        assert!(
            peak <= worker_limit,
            "peak {peak} exceeded limit {worker_limit}"
        );
        if worker_limit > 1 {
            assert!(peak > 1, "read-only calls did not run in parallel");
        }
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
        let agent = Agent::new(role(port), tools, [], stop).unwrap();
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
        let agent = Agent::new(role(1), vec![Box::new(Echo)], [], Arc::default()).unwrap();
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

    fn tool_call(i: u32, id: &str, name: &str) -> Value {
        json!({ "tool_calls": [{ "index": i, "id": id, "function": { "name": name, "arguments": "{}" } }] })
    }

    /// Counts the runs it is asked for, so a denied call can prove it never ran.
    struct Counted(Arc<AtomicUsize>);
    impl Tool for Counted {
        fn name(&self) -> &'static str {
            "counted"
        }
        fn description(&self) -> &'static str {
            "Counts its runs."
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object" })
        }
        fn run(&self, _: &Value) -> Result<String, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok("ran".into())
        }
    }

    /// One extension touching every hook, denying `counted` and counting what fired.
    #[derive(Default)]
    struct Denier {
        runs: Arc<AtomicUsize>,
        systems: Arc<AtomicUsize>,
        requests: Arc<AtomicUsize>,
    }

    impl Extension for Denier {
        fn name(&self) -> &'static str {
            "denier"
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(Counted(Arc::clone(&self.runs)))]
        }
        fn system(&self, prompt: &mut String) {
            self.systems.fetch_add(1, Ordering::Relaxed);
            prompt.push_str("\ndeny");
        }
        fn before_request(&self, _messages: &mut Vec<Message>) {
            self.requests.fetch_add(1, Ordering::Relaxed);
        }
        fn tool_call(&self, call: &ToolCall) -> Verdict {
            match call.function.name.as_str() {
                "counted" => Verdict::Deny("blocked by the denier".into()),
                _ => Verdict::Allow,
            }
        }
    }

    #[test]
    fn a_tool_result_hook_rewrites_the_model_copy_and_not_the_trace() {
        /// Replaces one word in every `echo` result.
        struct Redactor;
        impl Extension for Redactor {
            fn name(&self) -> &'static str {
                "redactor"
            }
            fn tool_result(&self, call: &ToolCall, content: &mut String) {
                if call.function.name == "echo" {
                    *content = content.replace("secret", "****");
                }
            }
        }
        let echo = json!({ "tool_calls": [{ "index": 0, "id": "c1", "function": {
            "name": "echo", "arguments": "{\"text\":\"the secret value\"}" } }] });
        let (port, _requests) = serve(vec![
            vec![delta(echo)],
            vec![delta(json!({ "content": "done" }))],
        ]);
        let agent = Agent::new(
            role(port),
            vec![Box::new(Echo)],
            [Box::new(Redactor) as Box<dyn Extension>],
            Arc::default(),
        )
        .unwrap();
        let mut messages = vec![Message::User {
            content: "go".into(),
        }];
        let mut traced: Vec<String> = Vec::new();
        agent
            .run(&mut messages, |event| {
                if let Event::ToolResult { content, .. } = event {
                    traced.push(content.to_owned());
                }
            })
            .unwrap();

        assert_eq!(
            messages[2],
            Message::Tool {
                tool_call_id: "c1".into(),
                content: "the **** value".into()
            },
            "the model's copy was not rewritten"
        );
        assert_eq!(traced, ["the secret value"], "the trace is the operator's");
    }

    #[test]
    fn an_extension_denies_a_call_and_the_tool_never_runs() {
        let (port, requests) = serve(vec![
            vec![
                delta(tool_call(0, "c1", "counted")),
                delta(tool_call(1, "c2", "echo")),
            ],
            vec![delta(json!({ "content": "done" }))],
        ]);
        let denier = Denier::default();
        let (runs, systems, seen) = (
            Arc::clone(&denier.runs),
            Arc::clone(&denier.systems),
            Arc::clone(&denier.requests),
        );
        let agent = Agent::new(
            role(port),
            vec![Box::new(Echo)],
            [Box::new(denier) as Box<dyn Extension>],
            Arc::default(),
        )
        .unwrap();
        let mut messages = vec![
            Message::System {
                content: "be terse".into(),
            },
            Message::User {
                content: "go".into(),
            },
        ];
        agent.run(&mut messages, |_| {}).unwrap();

        assert_eq!(runs.load(Ordering::Relaxed), 0, "the denied tool ran");
        // [0] system, [1] user, [2] the assistant's calls, [3] and [4] their results.
        assert_eq!(
            messages[3],
            Message::Tool {
                tool_call_id: "c1".into(),
                content: "blocked by the denier".into()
            }
        );
        // The allowed call in the same turn still ran.
        assert_eq!(
            messages[4],
            Message::Tool {
                tool_call_id: "c2".into(),
                content: "error: no text".into()
            }
        );
        assert_eq!(
            systems.load(Ordering::Relaxed),
            1,
            "system ran once per run"
        );
        assert_eq!(
            seen.load(Ordering::Relaxed),
            2,
            "before_request ran per request"
        );

        let first = requests.recv().unwrap();
        assert_eq!(first["messages"][0]["content"], "be terse\ndeny");
        assert_eq!(first["tools"][1]["function"]["name"], "counted");
    }

    #[test]
    fn a_bare_run_sends_the_core_request_body() {
        let agent = Agent::new(role(1), vec![Box::new(Echo)], [], Arc::default()).unwrap();
        let body = agent.request_body(&[Message::User {
            content: "ping".into(),
        }]);
        assert_eq!(
            body,
            json!({
                "model": "m",
                "messages": [{ "role": "user", "content": "ping" }],
                "stream": true,
                "tools": [{ "type": "function", "function": {
                    "name": "echo",
                    "description": "Returns its text argument.",
                    "parameters": { "type": "object", "properties": { "text": { "type": "string" } } },
                }}],
            }),
            "with no extensions named, the request body is F0's, key for key"
        );
    }

    /// A tool under a name it is given, for the collision cases.
    struct Named(&'static str);
    impl Tool for Named {
        fn name(&self) -> &'static str {
            self.0
        }
        fn description(&self) -> &'static str {
            ""
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object" })
        }
        fn run(&self, _: &Value) -> Result<String, String> {
            Ok(String::new())
        }
    }

    /// An extension carrying one tool under a name it is given.
    struct Carries(&'static str, &'static str);
    impl Extension for Carries {
        fn name(&self) -> &'static str {
            self.0
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(Named(self.1))]
        }
    }

    #[test]
    fn two_tools_with_one_name_stop_the_startup() {
        let err = Agent::new(
            role(1),
            Vec::new(),
            [
                Box::new(Carries("first", "dup")) as Box<dyn Extension>,
                Box::new(Carries("second", "dup")) as Box<dyn Extension>,
            ],
            Arc::default(),
        )
        .err()
        .expect("two extensions claiming one tool name started anyway");
        let said = err.to_string();
        assert!(said.contains("`dup`"), "{said}");
        assert!(said.contains("first") && said.contains("second"), "{said}");

        // Nor may an extension shadow a core tool: `echo` would stop being the tool the
        // operator can see.
        let err = Agent::new(
            role(1),
            vec![Box::new(Echo)],
            [Box::new(Carries("guard", "echo")) as Box<dyn Extension>],
            Arc::default(),
        )
        .err()
        .expect("an extension shadowed a core tool");
        let said = err.to_string();
        assert!(said.contains("`echo`"), "{said}");
        assert!(
            said.contains("the core tools") && said.contains("guard"),
            "{said}"
        );
    }
}

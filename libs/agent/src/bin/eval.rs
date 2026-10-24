//! `sjel-agent-eval --role <name>`: the agent loop with no tools of its own, for
//! tools/agent-eval.ts. `sjel-agent-eval --embed <role>` embeds texts with that role instead, so
//! the evaluator's retrieval tools reach the model without handling its credentials.
//!
//! The evaluator owns every tool, so a comparison with Pi gives both harnesses the same broker
//! (research/benchmarks/agent-harness/README.md, "How a future runner prepares a case").
//! `sjel-agent` cannot do this: it always adds `tools::coding`, which includes a shell.
//!
//! Protocol, one JSON object per line:
//! - stdin, first line: `{"system", "prompt", "tools": [{"name", "description", "parameters"}],
//!   "max_turns"}`
//! - stdout: `{"type":"tool_call","id","name","args"}`; the next stdin line answers it with
//!   `{"ok": bool, "content": string}`
//! - stdout, last line: `{"type":"final","model","endpoint","answer","turns","usage"}` or
//!   `{"type":"error","message"}`. `usage` holds one `{"input","output"}` token count per
//!   model request that reported one.
//!
//! `--embed`: stdin `{"texts": [..], "kind": "query" | "document"}`, stdout
//! `{"type":"embeddings","model","vectors"}` in input order, or the same error line.
//!
//! No overlay extensions, no session file and no AGENTS.md. The model comes from the role in
//! the overlay's `config/inference.json`, or from `SJEL_INFERENCE_CONFIG`.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{json, Value};
use sjel_agent::{Agent, Event, Message, Tool};

#[derive(Deserialize)]
struct Start {
    system: String,
    prompt: String,
    tools: Vec<ToolSpec>,
    max_turns: usize,
}

#[derive(Deserialize)]
struct ToolSpec {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    content: String,
}

/// The evaluator's end of the pipe. One lock, so a call and its reply are never interleaved.
struct Pipe {
    lines: std::io::Lines<std::io::BufReader<std::io::Stdin>>,
}

impl Pipe {
    fn send(value: &Value) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{value}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("stdout closed: {e}"))
    }

    fn receive<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, String> {
        let line = self
            .lines
            .next()
            .ok_or("the evaluator closed stdin")?
            .map_err(|e| format!("stdin: {e}"))?;
        serde_json::from_str(&line).map_err(|e| format!("not a protocol line: {e}"))
    }
}

struct Proxy {
    spec: ToolSpec,
    pipe: Arc<Mutex<Pipe>>,
    calls: Arc<Mutex<u64>>,
}

impl Tool for Proxy {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn parameters(&self) -> Value {
        self.spec.parameters.clone()
    }
    /// The broker parses the arguments, so a malformed call is recorded as the model made it.
    fn run(&self, args: &Value) -> Result<String, String> {
        let mut pipe = self.pipe.lock().map_err(|_| "the tool pipe is poisoned")?;
        let id = {
            let mut calls = self
                .calls
                .lock()
                .map_err(|_| "the call counter is poisoned")?;
            *calls += 1;
            *calls
        };
        Pipe::send(&json!({"type": "tool_call", "id": id, "name": self.spec.name, "args": args}))?;
        let reply: Reply = pipe.receive()?;
        if reply.ok {
            Ok(reply.content)
        } else {
            Err(reply.content)
        }
    }
    // Default `read_only() == false`: calls run one at a time, in the order the transcript
    // records them.
}

fn run(role: &str) -> Result<Value, String> {
    let mut pipe = Pipe {
        lines: std::io::BufReader::new(std::io::stdin()).lines(),
    };
    let start: Start = pipe.receive()?;
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(role)
        .ok_or_else(|| format!("the inference role `{role}` is not configured"))?;
    let endpoint = role.backend.base_url.clone();
    let pipe = Arc::new(Mutex::new(pipe));
    let calls = Arc::new(Mutex::new(0));
    let tools: Vec<Box<dyn Tool>> = start
        .tools
        .into_iter()
        .map(|spec| {
            Box::new(Proxy {
                spec,
                pipe: Arc::clone(&pipe),
                calls: Arc::clone(&calls),
            }) as Box<dyn Tool>
        })
        .collect();
    let mut agent = Agent::new(role, tools, [], Arc::default()).map_err(|e| e.to_string())?;
    agent.max_turns = start.max_turns;
    let mut messages = vec![
        Message::System {
            content: start.system,
        },
        Message::User {
            content: start.prompt,
        },
    ];
    let mut usage = Vec::new();
    agent
        .run(&mut messages, |event: Event<'_>| {
            if let Event::Usage {
                input_tokens,
                output_tokens,
            } = event
            {
                usage.push(json!({"input": input_tokens, "output": output_tokens}));
            }
        })
        .map_err(|e| e.to_string())?;
    let answer = match messages.last() {
        Some(Message::Assistant { content, .. }) => content.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let turns = messages
        .iter()
        .filter(|m| matches!(m, Message::Assistant { .. }))
        .count();
    Ok(json!({
        "type": "final", "model": agent.model(), "endpoint": endpoint, "answer": answer, "turns": turns,
        "usage": usage
    }))
}

#[derive(Deserialize)]
struct EmbedRequest {
    texts: Vec<String>,
    kind: String,
}

/// Requests this many texts at a time; a 400 KB corpus in one request is too large for a local server.
const EMBED_BATCH: usize = 64;

fn embed(role: &str) -> Result<Value, String> {
    let mut pipe = Pipe {
        lines: std::io::BufReader::new(std::io::stdin()).lines(),
    };
    let request: EmbedRequest = pipe.receive()?;
    let kind = match request.kind.as_str() {
        "query" => sjel_inference::TextRole::Query,
        "document" => sjel_inference::TextRole::Document,
        other => return Err(format!("kind must be query or document, not `{other}`")),
    };
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(role)
        .ok_or_else(|| format!("the inference role `{role}` is not configured"))?;
    let mut vectors = Vec::with_capacity(request.texts.len());
    for batch in request.texts.chunks(EMBED_BATCH) {
        vectors.extend(role.embed(batch, kind)?);
    }
    Ok(json!({"type": "embeddings", "model": role.model, "vectors": vectors}))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["--role", role] => run(role),
        ["--embed", role] => embed(role),
        _ => {
            eprintln!("usage: sjel-agent-eval --role <name> | --embed <name>");
            return ExitCode::from(2);
        }
    };
    let (line, code) = match result {
        Ok(final_line) => (final_line, ExitCode::SUCCESS),
        Err(message) => (
            json!({"type": "error", "message": message}),
            ExitCode::FAILURE,
        ),
    };
    match Pipe::send(&line) {
        Ok(()) => code,
        Err(_) => ExitCode::FAILURE,
    }
}

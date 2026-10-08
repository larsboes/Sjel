//! Folds a streamed chat completion (server-sent events) into one assistant [`Message`].
//!
//! Text arrives in fragments and is passed on as it arrives. A tool call arrives in fragments
//! too: the first chunk for an `index` carries the id and name, later ones append to
//! `arguments`. The call is only usable once the stream ends, so calls are returned, not
//! streamed.
//!
//! Chunk strings are borrowed from the line buffer (`Cow`), so a fragment with no JSON escapes
//! costs no allocation before it reaches the front end.

use std::borrow::Cow;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;

use crate::{AgentError, Event, FunctionCall, Message, ToolCall};

#[derive(Deserialize)]
struct Chunk<'a> {
    #[serde(default, borrow)]
    choices: Vec<Choice<'a>>,
    /// Some providers report a failure inside a 200 stream.
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Choice<'a> {
    #[serde(borrow)]
    delta: Delta<'a>,
}

#[derive(Deserialize, Default)]
struct Delta<'a> {
    #[serde(default, borrow)]
    content: Option<Cow<'a, str>>,
    /// The model's thinking, apart from the answer. Two names for one field: DeepSeek, vLLM
    /// and DashScope send `reasoning_content`, Ollama's `/v1` shim sends `reasoning`. A stream
    /// that carries only one of them is not a stream with no thinking.
    #[serde(default, borrow)]
    reasoning_content: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    reasoning: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    tool_calls: Option<Vec<CallDelta<'a>>>,
}

#[derive(Deserialize)]
struct CallDelta<'a> {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default, borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    function: Option<FunctionDelta<'a>>,
}

#[derive(Deserialize)]
struct FunctionDelta<'a> {
    #[serde(default, borrow)]
    name: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    arguments: Option<Cow<'a, str>>,
}

/// Stops reading when `stop` turns true and returns what arrived so far. A caller that sees
/// `stop` set must treat the tool calls as incomplete.
pub fn fold(
    mut reader: impl BufRead,
    stop: &AtomicBool,
    on: &mut impl FnMut(Event<'_>),
) -> Result<Message, AgentError> {
    let mut content = String::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut line = String::new();
    // ponytail: checked between lines, so a stop waits for the next chunk. A model that is
    // still reading a long prompt sends none; stopping that needs a reader that can be closed.
    while !stop.load(Ordering::Relaxed) {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        // `data:` lines only. Comments (`:`), `event:` and blank separators carry nothing here.
        let Some(data) = line.trim_end().strip_prefix("data:").map(str::trim_start) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: Chunk<'_> = serde_json::from_str(data)
            .map_err(|e| AgentError::Malformed(format!("{e}: {}", crate::tools::cap(data, 300))))?;
        if let Some(error) = chunk.error {
            return Err(AgentError::Malformed(crate::tools::cap(
                &error.to_string(),
                2_000,
            )));
        }
        let Some(delta) = chunk.choices.into_iter().next().map(|c| c.delta) else {
            continue;
        };
        if let Some(text) = delta
            .reasoning_content
            .as_deref()
            .or(delta.reasoning.as_deref())
            .filter(|t| !t.is_empty())
        {
            on(Event::Reasoning(text));
        }
        if let Some(text) = delta.content.as_deref().filter(|t| !t.is_empty()) {
            on(Event::Text(text));
            content.push_str(text);
        }
        for part in delta.tool_calls.into_iter().flatten() {
            // A provider that omits `index` sends each call whole, with its own id.
            let index = part.index.unwrap_or(if part.id.is_some() {
                calls.len()
            } else {
                calls.len().saturating_sub(1)
            });
            if index >= calls.len() {
                calls.resize_with(index + 1, || ToolCall {
                    id: String::new(),
                    kind: crate::function_kind(),
                    function: FunctionCall::default(),
                });
            }
            let call = &mut calls[index];
            if let Some(id) = part.id {
                call.id = id.into_owned();
            }
            if let Some(function) = part.function {
                if let Some(name) = function.name {
                    call.function.name.push_str(&name);
                }
                if let Some(arguments) = function.arguments {
                    call.function.arguments.push_str(&arguments);
                }
            }
        }
    }
    // A local server that omits ids still needs a stable one to pair results with calls.
    for (i, call) in calls.iter_mut().enumerate() {
        if call.id.is_empty() {
            call.id = format!("call_{i}");
        }
    }
    Ok(Message::Assistant {
        content: (!content.is_empty()).then_some(content),
        tool_calls: (!calls.is_empty()).then_some(calls),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sse: &str) -> (Result<Message, AgentError>, String, String) {
        let (mut text, mut thinking) = (String::new(), String::new());
        let result = fold(sse.as_bytes(), &AtomicBool::new(false), &mut |e| match e {
            Event::Text(t) => text.push_str(t),
            Event::Reasoning(t) => thinking.push_str(t),
            _ => {}
        });
        (result, text, thinking)
    }

    #[test]
    fn fragments_join_into_calls_and_text() {
        let sse = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi \\\"x\\\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":1}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (message, text, thinking) = run(sse);
        assert_eq!(text, "Hi \"x\"");
        assert_eq!(thinking, "hm");
        let Message::Assistant {
            content,
            tool_calls: Some(calls),
        } = message.unwrap()
        else {
            panic!("expected a tool call")
        };
        assert_eq!(content.as_deref(), Some("Hi \"x\""));
        assert_eq!(calls[0].id, "a");
        assert_eq!(calls[0].function.name, "read");
        assert_eq!(calls[0].function.arguments, "{\"path\":1}");
    }

    #[test]
    fn whole_calls_without_index_or_id_stay_separate() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"x\",\"function\":{\"name\":\"a\",\"arguments\":\"{}\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"name\":\"b\",\"arguments\":\"{}\"}}]}}]}\n",
        );
        let Message::Assistant {
            tool_calls: Some(calls),
            ..
        } = run(sse).0.unwrap()
        else {
            panic!()
        };
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].id, "call_1");
    }

    #[test]
    fn ollama_names_thinking_reasoning() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\",\"reasoning\":\"hm\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n",
            "data: [DONE]\n",
        );
        let (message, text, thinking) = run(sse);
        assert_eq!(thinking, "hm");
        assert_eq!(text, "pong");
        assert_eq!(message.unwrap(), Message::Assistant {
            content: Some("pong".into()),
            tool_calls: None
        });
    }

    #[test]
    fn an_error_inside_the_stream_is_an_error() {
        let (result, ..) = run("data: {\"error\":{\"message\":\"rate limited\"}}\n");
        assert!(result.unwrap_err().to_string().contains("rate limited"));
    }
}

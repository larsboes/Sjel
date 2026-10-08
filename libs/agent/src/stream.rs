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
    use proptest::prelude::*;
    use serde_json::json;

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
        assert_eq!(
            message.unwrap(),
            Message::Assistant {
                content: Some("pong".into()),
                tool_calls: None
            }
        );
    }

    #[test]
    fn an_error_inside_the_stream_is_an_error() {
        let (result, ..) = run("data: {\"error\":{\"message\":\"rate limited\"}}\n");
        assert!(result.unwrap_err().to_string().contains("rate limited"));
    }

    /// The bytes `fold` is given, handed over `size` bytes at a time.
    ///
    /// A stream arrives in whatever pieces the network wrote, and reading it a line at a time
    /// through `BufRead` is what has to survive a cut in the middle of one.
    struct InChunks {
        data: Vec<u8>,
        at: usize,
        size: usize,
    }

    impl std::io::Read for InChunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.size.min(buf.len()).min(self.data.len() - self.at);
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    /// `run`, for a stream handed over in pieces of `size` bytes.
    fn run_in_chunks(stream: &str, size: usize) -> (Result<Message, AgentError>, String, String) {
        let (mut text, mut thinking) = (String::new(), String::new());
        let reader = std::io::BufReader::new(InChunks {
            data: stream.as_bytes().to_vec(),
            at: 0,
            size,
        });
        let result = fold(reader, &AtomicBool::new(false), &mut |e| match e {
            Event::Text(t) => text.push_str(t),
            Event::Reasoning(t) => thinking.push_str(t),
            _ => {}
        });
        (result, text, thinking)
    }

    /// The body a server would send for these chunks: `data:` lines, blanks between them, and
    /// the terminator.
    fn sse(chunks: &[String]) -> String {
        let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
        body.push_str("data: [DONE]\n\n");
        body
    }

    /// Deltas of the shapes the fold has to understand, as JSON text.
    fn deltas() -> impl Strategy<Value = String> {
        let text = "[a-zA-Z ]{0,12}";
        prop_oneof![
            text.prop_map(|t| json!({ "choices": [{ "delta": { "content": t } }] }).to_string()),
            text.prop_map(
                |t| json!({ "choices": [{ "delta": { "reasoning_content": t } }] }).to_string()
            ),
            "[a-z]{1,8}".prop_map(|name| json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "c1", "function": { "name": name, "arguments": "{}" } }] } }] })
            .to_string()),
        ]
    }

    proptest! {
        /// AGT-22, first half: one valid stream, folded in pieces of any size, gives the same
        /// message and the same streamed text as folding it whole.
        #[test]
        fn the_fold_is_the_same_however_the_stream_is_cut(
            chunks in proptest::collection::vec(deltas(), 0..8),
            size in 1usize..64,
        ) {
            let stream = sse(&chunks);
            let (whole, whole_text, whole_thinking) = run(&stream);
            let (cut, cut_text, cut_thinking) = run_in_chunks(&stream, size);
            let whole = whole.expect("a stream built from valid chunks folds to a message");
            let cut = cut.expect("and keeps folding that way whatever pieces it arrives in");
            prop_assert_eq!(whole, cut);
            prop_assert_eq!(whole_text, cut_text);
            prop_assert_eq!(whole_thinking, cut_thinking);
        }

        /// AGT-22, second half: arbitrary bytes — not a stream, not UTF-8, not JSON — come back
        /// as a message or an error. The claim is that they return at all.
        #[test]
        fn arbitrary_bytes_return_rather_than_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..512),
            size in 1usize..32,
        ) {
            let stream = String::from_utf8_lossy(&bytes).into_owned();
            let (result, ..) = run_in_chunks(&stream, size);
            // Ok or Err both satisfy the claim; a panic would not have returned.
            let _ = result;
        }

        /// And a valid stream cut short mid-flight — the connection dropped — likewise returns.
        /// Truncation is where a half-read line is most likely to be mishandled.
        #[test]
        fn a_truncated_stream_returns(
            chunks in proptest::collection::vec(deltas(), 0..8),
            cut_at in 0usize..512,
        ) {
            let stream = sse(&chunks);
            let cut: String = stream.chars().take(cut_at).collect();
            let (result, ..) = run_in_chunks(&cut, 5);
            let _ = result;
        }
    }
}

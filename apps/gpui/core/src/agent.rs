//! The agent capability: sessions, the events one run emits, and the owner's decisions.
//!
//! A session outlives the request that started it, so this is not a request-and-reply client.
//! `agent_prompt` starts a run and returns at once; the run's events arrive on the stream, which
//! can be attached from anywhere and resumed with `from`. That is the whole reason the phone and
//! the Mac can watch one run: neither holds the conversation, the capability does.
//!
//! Nothing here draws. `apps/gpui/src/finder.rs` and `ui/src/agent/` render what this returns.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use serde::Deserialize;
use serde_json::json;

use crate::shell::{spawn, Shell, ShellError, ShellResult};

/// One conversation, as the capability lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub root: String,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub turns: usize,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub title: Option<String>,
}

impl Session {
    /// The first prompt, on one line, or the root when the session has none yet.
    pub fn label(&self) -> String {
        match self
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
        {
            Some(title) => title.to_owned(),
            None => self.root.clone(),
        }
    }

    /// The last path component of the root: what a list can show without a slash per row.
    pub fn root_name(&self) -> String {
        self.root
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.root)
            .to_owned()
    }
}

/// One step of a run, as the stream carried it.
///
/// `kind` is the capability's `type`, kept as a string so a front end that meets a kind it does
/// not know can say so instead of failing to parse the whole event.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Event {
    /// Position in the run's log. A reconnect asks for everything after the last one seen.
    #[serde(default)]
    pub seq: u64,
    /// The oldest retained event when a stream reports that its cursor fell behind the log.
    #[serde(default)]
    pub first_seq: u64,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    /// A tool call's id, so its result can be matched to it.
    #[serde(default, rename = "id")]
    pub tool_id: String,
    /// The one-line summary an approval card shows.
    #[serde(default)]
    pub preview: String,
    /// The lines an `edit` or a `write` will change, as they will read afterwards.
    #[serde(default)]
    pub diff: Option<String>,
    #[serde(default, rename = "approval_id")]
    pub approval: String,
    #[serde(default)]
    pub rule: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub message: String,
}

impl Event {
    /// Why a run stopped, when it stopped for a reason the operator has to read.
    pub fn failure(&self) -> Option<&str> {
        (self.kind == "error").then(|| self.message.as_str())
    }

    /// True for the events that end a run's stream.
    pub fn terminal(&self) -> bool {
        matches!(self.kind.as_str(), "done" | "stopped" | "error")
    }

    /// A tool call the operator has to answer before it runs.
    pub fn needs_approval(&self) -> Option<&str> {
        (self.kind == "needs_approval" && !self.approval.is_empty()).then(|| self.approval.as_str())
    }
}

/// One stored turn, as `/transcript` returns it: the `Message` enum `libs/agent` writes.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Turn {
    /// `system`, `user`, `assistant` or `tool`.
    #[serde(rename = "role", default)]
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    /// Present for a `tool` turn: which call this answers.
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// The path prefix the shell proxies to the capability.
const BASE: &str = "/agent";
/// Starting a run answers as soon as the turn is claimed; the work happens behind the stream.
const START_TIMEOUT: u64 = 30;
/// A stream lives as long as its run, including the minutes a tool call waits for a human.
const STREAM_TIMEOUT: u64 = 1_800;

impl Shell {
    pub async fn agent_sessions(&self) -> ShellResult<Vec<Session>> {
        #[derive(Deserialize)]
        struct Listed {
            #[serde(default)]
            sessions: Vec<Session>,
        }
        let listed: Listed = self
            .json(
                reqwest::Method::GET,
                &format!("{BASE}/api/agent/sessions"),
                None,
                10,
            )
            .await?;
        Ok(listed.sessions)
    }

    /// Starts a session. `root` is an absolute path; absent means the checkout the capability
    /// runs from, which is what the finder offers by default.
    pub async fn agent_create(
        &self,
        root: Option<&str>,
        tier: Option<&str>,
    ) -> ShellResult<Session> {
        let mut body = json!({});
        if let Some(root) = root {
            body["root"] = json!(root);
        }
        if let Some(tier) = tier {
            body["tier"] = json!(tier);
        }
        self.json(
            reqwest::Method::POST,
            &format!("{BASE}/api/agent/sessions"),
            Some(body),
            10,
        )
        .await
    }

    /// Sends one prompt and returns as soon as the run is claimed. Events arrive on the stream.
    pub async fn agent_prompt(&self, id: &str, prompt: &str) -> ShellResult<()> {
        let _: serde_json::Value = self
            .json(
                reqwest::Method::POST,
                &format!("{BASE}/api/agent/sessions/{id}/prompt"),
                Some(json!({ "prompt": prompt })),
                START_TIMEOUT,
            )
            .await?;
        Ok(())
    }

    /// Attaches to the session's run, from `from` (0 for the whole of the current turn).
    pub async fn agent_stream(&self, id: &str, from: u64) -> ShellResult<UnboundedReceiver<Event>> {
        let response = self
            .send(
                reqwest::Method::GET,
                &format!("{BASE}/api/agent/sessions/{id}/stream?from={from}"),
                None,
                STREAM_TIMEOUT,
            )
            .await?;
        let (to, from_channel) = unbounded();
        spawn(async move {
            let mut response = response;
            let mut buffer = Vec::new();
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) | Err(_) => break,
                };
                buffer.extend_from_slice(&chunk);
                while let Some(frame) = next_frame(&mut buffer) {
                    // A `truncated` frame carries a body this client has no use for: the events
                    // the reader asked for are gone, and it re-reads the transcript instead.
                    if frame.event == "truncated" {
                        let details =
                            serde_json::from_str::<Event>(&frame.data).unwrap_or_default();
                        let _ = to.unbounded_send(Event {
                            kind: "truncated".into(),
                            first_seq: details.first_seq,
                            ..Event::default()
                        });
                        return;
                    }
                    let Ok(event) = serde_json::from_str::<Event>(&frame.data) else {
                        continue;
                    };
                    let terminal = event.terminal();
                    if to.unbounded_send(event).is_err() || terminal {
                        return;
                    }
                }
            }
        });
        Ok(from_channel)
    }

    /// The owner's answer to one tool call. `for_session` allows this exact call for the rest of
    /// the session, which is what makes a run of twenty commands usable.
    pub async fn agent_approve(
        &self,
        id: &str,
        approval: &str,
        allow: bool,
        for_session: bool,
    ) -> ShellResult<()> {
        let scope = if for_session { "session" } else { "once" };
        let _: serde_json::Value = self
            .json(
                reqwest::Method::POST,
                &format!("{BASE}/api/agent/sessions/{id}/approve"),
                Some(json!({
                    "approval_id": approval,
                    "allow": allow,
                    "scope": scope,
                })),
                10,
            )
            .await?;
        Ok(())
    }

    /// Queues a message for the run's next model request.
    pub async fn agent_steer(&self, id: &str, message: &str) -> ShellResult<()> {
        let _: serde_json::Value = self
            .json(
                reqwest::Method::POST,
                &format!("{BASE}/api/agent/sessions/{id}/steer"),
                Some(json!({ "message": message })),
                10,
            )
            .await?;
        Ok(())
    }

    /// Stops the turn and keeps the conversation.
    pub async fn agent_stop(&self, id: &str) -> ShellResult<()> {
        let _: serde_json::Value = self
            .json(
                reqwest::Method::POST,
                &format!("{BASE}/api/agent/sessions/{id}/stop"),
                None,
                10,
            )
            .await?;
        Ok(())
    }

    /// The turns as stored, for a session whose run happened on another screen or before a
    /// restart.
    pub async fn agent_transcript(&self, id: &str) -> ShellResult<Vec<Turn>> {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            messages: Vec<Turn>,
        }
        let stored: Stored = self
            .json(
                reqwest::Method::GET,
                &format!("{BASE}/api/agent/sessions/{id}/transcript"),
                None,
                10,
            )
            .await?;
        Ok(stored.messages)
    }
}

/// One server-sent event: its `event:` name (default `message`) and its joined `data:` lines.
pub(crate) struct Frame {
    pub(crate) event: String,
    pub(crate) data: String,
}

/// Reads one event out of `buffer`, when a blank line has arrived.
///
/// The double newline is the delimiter: a chunk boundary can fall anywhere, so nothing is returned
/// until the event that ends it is complete. Comments (`: keep-alive`) carry no data and are
/// dropped — that is what keeps a stream that waits for an approval from looking dead.
pub(crate) fn next_frame(buffer: &mut Vec<u8>) -> Option<Frame> {
    let mut cursor = 0;
    let mut event = String::from("message");
    let mut data = String::new();
    let mut lines = 0;
    while cursor < buffer.len() {
        let end = buffer[cursor..].iter().position(|byte| *byte == b'\n')? + cursor;
        let line = String::from_utf8_lossy(&buffer[cursor..end]).into_owned();
        let next = end + 1;
        if line.trim_end_matches('\r').is_empty() {
            buffer.drain(..next);
            // A blank line with nothing before it is not an event; keep looking.
            if lines == 0 {
                cursor = 0;
                continue;
            }
            return Some(Frame {
                event,
                data: data.trim_end_matches('\n').to_owned(),
            });
        }
        let line = line.trim_end_matches('\r');
        if line.starts_with(':') {
            // A comment carries no data. It is not an event, so it must not count as one.
            cursor = next;
            continue;
        }
        lines += 1;
        if let Some(rest) = line.strip_prefix("event:") {
            event = rest.trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            data.push('\n');
        }
        cursor = next;
    }
    None
}

/// One line for a tool row: the command for `bash`, the path for a file tool, and the raw
/// arguments for anything else.
pub fn tool_summary(name: &str, arguments: &str) -> String {
    let trimmed = arguments.trim();
    let parsed: Option<serde_json::Value> = serde_json::from_str(trimmed).ok();
    let field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let summary = match name {
        "bash" => field("command"),
        "read" | "write" | "edit" | "ls" => field("path"),
        "grep" | "find" => field("pattern").or_else(|| field("glob")),
        "sql" => field("q").or_else(|| field("query")),
        _ => None,
    };
    let text = summary.unwrap_or_else(|| trimmed.to_owned());
    let one_line = text.lines().next().unwrap_or_default().trim().to_owned();
    if one_line.chars().count() > 90 {
        format!("{}…", one_line.chars().take(90).collect::<String>())
    } else {
        one_line
    }
}

/// Why an agent call failed, in the words a panel can show.
pub fn describe(error: &ShellError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_arrives_whole_or_not_at_all() {
        let mut buffer = b"data: {\"type\":\"text\"}".to_vec();
        assert!(
            next_frame(&mut buffer).is_none(),
            "half an event is not an event"
        );
        buffer.extend_from_slice(b"\n\n");
        let frame = next_frame(&mut buffer).unwrap();
        assert_eq!(frame.event, "message");
        assert_eq!(frame.data, "{\"type\":\"text\"}");
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_named_event_and_a_comment() {
        let mut buffer = b": keep-alive\n\nevent: truncated\ndata: {\"from\":3}\n\n".to_vec();
        // The comment is not an event: `lines` is 0 for it, so the parser skips past it.
        let frame = next_frame(&mut buffer).unwrap();
        assert_eq!(frame.event, "truncated");
        assert_eq!(frame.data, "{\"from\":3}");
    }

    #[test]
    fn two_events_in_one_chunk_are_read_in_order() {
        let mut buffer = b"data: one\n\ndata: two\n\n".to_vec();
        assert_eq!(next_frame(&mut buffer).unwrap().data, "one");
        assert_eq!(next_frame(&mut buffer).unwrap().data, "two");
        assert!(next_frame(&mut buffer).is_none());
    }

    #[test]
    fn a_carriage_return_does_not_become_a_field_name() {
        let mut buffer = b"event: truncated\r\ndata: x\r\n\r\n".to_vec();
        let frame = next_frame(&mut buffer).unwrap();
        assert_eq!(frame.event, "truncated");
        assert_eq!(frame.data, "x");
    }

    #[test]
    fn an_event_carries_the_kind_its_reader_branches_on() {
        let event: Event = serde_json::from_str(
            r#"{"type":"needs_approval","seq":7,"approval_id":"abc","name":"bash","preview":"cargo test","diff":null}"#,
        )
        .unwrap();
        assert_eq!(event.seq, 7);
        assert_eq!(event.needs_approval(), Some("abc"));
        assert_eq!(event.name, "bash");
        assert!(event.diff.is_none());
        assert!(!event.terminal());

        let done: Event = serde_json::from_str(r#"{"type":"done","seq":9}"#).unwrap();
        assert!(done.terminal());
        assert!(done.failure().is_none());

        let error: Event =
            serde_json::from_str(r#"{"type":"error","seq":9,"message":"no role"}"#).unwrap();
        assert_eq!(error.failure(), Some("no role"));
    }

    #[test]
    fn an_unknown_kind_still_parses_so_the_rest_of_the_run_is_readable() {
        let event: Event = serde_json::from_str(r#"{"type":"something-new","seq":1}"#).unwrap();
        assert_eq!(event.kind, "something-new");
        assert!(!event.terminal());
    }

    #[test]
    fn a_tool_row_says_what_the_call_is_about() {
        assert_eq!(
            tool_summary("bash", r#"{"command":"cargo test --lib"}"#),
            "cargo test --lib"
        );
        assert_eq!(
            tool_summary(
                "edit",
                r#"{"path":"src/finder.rs","old_text":"a","new_text":"b"}"#
            ),
            "src/finder.rs"
        );
        assert_eq!(tool_summary("sql", r#"{"q":"select 1"}"#), "select 1");
        // Not JSON, or a tool this app does not know: the arguments are still shown.
        assert_eq!(tool_summary("mystery", "{\"a\":"), "{\"a\":");
        // Multi-line arguments are one row, not several.
        assert_eq!(tool_summary("bash", r#"{"command":"one\ntwo"}"#), "one");
    }

    #[test]
    fn a_session_is_labelled_by_its_first_prompt_or_its_root() {
        let session = Session {
            id: "a".into(),
            root: "/Users/x/Developer/Sjel".into(),
            title: Some("fix the inbox test".into()),
            ..Session::default()
        };
        assert_eq!(session.label(), "fix the inbox test");
        assert_eq!(session.root_name(), "Sjel");
        let untitled = Session {
            title: None,
            ..session
        };
        assert_eq!(untitled.label(), "/Users/x/Developer/Sjel");
    }
}

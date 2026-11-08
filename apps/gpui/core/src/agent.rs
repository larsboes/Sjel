//! The agent capability: sessions, the events one run emits, and the owner's decisions.
//!
//! A session outlives the request that started it, so this is not a request-and-reply client.
//! `agent_prompt` starts a run and returns at once; the run's events arrive on the stream, which
//! can be attached from anywhere and resumed with `from`. That is the whole reason the phone and
//! the Mac can watch one run: neither holds the conversation, the capability does.
//!
//! Nothing here draws. `ui/src/agent_seat.rs` renders what this returns, on the Mac and the phone.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use serde::Deserialize;
use serde_json::json;

use crate::device::DeviceClient;
use crate::shell::{spawn, Shell, ShellError, ShellResult};
pub use crate::terminal::Host;

pub use agent_api::{
    ApproveRequest, CreateSession, Event, PromptRequest, Session, SessionMeta, SteerRequest,
    StreamQuery, Tier, Turn, DEFAULT_AGENT_PORT, DEFAULT_TIER, ROUTE_AGENT_SESSION,
    ROUTE_AGENT_SESSIONS, ROUTE_AGENT_SESSION_APPROVE, ROUTE_AGENT_SESSION_PROMPT,
    ROUTE_AGENT_SESSION_STEER, ROUTE_AGENT_SESSION_STOP, ROUTE_AGENT_SESSION_STREAM,
    ROUTE_AGENT_SESSION_TRANSCRIPT,
};

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
        Ok(events(response))
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

/// Reads a run's event stream into a channel that ends after the terminal event.
fn events(response: reqwest::Response) -> UnboundedReceiver<Event> {
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
                    let details = serde_json::from_str::<Event>(&frame.data).unwrap_or_default();
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
    from_channel
}

/// The agent routes from either side of the pairing: the Mac's own shell, or the phone's signed
/// client. The panel above them is written once (`ui/src/agent_seat.rs`).
impl Host {
    pub async fn agent_sessions(&self) -> Result<Vec<Session>, String> {
        #[derive(Deserialize)]
        struct Listed {
            #[serde(default)]
            sessions: Vec<Session>,
        }
        match self {
            Self::Shell(shell) => shell.agent_sessions().await.map_err(|e| e.to_string()),
            Self::Device(client) => client
                .get::<Listed>(&format!("{BASE}/api/agent/sessions"))
                .await
                .map(|listed| listed.sessions),
        }
    }

    pub async fn agent_create(&self, tier: &str) -> Result<Session, String> {
        match self {
            Self::Shell(shell) => shell
                .agent_create(None, Some(tier))
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let body = json!({ "tier": tier }).to_string();
                client
                    .post(&format!("{BASE}/api/agent/sessions"), body.as_bytes())
                    .await
            }
        }
    }

    pub async fn agent_prompt(&self, id: &str, prompt: &str) -> Result<(), String> {
        match self {
            Self::Shell(shell) => shell
                .agent_prompt(id, prompt)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let body = json!({ "prompt": prompt }).to_string();
                post_device(
                    client,
                    &format!("{BASE}/api/agent/sessions/{id}/prompt"),
                    &body,
                )
                .await
            }
        }
    }

    pub async fn agent_steer(&self, id: &str, message: &str) -> Result<(), String> {
        match self {
            Self::Shell(shell) => shell
                .agent_steer(id, message)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let body = json!({ "message": message }).to_string();
                post_device(
                    client,
                    &format!("{BASE}/api/agent/sessions/{id}/steer"),
                    &body,
                )
                .await
            }
        }
    }

    pub async fn agent_approve(
        &self,
        id: &str,
        approval: &str,
        allow: bool,
        for_session: bool,
    ) -> Result<(), String> {
        match self {
            Self::Shell(shell) => shell
                .agent_approve(id, approval, allow, for_session)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let scope = if for_session { "session" } else { "once" };
                let body =
                    json!({ "approval_id": approval, "allow": allow, "scope": scope }).to_string();
                post_device(
                    client,
                    &format!("{BASE}/api/agent/sessions/{id}/approve"),
                    &body,
                )
                .await
            }
        }
    }

    pub async fn agent_stop(&self, id: &str) -> Result<(), String> {
        match self {
            Self::Shell(shell) => shell.agent_stop(id).await.map_err(|e| e.to_string()),
            Self::Device(client) => {
                post_device(
                    client,
                    &format!("{BASE}/api/agent/sessions/{id}/stop"),
                    "{}",
                )
                .await
            }
        }
    }

    pub async fn agent_transcript(&self, id: &str) -> Result<Vec<Turn>, String> {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            messages: Vec<Turn>,
        }
        match self {
            Self::Shell(shell) => shell.agent_transcript(id).await.map_err(|e| e.to_string()),
            Self::Device(client) => client
                .get::<Stored>(&format!("{BASE}/api/agent/sessions/{id}/transcript"))
                .await
                .map(|stored| stored.messages),
        }
    }

    pub async fn agent_stream(
        &self,
        id: &str,
        from: u64,
    ) -> Result<UnboundedReceiver<Event>, String> {
        match self {
            Self::Shell(shell) => shell
                .agent_stream(id, from)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => client
                .streaming_get(&format!(
                    "{BASE}/api/agent/sessions/{id}/stream?from={from}"
                ))
                .await
                .map(events),
        }
    }
}

async fn post_device(client: &DeviceClient, path: &str, body: &str) -> Result<(), String> {
    client
        .post::<serde_json::Value>(path, body.as_bytes())
        .await
        .map(|_| ())
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

/// One line of the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// What was asked.
    Prompt(String),
    /// What came back, appended to as it streams.
    Answer(String),
    /// A tool call, its arguments, and its result once there is one.
    Tool {
        name: String,
        call_id: String,
        arguments: String,
        result: Option<String>,
    },
    /// An approval that was answered, in one line.
    Decided {
        name: String,
        allowed: bool,
        for_session: bool,
    },
    /// A reason the run stopped, or a fact worth a line: usage, a stop, a truncated stream.
    Notice(String),
}

/// The call the run is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub approval: String,
    pub name: String,
    /// The one-line summary: a command, or `path +3 −1`.
    pub preview: String,
    /// The lines that will change, when the call changes a file.
    pub diff: Option<String>,
    pub rule: Option<String>,
    pub class: Option<String>,
}

impl Pending {
    pub fn is_block(&self) -> bool {
        self.class.as_deref() == Some("block")
            || self.rule.as_ref().is_some_and(|r| r.starts_with('B'))
    }
}

pub struct AgentRun {
    /// The session this panel is attached to.
    pub session: Option<Session>,
    pub rows: Vec<Row>,
    pub pending: Option<Pending>,
    /// A reason nothing is running, when something went wrong before it started.
    pub error: Option<String>,
    pub running: bool,
    /// The last event applied, so a reconnect asks for what came after it.
    pub last_seq: u64,
    /// True once the stream reported that events this panel never saw are gone.
    pub truncated: bool,
    /// Sessions available from the capability, newest first.
    pub sessions: Vec<Session>,
    pub sessions_loading: bool,
    /// Tier selected for the next session if none is open.
    pub draft_tier: &'static str,
}

impl Default for AgentRun {
    fn default() -> Self {
        Self {
            session: None,
            rows: Vec::new(),
            pending: None,
            error: None,
            running: false,
            last_seq: 0,
            truncated: false,
            sessions: Vec::new(),
            sessions_loading: false,
            draft_tier: "project",
        }
    }
}

impl AgentRun {
    /// Which session, and whether it is still the one on screen.
    pub fn id(&self) -> Option<&str> {
        self.session.as_ref().map(|session| session.id.as_str())
    }

    /// Everything that has to be said before a new prompt can start from here.
    pub fn busy(&self) -> bool {
        self.running || self.pending.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Restores a stored conversation for the session shelf.
    pub fn restore(&mut self, session: Session, turns: Vec<Turn>) {
        let sessions = std::mem::take(&mut self.sessions);
        let sessions_loading = self.sessions_loading;
        self.clear();
        self.sessions = sessions;
        self.sessions_loading = sessions_loading;
        self.running = session.active;
        self.session = Some(session);
        for turn in turns {
            match turn.role.as_str() {
                "user" => self
                    .rows
                    .push(Row::Prompt(turn.content.unwrap_or_default())),
                "assistant" => {
                    if let Some(content) = turn.content.filter(|content| !content.is_empty()) {
                        self.rows.push(Row::Answer(content));
                    }
                }
                "tool" => self.rows.push(Row::Tool {
                    name: "tool".into(),
                    call_id: turn.tool_call_id.unwrap_or_default(),
                    arguments: String::new(),
                    result: turn.content,
                }),
                _ => {}
            }
        }
    }

    /// One event from the stream.
    pub fn apply(&mut self, event: Event) {
        self.last_seq = self.last_seq.max(event.seq);
        match event.kind.as_str() {
            "turn" => {
                if !event.content.trim().is_empty() {
                    self.rows.push(Row::Prompt(event.content));
                }
            }
            "text" => match self.rows.last_mut() {
                // Text arrives in fragments: one answer is one row, not one row per token.
                Some(Row::Answer(answer)) => answer.push_str(&event.content),
                _ => self.rows.push(Row::Answer(event.content)),
            },
            // A model's thinking is not the answer and not a step. It is dropped rather than
            // shown as either, because a panel that mixes them is a panel you stop reading.
            "reasoning" => {}
            "tool_call" => {
                self.rows.push(Row::Tool {
                    name: event.name,
                    call_id: event.tool_id,
                    arguments: event.arguments,
                    result: None,
                });
            }
            "tool_result" => {
                // Match on the call id, so a batch of parallel calls keeps its own answers.
                let slot = self.rows.iter_mut().rev().find(|row| match row {
                    Row::Tool {
                        call_id, result, ..
                    } => result.is_none() && (call_id.is_empty() || *call_id == event.tool_id),
                    _ => false,
                });
                if let Some(Row::Tool { result, .. }) = slot {
                    *result = Some(event.content);
                } else {
                    self.rows.push(Row::Tool {
                        name: event.name,
                        call_id: event.tool_id,
                        arguments: String::new(),
                        result: Some(event.content),
                    });
                }
            }
            "needs_approval" => {
                if !event.approval.is_empty() {
                    self.pending = Some(Pending {
                        approval: event.approval,
                        name: event.name,
                        preview: event.preview,
                        diff: event.diff,
                        rule: event.rule,
                        class: event.class,
                    });
                }
            }
            "blocked" => {
                let rule_suffix = event
                    .rule
                    .as_deref()
                    .map(|r| format!(" · rule {r}"))
                    .unwrap_or_default();
                let reason = if let Some(reason) = event.reason.filter(|r| !r.is_empty()) {
                    reason
                } else if !event.name.is_empty() {
                    format!("{} blocked", event.name)
                } else {
                    "blocked by policy".to_owned()
                };
                self.rows
                    .push(Row::Notice(format!("policy block{rule_suffix}: {reason}")));
            }
            "approved" => {
                self.pending = None;
                self.rows.push(Row::Decided {
                    name: event.name,
                    allowed: true,
                    for_session: false,
                });
            }
            "denied" => {
                self.pending = None;
                self.rows.push(Row::Decided {
                    name: event.name,
                    allowed: false,
                    for_session: false,
                });
            }
            "usage" => {
                if event.input_tokens > 0 || event.output_tokens > 0 {
                    self.rows.push(Row::Notice(format!(
                        "{} in · {} out",
                        event.input_tokens, event.output_tokens
                    )));
                }
            }
            "stopped" => {
                self.running = false;
                self.pending = None;
                self.rows.push(Row::Notice("stopped".into()));
            }
            "done" => {
                self.running = false;
                self.pending = None;
            }
            "error" => {
                self.running = false;
                self.pending = None;
                self.error = Some(event.message);
            }
            "truncated" => {
                // The stream ends after this frame, so nothing here can show the run as live.
                self.running = false;
                self.truncated = true;
                self.last_seq = event.first_seq.saturating_sub(1);
                // The card may be for a call that was already answered while this panel was
                // away. Clearing it beats offering an Allow for something that already ran.
                self.pending = None;
                self.rows.push(Row::Notice(
                    "This panel missed part of the run. Open the transcript to read it all.".into(),
                ));
            }
            other => {
                self.rows
                    .push(Row::Notice(format!("unhandled event: {other}")));
            }
        }
    }

    /// What the footer says while a run is active.
    pub fn status(&self) -> String {
        if let Some(pending) = &self.pending {
            return format!("waiting on you · {}", pending.name);
        }
        if self.running {
            let tools = self
                .rows
                .iter()
                .filter(|row| matches!(row, Row::Tool { .. }))
                .count();
            return match tools {
                0 => "working…".to_owned(),
                count => format!("working · {count} call(s)"),
            };
        }
        match self.session.as_ref() {
            Some(session) => format!("{} · {} turn(s)", session.root_name(), session.turns),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str) -> Event {
        Event {
            kind: kind.into(),
            ..Event::default()
        }
    }

    #[test]
    fn streamed_fragments_become_one_answer() {
        let mut run = AgentRun::default();
        for fragment in ["The ", "test ", "fails."] {
            run.apply(Event {
                content: fragment.into(),
                ..event("text")
            });
        }
        assert_eq!(run.rows, [Row::Answer("The test fails.".into())]);
    }

    #[test]
    fn a_tool_result_lands_on_the_call_it_answers() {
        let mut run = AgentRun::default();
        for (id, name) in [("c1", "read"), ("c2", "bash")] {
            run.apply(Event {
                tool_id: id.into(),
                name: name.into(),
                arguments: "{}".into(),
                ..event("tool_call")
            });
        }
        run.apply(Event {
            tool_id: "c2".into(),
            name: "bash".into(),
            content: "ok".into(),
            ..event("tool_result")
        });
        let results: Vec<Option<String>> = run
            .rows
            .iter()
            .map(|row| match row {
                Row::Tool { result, .. } => result.clone(),
                _ => panic!("only tool rows here"),
            })
            .collect();
        assert_eq!(results, [None, Some("ok".into())]);
    }

    #[test]
    fn a_call_without_an_id_still_gets_its_result() {
        let mut run = AgentRun::default();
        run.apply(Event {
            name: "read".into(),
            ..event("tool_call")
        });
        run.apply(Event {
            name: "read".into(),
            content: "text".into(),
            ..event("tool_result")
        });
        assert!(matches!(
            &run.rows[0],
            Row::Tool { result: Some(result), .. } if result == "text"
        ));
    }

    #[test]
    fn an_approval_waits_and_is_cleared_by_its_answer() {
        let mut run = AgentRun::default();
        run.apply(Event {
            approval: "abc".into(),
            name: "bash".into(),
            preview: "cargo test".into(),
            diff: None,
            ..event("needs_approval")
        });
        let pending = run.pending.clone().expect("a call is waiting");
        assert_eq!(pending.preview, "cargo test");
        assert!(!pending.is_block());
        assert!(run.busy(), "a waiting call is a busy run");
        assert!(run.status().contains("waiting on you · bash"));

        run.apply(Event {
            approval: "abc".into(),
            name: "bash".into(),
            ..event("approved")
        });
        assert!(run.pending.is_none());
        assert_eq!(
            run.rows,
            [Row::Decided {
                name: "bash".into(),
                allowed: true,
                for_session: false
            }]
        );
    }

    #[test]
    fn a_denial_and_a_stop_both_end_the_wait() {
        let mut run = AgentRun {
            running: true,
            ..Default::default()
        };
        run.apply(Event {
            approval: "abc".into(),
            name: "write".into(),
            ..event("needs_approval")
        });
        run.apply(Event {
            approval: "abc".into(),
            name: "write".into(),
            ..event("denied")
        });
        assert!(run.pending.is_none());
        assert!(run.running);
        assert!(!run.status().contains("waiting on you"), "{}", run.status());

        run.running = true;
        run.apply(event("stopped"));
        assert!(!run.running);
        assert!(matches!(run.rows.last(), Some(Row::Notice(text)) if text == "stopped"));
    }

    #[test]
    fn every_terminal_event_leaves_the_panel_idle() {
        for kind in ["done", "stopped", "error", "truncated"] {
            let mut run = AgentRun {
                running: true,
                ..Default::default()
            };
            run.apply(Event {
                approval: "abc".into(),
                ..event("needs_approval")
            });
            run.apply(event(kind));
            assert!(!run.running, "{kind} left the panel running");
            assert!(run.pending.is_none(), "{kind} left a call waiting");
        }
    }

    #[test]
    fn the_sequence_is_the_high_water_mark_so_a_reconnect_does_not_replay() {
        let mut run = AgentRun::default();
        run.apply(Event {
            seq: 4,
            ..event("text")
        });
        run.apply(Event {
            seq: 2,
            ..event("text")
        });
        assert_eq!(run.last_seq, 4);
    }

    #[test]
    fn reasoning_is_not_drawn_as_the_answer() {
        let mut run = AgentRun::default();
        run.apply(Event {
            content: "hmm".into(),
            ..event("reasoning")
        });
        run.apply(Event {
            content: "answer".into(),
            ..event("text")
        });
        assert_eq!(run.rows, [Row::Answer("answer".into())]);
    }

    #[test]
    fn an_unknown_event_is_named_rather_than_dropped_in_silence() {
        let mut run = AgentRun::default();
        run.apply(event("something-new"));
        assert!(
            matches!(run.rows.last(), Some(Row::Notice(text)) if text.contains("something-new"))
        );
    }

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

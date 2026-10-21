//! Client for the Sjel shell (`capabilities/sjel-status`), authenticated with the inbound token.
//!
//! The same contracts and token as the menu-bar app (`apps/mac/Sources/SjelRelay`): the token
//! comes from the login Keychain item that `tools/setup-inbound-auth.sh` writes, through
//! `/usr/bin/security`, which that script puts on the item's access list.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use futures::channel::mpsc::UnboundedReceiver;

/// The default shell endpoint, from `capabilities/sjel-status/service.toml`.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8082";
const KEYCHAIN_SERVICE: &str = "sjel-inbound-token";
const TICKET_PATH: &str = "/api/sjel-status/session/ticket";
const OPEN_PATH: &str = "/session/open?";
const EVENTS_PATH: &str = "/api/sjel-status/events";

/// `GET /api/sjel-status/health` (`SjelStatusHealth` in
/// `capabilities/sjel-status/src/status/registry.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct NodeHealth {
    pub version: String,
    pub uptime_seconds: u64,
    pub capabilities: BTreeMap<String, CapabilityHealth>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CapabilityHealth {
    pub up: bool,
    /// Kept running by the machine. A shell from before the field kept every capability running.
    #[serde(default = "kept_running")]
    pub autostart: bool,
}

fn kept_running() -> bool {
    true
}

/// What a capability's state means to the reader. On-demand capabilities that are stopped are
/// normal, not down (`NodeHealth.down` in `apps/mac/Sources/SjelRelay/NodeBridge.swift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    Up,
    Idle,
    Down,
}

impl CapabilityHealth {
    pub fn liveness(self) -> Liveness {
        match (self.up, self.autostart) {
            (true, _) => Liveness::Up,
            (false, false) => Liveness::Idle,
            (false, true) => Liveness::Down,
        }
    }
}

impl NodeHealth {
    /// Kept-running capabilities that did not answer, by name.
    pub fn down(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|(_, health)| health.liveness() == Liveness::Down)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// `GET /api/sjel-status/agent`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentView {
    #[serde(default)]
    pub pending: Vec<Approval>,
    /// Newest first, at most 50.
    #[serde(default)]
    pub calls: Vec<AgentCall>,
}

/// One agent write that waits for the owner.
#[derive(Clone, Debug, Deserialize)]
pub struct Approval {
    pub id: String,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub preview: String,
}

/// One call an agent made through the gate (`AgentCall` in `libs/sjel-server/src/agent_policy.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct AgentCall {
    /// Unix seconds.
    pub at: i64,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// `read` for a plain read; anything else is a write or a refusal.
    pub decision: String,
}

/// `GET /calendar/api/entries` (`CalendarEntry` in `dashboard/src/lib/api.ts`).
#[derive(Clone, Debug, Deserialize)]
pub struct CalendarEntry {
    pub title: String,
    /// `YYYY-MM-DD` when `all_day`, else local `YYYY-MM-DDTHH:MM:SS`.
    pub starts_at: String,
    /// Exclusive for an all-day entry.
    pub ends_at: String,
    pub all_day: bool,
    /// `possible`, `planned` or `committed`.
    pub commitment: String,
}

/// `GET /projects/api/projects` (`ProjectSummary` in `capabilities/projects/src/server.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub open: usize,
    #[serde(default)]
    pub done: usize,
    #[serde(default)]
    pub notes: usize,
}

/// One record of `GET /projects/api/project` and `GET /projects/api/record`.
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectRecord {
    pub path: String,
    /// `project`, `task` or `note`.
    pub kind: String,
    pub title: String,
    /// Absent from a list read with `content=false`; the note editor reads the record first.
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub due: Option<String>,
    /// 1 to 3; the capability reads a missing or other value as 2.
    #[serde(default)]
    pub priority: u8,
}

/// One task across projects (`GET /projects/api/tasks`, and the reply to every task write).
#[derive(Clone, Debug, Deserialize)]
pub struct GlobalTask {
    /// The record's path, which every write takes as `id` or `path`.
    pub id: String,
    /// `task` or `note`.
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub done: bool,
    pub due: Option<String>,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub project: String,
}

/// One note across projects (`GET /projects/api/records`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GlobalNote {
    pub path: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
}

/// The soundscape playback state (`GET /soundscape/api/soundscape/state`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundscapeState {
    pub preset: String,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub volume: f32,
    #[serde(default)]
    pub energy: f32,
}

/// One message in an Ask conversation (`POST /assistant/api/ask`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskTurn {
    pub role: String,
    pub content: String,
}

/// Reply from `POST /assistant/api/ask` (or deterministic capability fallback).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AskReply {
    pub text: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// One frame of `GET /api/sjel-status/events`: both answers, pushed when either changes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Snapshot {
    pub health: Option<NodeHealth>,
    pub agent: Option<AgentView>,
    /// Why the shell could not read one of the two answers.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// The endpoint is not on loopback.
    NotLoopback,
    /// No token in the Keychain.
    NotSetUp,
    /// Nothing answered.
    Unreachable,
    /// The shell answered with an error status, and its `error` text when it sent one.
    Refused(u16, Option<String>),
    /// The shell answered 200 with a body this client cannot read.
    Unreadable,
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback => {
                f.write_str("SJEL_STATUS_URL must point to 127.0.0.1 or localhost.")
            }
            Self::NotSetUp => {
                f.write_str("No Sjel token on this Mac. Run tools/setup-inbound-auth.sh.")
            }
            Self::Unreachable => f.write_str("Sjel is not running on this Mac."),
            Self::Refused(_, Some(error)) => f.write_str(error),
            Self::Refused(status, None) => write!(f, "Sjel refused the request ({status})."),
            Self::Unreadable => f.write_str("Sjel answered with something this app cannot read."),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;

pub struct Shell {
    base: Url,
    client: reqwest::Client,
    token: Mutex<Option<String>>,
}

impl Shell {
    /// # Errors
    /// `NotLoopback` when `endpoint` is not an `http` URL on 127.0.0.1 or localhost.
    pub fn new(endpoint: &str) -> ShellResult<Self> {
        let base = Url::parse(endpoint).map_err(|_| ShellError::NotLoopback)?;
        let loopback =
            base.scheme() == "http" && matches!(base.host_str(), Some("127.0.0.1" | "localhost"));
        if !loopback {
            return Err(ShellError::NotLoopback);
        }
        Ok(Self {
            base,
            client: reqwest::Client::new(),
            token: Mutex::new(None),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    pub async fn health(&self) -> ShellResult<NodeHealth> {
        self.json(Method::GET, "/api/sjel-status/health", None, 10)
            .await
    }

    pub async fn agent(&self) -> ShellResult<AgentView> {
        self.json(Method::GET, "/api/sjel-status/agent", None, 5)
            .await
    }

    /// Entries from `from` to `to`, both `YYYY-MM-DD`, both inclusive.
    pub async fn calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        let path = format!("/calendar/api/entries?from={from}&to={to}");
        self.json(Method::GET, &path, None, 5).await
    }

    pub async fn projects(&self) -> ShellResult<Vec<ProjectSummary>> {
        #[derive(Deserialize)]
        struct List {
            projects: Vec<ProjectSummary>,
        }
        let list: List = self
            .json(Method::GET, "/projects/api/projects", None, 5)
            .await?;
        Ok(list.projects)
    }

    /// A project's records without their content: what a list of notes draws.
    pub async fn project(&self, name: &str) -> ShellResult<Vec<ProjectRecord>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<ProjectRecord>,
        }
        let path = with_query(
            "/projects/api/project",
            &[("name", name), ("content", "false")],
        );
        let records: Records = self.json(Method::GET, &path, None, 10).await?;
        Ok(records.records)
    }

    /// One record with its content: what the note editor opens.
    pub async fn record(&self, path: &str) -> ShellResult<ProjectRecord> {
        #[derive(Deserialize)]
        struct One {
            record: ProjectRecord,
        }
        let path = with_query("/projects/api/record", &[("path", path)]);
        let one: One = self.json(Method::GET, &path, None, 10).await?;
        Ok(one.record)
    }

    /// Every note across projects (`GET /projects/api/records`).
    pub async fn notes(&self) -> ShellResult<Vec<GlobalNote>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<RecordItem>,
        }
        #[derive(Deserialize)]
        struct RecordItem {
            id: String,
            #[serde(default)]
            kind: String,
            title: String,
            #[serde(default)]
            project: String,
        }
        let notes_req: Result<Records, _> = self
            .json(
                Method::GET,
                "/projects/api/records?kind=note&content=false",
                None,
                10,
            )
            .await;
        let mut list = Vec::new();
        if let Ok(records) = notes_req {
            for r in records.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        if let Ok(projects) = self
            .json::<Records>(
                Method::GET,
                "/projects/api/records?kind=project&content=false",
                None,
                10,
            )
            .await
        {
            for r in projects.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        list.sort_by(|a, b| a.project.cmp(&b.project).then_with(|| a.title.cmp(&b.title)));
        Ok(list)
    }

    /// Replaces a record's content. `base` is the content the edit started from; the capability
    /// answers 409 and writes nothing when the record changed since.
    pub async fn save_record(&self, path: &str, content: &str, base: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path, "content": content, "base": base });
        self.send(Method::POST, "/projects/api/record", Some(body), 10)
            .await
            .map(drop)
    }

    /// Marks a task done, or open again, and answers with the record's new frontmatter.
    pub async fn set_task_done(&self, path: &str, done: bool) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "id": path, "done": done });
        let task: Task = self
            .json(Method::POST, "/projects/api/tasks/done", Some(body), 10)
            .await?;
        Ok(task.task)
    }

    /// Sets one frontmatter key and answers with the record's new frontmatter, so a caller can
    /// update one row instead of reading the project again.
    pub async fn set_property(
        &self,
        path: &str,
        key: &str,
        value: &str,
    ) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "path": path, "key": key, "value": value });
        let task: Task = self
            .json(
                Method::POST,
                "/projects/api/record/property",
                Some(body),
                10,
            )
            .await?;
        Ok(task.task)
    }

    pub async fn add_task(&self, project: &str, title: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "project": project, "title": title });
        self.send(Method::POST, "/projects/api/tasks", Some(body), 10)
            .await
            .map(drop)
    }

    /// Tasks across all live projects, optionally filtered by ?status=open|done.
    pub async fn tasks(&self, status: Option<&str>) -> ShellResult<Vec<GlobalTask>> {
        #[derive(Deserialize)]
        struct List {
            tasks: Vec<GlobalTask>,
        }
        let path = match status {
            Some(s) => format!("/projects/api/tasks?status={s}"),
            None => "/projects/api/tasks".to_string(),
        };
        let list: List = self.json(Method::GET, &path, None, 10).await?;
        Ok(list.tasks)
    }

    /// Creates a project hub.
    pub async fn add_project(&self, name: &str, summary: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "name": name, "summary": summary });
        self.send(Method::POST, "/projects/api/projects", Some(body), 10)
            .await
            .map(drop)
    }

    /// Creates a new note in a live project.
    pub async fn add_note(&self, project: &str, title: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "project": project, "title": title, "summary": "" });
        self.send(Method::POST, "/projects/api/notes", Some(body), 10)
            .await
            .map(drop)
    }

    /// Removes a record from the projects store and disk.
    pub async fn remove_record(&self, path: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path });
        self.send(Method::POST, "/projects/api/record/remove", Some(body), 10)
            .await
            .map(drop)
    }

    /// Allows or denies one waiting agent write.
    pub async fn decide(&self, id: &str, allow: bool) -> ShellResult<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not an approval id.".into())));
        }
        let path = format!("/api/sjel-status/agent/approvals/{id}");
        let body = serde_json::json!({ "allow": allow });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Starts or stops one capability. A start waits for the service to come up.
    pub async fn set_running(&self, name: &str, running: bool) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let action = if running { "start" } else { "stop" };
        let path = format!("/api/sjel-status/capabilities/{name}/{action}");
        self.send(Method::POST, &path, None, 60).await.map(drop)
    }

    /// Follows the shell's change stream. The receiver ends when the shell has no stream to
    /// send, so a caller that gets no frame keeps reading on its own timer; after a restart the
    /// connection is reopened silently.
    ///
    /// # Errors
    /// `NotSetUp`, `NotLoopback` or `Unreadable`, before any connection is opened.
    pub fn subscribe(&self) -> ShellResult<UnboundedReceiver<Snapshot>> {
        let token = self.token()?;
        let url = self
            .base
            .join(EVENTS_PATH)
            .map_err(|_| ShellError::Unreadable)?;
        let (to, from) = futures::channel::mpsc::unbounded();
        let client = self.client.clone();
        spawn(async move {
            while !to.is_closed() {
                match follow(&client, &url, &token, &to).await {
                    Followed::Unsupported => return,
                    Followed::Lost => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
        Ok(from)
    }

    /// Runs one turn in a native Ask session through `POST /assistant/api/ask`, starting the
    /// on-demand `assistant` capability if it is stopped, and falling back to a direct capability
    /// summary when the local model role is unavailable.
    pub async fn ask(&self, prompt: &str, history: &[AskTurn]) -> ShellResult<AskReply> {
        let body = serde_json::json!({ "prompt": prompt, "history": history });
        match self
            .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body.clone()), 60)
            .await
        {
            Ok(reply) => return Ok(reply),
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                if let Ok(reply) = self
                    .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body), 60)
                    .await
                {
                    return Ok(reply);
                }
            }
            Err(ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback) => {
                return self
                    .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body), 5)
                    .await;
            }
            Err(_) => {}
        }
        self.capability_fallback(prompt).await
    }

    async fn capability_fallback(&self, prompt: &str) -> ShellResult<AskReply> {
        let health = self.health().await?;
        let up = health.capabilities.values().filter(|c| c.up).count();
        let down = health.down();
        let projects = self.projects().await.unwrap_or_default();
        let mut lines = vec![format!(
            "Sjel {} (up {}m): {} of {} capabilities running{}.",
            health.version,
            health.uptime_seconds / 60,
            up,
            health.capabilities.len(),
            if down.is_empty() {
                String::new()
            } else {
                format!("; down: {}", down.join(", "))
            }
        )];
        if !projects.is_empty() {
            let names: Vec<String> = projects
                .iter()
                .filter(|p| !p.archived)
                .map(|p| format!("{} ({})", p.name, p.notes))
                .collect();
            lines.push(format!("Active projects: {}.", names.join(", ")));
        }
        let _ = prompt;
        Ok(AskReply {
            text: lines.join("\n"),
            model: "capabilities".into(),
            source: "deterministic".into(),
            tools: vec!["sjel_health".into(), "projects_list".into()],
        })
    }

    /// A single-use URL that logs the browser in and lands on `next`
    /// (`landing` in `capabilities/sjel-status/src/session.rs`).
    pub async fn dashboard_url(&self, next: &str) -> ShellResult<Url> {
        #[derive(Deserialize)]
        struct Ticket {
            open: String,
        }
        let ticket: Ticket = self.json(Method::POST, TICKET_PATH, None, 5).await?;
        landing_url(&self.base, &ticket.open, next).ok_or(ShellError::Unreadable)
    }

    /// Read soundscape state (`GET /soundscape/api/soundscape/state`).
    pub async fn soundscape_state(&self) -> ShellResult<SoundscapeState> {
        self.json(Method::GET, "/soundscape/api/soundscape/state", None, 3).await
    }

    /// Update playback status (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_playing(&self, playing: bool) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "playing": playing,
            "origin": "gpui",
        });
        self.json(Method::POST, "/soundscape/api/soundscape/state", Some(body), 3).await
    }

    /// Update soundscape preset (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_preset(&self, preset: &str) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "preset": preset,
            "origin": "gpui",
        });
        self.json(Method::POST, "/soundscape/api/soundscape/state", Some(body), 3).await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<T> {
        let response = self.send(method, path, body, timeout_secs).await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        let token = self.token()?;
        let url = self.base.join(path).map_err(|_| ShellError::Unreadable)?;
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| ShellError::Unreachable)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            // The token was rotated: read the Keychain again on the next request.
            *self.token.lock().expect("token lock") = None;
        }
        if status.is_success() {
            return Ok(response);
        }
        let error = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
        Err(ShellError::Refused(status.as_u16(), error))
    }

    fn token(&self) -> ShellResult<String> {
        let mut cached = self.token.lock().expect("token lock");
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let user = std::env::var("USER").map_err(|_| ShellError::NotSetUp)?;
        let output = Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                &user,
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .output()
            .map_err(|_| ShellError::NotSetUp)?;
        let token = String::from_utf8(output.stdout).map_err(|_| ShellError::NotSetUp)?;
        let token = token.trim();
        if !output.status.success() || token.is_empty() {
            return Err(ShellError::NotSetUp);
        }
        *cached = Some(token.to_owned());
        Ok(token.to_owned())
    }
}

/// A path with its query, each pair encoded once. The capability owns the names; this only joins
/// them, so a project name with a `/` or a space reaches it intact.
fn with_query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
    url.set_path(path);
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// Why one connection to the change stream ended.
enum Followed {
    /// The shell answered, but does not serve the stream: a build from before it existed.
    Unsupported,
    /// The shell or the network dropped it. Reconnecting is worth a try.
    Lost,
}

async fn follow(
    client: &reqwest::Client,
    url: &Url,
    token: &str,
    to: &UnboundedSender<Snapshot>,
) -> Followed {
    let response = client
        .get(url.clone())
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await;
    let Ok(response) = response else {
        return Followed::Lost;
    };
    let status = response.status();
    if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
        return Followed::Unsupported;
    }
    if !status.is_success() {
        return Followed::Lost;
    }
    let mut response = response;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Followed::Lost,
            Err(_) => return Followed::Lost,
        };
        buffer.extend_from_slice(&chunk);
        while let Some(data) = next_frame(&mut buffer) {
            if let Ok(snapshot) = serde_json::from_str::<Snapshot>(&data) {
                if to.unbounded_send(snapshot).is_err() {
                    return Followed::Unsupported;
                }
            }
        }
    }
}

/// One event's data, if a whole event has arrived. A comment line carries no data and yields an
/// empty string, which no snapshot parses as, so it passes through unremarked.
fn next_frame(buffer: &mut Vec<u8>) -> Option<String> {
    let end = buffer.windows(2).position(|pair| pair == b"\n\n")?;
    let block = String::from_utf8_lossy(&buffer[..end]).into_owned();
    buffer.drain(..end + 2);
    let mut data = String::new();
    for line in block.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    }
    Some(data)
}

/// Letters, digits and dashes: nothing a path can be built from.
fn is_registry_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Accepts only `/session/open?…` on the shell's own origin, so a wrong answer cannot send the
/// browser anywhere else.
fn landing_url(base: &Url, open: &str, next: &str) -> Option<Url> {
    if !open.starts_with(OPEN_PATH) {
        return None;
    }
    let mut url = base.join(open).ok()?;
    if url.origin() != base.origin() {
        return None;
    }
    url.query_pairs_mut().append_pair("next", next);
    Some(url)
}

/// Two worker threads for every request. GPUI's executor does not provide Tokio's reactor,
/// which reqwest needs. A token read through `/usr/bin/security` blocks its worker, so one
/// read must not be able to stall an unrelated request.
fn runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: LazyLock<tokio::runtime::Handle> = LazyLock::new(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build the HTTP runtime");
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("sjel-http".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("start the HTTP thread");
        handle
    });
    &RUNTIME
}

/// Runs `work` on the HTTP runtime; the returned future can be awaited from GPUI.
pub fn run<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(work.await);
    });
    async move { rx.await.ok() }
}

/// Runs `work` on the HTTP runtime with nothing waiting for its answer, for a connection that
/// outlives one call.
pub fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(work);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_must_be_loopback_http() {
        assert!(Shell::new("http://127.0.0.1:8082").is_ok());
        assert!(Shell::new("http://localhost:9000/").is_ok());
        assert_eq!(
            Shell::new("https://127.0.0.1:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://example.com:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://127.0.0.1.evil:8082").err(),
            Some(ShellError::NotLoopback)
        );
    }

    #[test]
    fn landing_stays_on_the_shell() {
        let base = Url::parse(DEFAULT_ENDPOINT).unwrap();
        let url = landing_url(&base, "/session/open?ticket=ab", "/calendar").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8082/session/open?ticket=ab&next=%2Fcalendar"
        );
        assert!(landing_url(&base, "//evil.example/session/open?x", "/").is_none());
        assert!(landing_url(&base, "/elsewhere?ticket=ab", "/").is_none());
    }

    #[test]
    fn stopped_on_demand_capability_is_idle_not_down() {
        let health: NodeHealth = serde_json::from_str(
            r#"{"ok":true,"version":"v1","uptime_seconds":5,"capabilities":{
                "a":{"up":true,"autostart":true},
                "b":{"up":false,"autostart":false},
                "c":{"up":false}
            }}"#,
        )
        .unwrap();
        assert_eq!(health.capabilities["b"].liveness(), Liveness::Idle);
        assert_eq!(health.down(), vec!["c".to_owned()]);
    }

    #[test]
    fn registry_names_reject_paths() {
        assert!(is_registry_name("sjel-status"));
        assert!(!is_registry_name("../x"));
        assert!(!is_registry_name(""));
    }

    #[test]
    fn a_query_encodes_what_a_project_name_can_hold() {
        assert_eq!(
            with_query(
                "/projects/api/project",
                &[("name", "Home Lab"), ("content", "false")]
            ),
            "/projects/api/project?name=Home+Lab&content=false"
        );
        assert_eq!(
            with_query("/projects/api/record", &[("path", "A/B & C.md")]),
            "/projects/api/record?path=A%2FB+%26+C.md"
        );
    }

    #[test]
    fn a_list_read_without_content_still_parses() {
        let record: ProjectRecord = serde_json::from_str(
            r#"{"path":"P/a.md","kind":"note","title":"A","done":false,
                "due":null,"priority":2}"#,
        )
        .unwrap();
        assert_eq!(record.content, "");
        assert_eq!(record.title, "A");
    }

    #[test]
    fn a_task_reply_carries_the_path_as_its_id() {
        let task: GlobalTask = serde_json::from_str(
            r#"{"id":"P/t.md","kind":"task","title":"T","done":true,
                "due":"2026-10-24","priority":1,"project":"P"}"#,
        )
        .unwrap();
        assert_eq!(task.id, "P/t.md");
        assert!(task.done);
    }

    #[test]
    fn frames_come_out_whole_and_leave_the_rest() {
        let mut buffer =
            b"event: snapshot\ndata: {\"agent\":1}\n\n: keep-alive\n\nevent: snapshot\ndata: {\"hea"
                .to_vec();
        assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"agent\":1}"));
        // A comment carries no data and is not a snapshot.
        assert_eq!(next_frame(&mut buffer).as_deref(), Some(""));
        // The third event is still split across chunks.
        assert_eq!(next_frame(&mut buffer), None);
        buffer.extend_from_slice(b"lth\":2}\n\n");
        assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"health\":2}"));
    }

    #[test]
    fn a_frame_carries_both_answers_or_why_not() {
        let both: Snapshot = serde_json::from_str(
            r#"{"health":{"version":"v","uptime_seconds":1,"capabilities":{}},
                "agent":{"pending":[],"calls":[]}}"#,
        )
        .unwrap();
        assert!(both.health.is_some() && both.agent.is_some());
        assert!(both.error.is_none());

        let failed: Snapshot = serde_json::from_str(r#"{"error":"agent policy: x"}"#).unwrap();
        assert_eq!(failed.error.as_deref(), Some("agent policy: x"));
        assert!(failed.health.is_none());
    }
}

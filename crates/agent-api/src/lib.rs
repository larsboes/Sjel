#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// --- Route Constants ---

pub const DEFAULT_AGENT_PORT: u16 = 8108;
pub const ROUTE_HEALTH: &str = "/health";
pub const ROUTE_READY: &str = "/ready";
pub const ROUTE_ROUTES: &str = "/routes";
pub const ROUTE_AGENT_SESSIONS: &str = "/api/agent/sessions";
pub const ROUTE_AGENT_SESSION: &str = "/api/agent/sessions/{id}";
pub const ROUTE_AGENT_SESSION_TRANSCRIPT: &str = "/api/agent/sessions/{id}/transcript";
pub const ROUTE_AGENT_SESSION_STREAM: &str = "/api/agent/sessions/{id}/stream";
pub const ROUTE_AGENT_SESSION_PROMPT: &str = "/api/agent/sessions/{id}/prompt";
pub const ROUTE_AGENT_SESSION_APPROVE: &str = "/api/agent/sessions/{id}/approve";
pub const ROUTE_AGENT_SESSION_STEER: &str = "/api/agent/sessions/{id}/steer";
pub const ROUTE_AGENT_SESSION_STOP: &str = "/api/agent/sessions/{id}/stop";
pub const ROUTE_AGENT_TERMINALS: &str = "/api/agent/terminals";
pub const ROUTE_AGENT_TERMINALS_OPTIONS: &str = "/api/agent/terminals/options";
pub const ROUTE_AGENT_TERMINAL_STREAM: &str = "/api/agent/terminals/{id}/stream";
pub const ROUTE_AGENT_TERMINAL_INPUT: &str = "/api/agent/terminals/{id}/input";
pub const ROUTE_AGENT_TERMINAL_RESIZE: &str = "/api/agent/terminals/{id}/resize";
pub const ROUTE_AGENT_TERMINAL_DELETE: &str = "/api/agent/terminals/{id}";

// --- Tiers ---

/// What a session may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Sjel's own data: capability routes and read-only SQL. No shell, no writes.
    Sjel,
    /// One checkout: the coding tools, inside a root under `$HOME`.
    Project,
    /// The user account: the same tools, reaching all of `$HOME`. `sudo` stays blocked.
    Account,
}

pub const DEFAULT_TIER: Tier = Tier::Project;

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Sjel, Tier::Project, Tier::Account];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Sjel => "sjel",
            Tier::Project => "project",
            Tier::Account => "account",
        }
    }

    pub fn parse(text: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.as_str() == text)
    }

    /// The plugins this tier assembles.
    pub fn plugins(self) -> Vec<String> {
        let names: &[&str] = match self {
            Tier::Sjel => &["sjel", "sql", "guard", "compaction"],
            Tier::Project | Tier::Account => &["fs", "sjel", "sql", "guard", "compaction"],
        };
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The root this tier accepts, or the reason it refuses the one asked for.
    pub fn root(self, requested: &Path, home: &Path) -> Result<PathBuf, String> {
        if self == Tier::Sjel {
            return Ok(requested.to_path_buf());
        }
        if !requested.starts_with(home) {
            return Err(format!(
                "a `{}` session reaches only inside {}. {} is outside it",
                self.as_str(),
                home.display(),
                requested.display()
            ));
        }
        Ok(requested.to_path_buf())
    }

    /// The directory a call in this session may touch, given the session's root. `None` is the
    /// tier that reaches no filesystem at all.
    pub fn reach(self, root: &Path, home: &Path) -> Option<PathBuf> {
        match self {
            Tier::Sjel => None,
            Tier::Project => Some(root.to_path_buf()),
            Tier::Account => Some(home.to_path_buf()),
        }
    }
}

// --- Session Requests & Envelopes ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateSession {
    /// Absolute path the run works in. Absent means `$HOME` for an `account` session and this
    /// capability's checkout otherwise.
    #[serde(default)]
    pub root: Option<String>,
    /// `sjel`, `project` or `account`. Absent means `project`. The tier sets the plugin set and
    /// the scope, and cannot be changed afterwards.
    #[serde(default)]
    pub tier: Option<String>,
    /// Plugin names, to narrow the tier's own set. Absent means the whole tier. A name the tier
    /// does not carry is refused at creation rather than dropped, so a client cannot believe it
    /// widened the session.
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PromptRequest {
    pub prompt: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SteerRequest {
    pub message: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApproveRequest {
    pub approval_id: String,
    pub allow: bool,
    /// `once`, or `session` to allow this exact call for the rest of the session. Anything else
    /// is refused rather than read as one of them.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StreamQuery {
    /// The last sequence the caller already has. A reconnect sends what it saw last.
    #[serde(default)]
    pub from: Option<u64>,
}

// --- Terminal Requests & Envelopes ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenTerminal {
    /// A preset name from `<overlay>/config/agent.toml`, or one of the five defaults. An empty
    /// string is the login shell.
    pub preset: String,
    /// An existing directory under `$HOME`. Absent means `$HOME`.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalInput {
    /// Bytes to write to the pty, base64. At most 64 KiB, before decoding.
    pub data: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalResize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalStreamQuery {
    /// The byte offset the caller already has. Absent means the oldest output still held.
    #[serde(default)]
    pub from: Option<u64>,
}

// --- Session & Terminal Projections ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct SessionMeta {
    pub id: String,
    pub root: String,
    pub tier: String,
    #[serde(default)]
    pub reach: Option<String>,
    pub created_at: u64,
    pub plugins: Vec<String>,
    pub turns: u64,
    pub active: bool,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct TerminalMeta {
    pub id: String,
    pub preset: String,
    pub title: String,
    pub cwd: String,
    pub started_at: u64,
    pub last_output_at: u64,
    pub alive: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exit_signal: Option<String>,
    pub clients: usize,
    #[serde(default)]
    pub shell: Option<serde_json::Value>,
}

/// One conversation, as the capability lists it on the wire.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
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

/// What a shell with shell integration reports about its commands, from OSC 133 marks.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommandMarks {
    #[serde(default)]
    pub integration: bool,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub last_exit_code: Option<i32>,
    #[serde(default)]
    pub last_finished_at: Option<i64>,
}

/// One terminal the capability holds, as `GET /api/agent/terminals` lists it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Terminal {
    pub id: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub last_output_at: i64,
    #[serde(default)]
    pub alive: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exit_signal: Option<String>,
    #[serde(default)]
    pub clients: usize,
    #[serde(default)]
    pub shell: CommandMarks,
}

impl Terminal {
    /// What a list row says: the program, or the preset when the program gave no name.
    pub fn label(&self) -> String {
        match self.title.trim() {
            "" => self.preset.clone(),
            title => title.to_owned(),
        }
    }

    /// The last path component of the working directory, for a row that has no room for the rest.
    pub fn directory(&self) -> String {
        self.cwd
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.cwd)
            .to_owned()
    }

    /// How it ended, for the line under the grid.
    pub fn ending(&self) -> Option<String> {
        if self.alive {
            return None;
        }
        Some(match (&self.exit_signal, self.exit_code) {
            (Some(signal), _) => format!("terminated by {signal}"),
            (None, Some(code)) => format!("exited with {code}"),
            (None, None) => "ended".to_owned(),
        })
    }

    /// What the shell says about the command it is running, for a status line beside `live`.
    pub fn command(&self) -> Option<String> {
        if !self.shell.integration {
            return None;
        }
        if self.shell.running {
            return Some("running".to_owned());
        }
        match self.shell.last_exit_code {
            Some(0) | None => None,
            Some(code) => Some(format!("exit {code}")),
        }
    }
}

/// What a new terminal can be, as `GET /api/agent/terminals/options` answers.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TerminalOptions {
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub folders: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/Users/someone")
    }

    #[test]
    fn tier_round_trips_and_names_itself() {
        for tier in Tier::ALL {
            let text = tier.as_str();
            assert_eq!(Tier::parse(text), Some(tier));
        }
        assert_eq!(Tier::parse("invalid"), None);
    }

    #[test]
    fn create_session_serde() {
        let req = CreateSession {
            root: Some("/tmp".into()),
            tier: Some("project".into()),
            plugins: Some(vec!["fs".into()]),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: CreateSession = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn a_root_outside_the_account_is_refused_with_the_reason() {
        let inside = home().join("Developer/Sjel");
        assert_eq!(Tier::Project.root(&inside, &home()), Ok(inside.clone()));
        assert_eq!(Tier::Account.root(&inside, &home()), Ok(inside));

        let outside = PathBuf::from("/etc");
        let refused = Tier::Project.root(&outside, &home()).unwrap_err();
        assert!(refused.contains("outside"), "{refused}");
        assert!(Tier::Account.root(&outside, &home()).is_err());
    }

    #[test]
    fn a_project_session_reaches_its_root_and_an_account_session_reaches_home() {
        let root = home().join("Developer/Sjel");
        assert_eq!(Tier::Project.reach(&root, &home()), Some(root.clone()));
        assert_eq!(Tier::Account.reach(&root, &home()), Some(home()));
        assert_eq!(Tier::Sjel.reach(&root, &home()), None);
    }
}

//! Pure wire models, backup records, agent gates, health models, and route constants for Sjel status.
//!
//! Shared across `crates/sjel-status`, `crates/backup`, and `apps/gpui/core`. Zero platform dependencies (pure `serde`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// --- Route Constants ---

pub const ROUTE_STATUS_HEALTH: &str = "/api/sjel-status/health";
pub const ROUTE_STATUS_ROUTES: &str = "/api/sjel-status/routes";
pub const ROUTE_STATUS_NOW: &str = "/api/sjel-status/now";
pub const ROUTE_STATUS_SEARCH: &str = "/api/sjel-status/search";
pub const ROUTE_STATUS_LAN: &str = "/api/sjel-status/lan";
pub const ROUTE_STATUS_CAPABILITIES: &str = "/api/sjel-status/capabilities";
pub const ROUTE_STATUS_SELF: &str = "/api/sjel-status/self";
pub const ROUTE_STATUS_LINKS: &str = "/api/sjel-status/links";
pub const ROUTE_STATUS_REPOS: &str = "/api/sjel-status/repos";
pub const ROUTE_STATUS_EVENTS: &str = "/api/sjel-status/events";
pub const ROUTE_STATUS_BACKUPS: &str = "/api/sjel-status/backups";
pub const ROUTE_STATUS_BACKUP_TARGETS: &str = "/api/sjel-status/backup/targets";
pub const ROUTE_STATUS_BACKUP_RUNS: &str = "/api/sjel-status/backup/runs";
pub const ROUTE_STATUS_BACKUP_POLICY: &str = "/api/sjel-status/backup/policy";
pub const ROUTE_STATUS_BACKUP_VERIFY: &str = "/api/sjel-status/backup/verify";
pub const ROUTE_STATUS_AGENT: &str = "/api/sjel-status/agent";
pub const ROUTE_STATUS_AGENT_MODE: &str = "/api/sjel-status/agent/mode";
pub const ROUTE_STATUS_STORAGE: &str = "/api/sjel-status/storage";
pub const ROUTE_STATUS_STORAGE_HISTORY: &str = "/api/sjel-status/storage-history";
pub const ROUTE_STATUS_UPDATES: &str = "/api/sjel-status/updates";
pub const ROUTE_STATUS_UPDATES_APPLY: &str = "/api/sjel-status/updates/apply";
pub const ROUTE_STATUS_HOST_WATCH: &str = "/api/sjel-status/host-watch";
pub const ROUTE_STATUS_RUNTIME: &str = "/api/sjel-status/runtime";
pub const ROUTE_STATUS_PACKS: &str = "/api/sjel-status/packs";
pub const ROUTE_STATUS_CV: &str = "/api/sjel-status/cv";
pub const ROUTE_STATUS_SETUP: &str = "/api/sjel-status/setup";
pub const ROUTE_STATUS_SETUP_VERIFY: &str = "/api/sjel-status/setup/verify";
pub const ROUTE_STATUS_SETUP_APPLY: &str = "/api/sjel-status/setup/apply";
pub const ROUTE_STATUS_CREDENTIALS: &str = "/api/sjel-status/credentials";

// --- Node & Capability Health ---

pub fn default_kept_running() -> bool {
    true
}

/// What a capability's state means to the reader. On-demand capabilities that are stopped are
/// normal, not down (`NodeHealth.down` in `apps/mac/Sources/SjelRelay/NodeBridge.swift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Liveness {
    Up,
    Idle,
    Down,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilityHealth {
    pub up: bool,
    /// Kept running by the machine. A shell from before the field kept every capability running.
    #[serde(default = "default_kept_running")]
    pub autostart: bool,
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

/// `GET /api/sjel-status/health` (`SjelStatusHealth` in `capabilities/sjel-status/src/status/registry.rs`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct NodeHealth {
    #[serde(default)]
    pub ok: bool,
    pub version: String,
    pub uptime_seconds: u64,
    pub capabilities: BTreeMap<String, CapabilityHealth>,
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

/// One registry entry (`GET /api/sjel-status/capabilities`), as far as related-record links read it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RegistryEntry {
    pub name: String,
    #[serde(default)]
    pub links_to: Vec<String>,
}

// --- Agent Gate & Approvals ---

/// `GET /api/sjel-status/agent`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentView {
    /// False until this machine holds an agent token: every mode below is then unused.
    #[serde(default)]
    pub enrolled: bool,
    /// Every mode the policy admits: `off`, `read-only`, `ask`, `auto`.
    #[serde(default)]
    pub modes: Vec<String>,
    /// The mode a capability with no entry runs at.
    #[serde(default)]
    pub default_mode: String,
    /// One entry per capability that has started a gate on this machine.
    #[serde(default)]
    pub capabilities: Vec<AgentGate>,
    #[serde(default)]
    pub pending: Vec<Approval>,
    /// Newest first, at most 50.
    #[serde(default)]
    pub calls: Vec<AgentCall>,
}

/// One capability's agent gate: the mode it runs at, and the routes that always ask.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentGate {
    pub capability: String,
    /// `off` · `read-only` · `ask` · `auto`.
    #[serde(default)]
    pub mode: String,
    /// Routes recorded as `METHOD /path` that always wait for the owner.
    #[serde(default)]
    pub confirm: Vec<String>,
    /// Writes reachable over GET, recorded as `METHOD /path`.
    #[serde(default, rename = "get_writes")]
    pub get_writes: Vec<String>,
}

/// One agent write that waits for the owner.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub preview: String,
    /// Unix seconds.
    #[serde(default)]
    pub created_at: i64,
    /// `pending`, `allowed`, `denied` or `used`.
    #[serde(default)]
    pub state: String,
}

/// One call an agent made through the gate (`AgentCall` in `libs/sjel-server/src/agent_policy.rs`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

// --- Backup Status & Records ---

/// Target record as returned by `/api/sjel-status/backup/targets`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupTargetRecord {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub present: String,
    #[serde(default)]
    pub declared_by: Vec<String>,
    #[serde(default)]
    pub seen_at: String,
    #[serde(default)]
    pub verified_at: Option<String>,
    #[serde(default)]
    pub verified_verdict: Option<String>,
    #[serde(default)]
    pub verified_detail: Option<String>,
    #[serde(default)]
    pub interval_hours: Option<i64>,
}

pub type TargetRow = BackupTargetRecord;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupTargetsResponse {
    pub targets: Vec<BackupTargetRecord>,
}

/// Archive details recorded on successful run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveIdentity {
    pub name: String,
    pub bytes: i64,
    pub sha256: String,
}

/// A recorded run attempt from `/api/sjel-status/backup/runs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupRunAttemptRecord {
    pub id: i64,
    pub capability: String,
    pub target: String,
    pub started_at: String,
    pub started_epoch: i64,
    pub finished_at: Option<String>,
    pub finished_epoch: Option<i64>,
    pub exit_code: Option<i64>,
    pub archive: Option<ArchiveIdentity>,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub log_path: String,
}

pub type RunRow = BackupRunAttemptRecord;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackupRunsResponse {
    pub runs: Vec<BackupRunAttemptRecord>,
    pub running: usize,
}

/// Rehearsal verify response from `/api/sjel-status/backup/verify`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifyTargetResult {
    pub target: String,
    pub verdict: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BackupRun {
    /// `running`, `succeeded` or `failed`.
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

/// One capability with a backup contract.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BackupStatus {
    pub capability: String,
    /// `ok`, `due`, `overdue`, `never` or `unknown` (`backup_state` in sjel-status).
    pub state: String,
    pub last_success: Option<String>,
    pub age_seconds: Option<u64>,
    pub bytes: Option<u64>,
    #[serde(default)]
    pub holds_service: bool,
    #[serde(default)]
    pub advise_days: Option<i64>,
    #[serde(default)]
    pub stale_days: Option<i64>,
    /// The run this sjel-status process started, if any.
    pub run: Option<BackupRun>,
    #[serde(default)]
    pub attempt: Option<BackupRunAttemptRecord>,
}

/// `GET /api/sjel-status/backups` (`backups_handler` in `capabilities/sjel-status/src/status/backup.rs`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Backups {
    pub backups: Vec<BackupStatus>,
    #[serde(default)]
    pub attempt_error: Option<String>,
}

impl Backups {
    pub fn running(&self) -> bool {
        self.backups
            .iter()
            .any(|b| b.run.as_ref().is_some_and(|run| run.state == "running"))
    }
}

// --- Search Hit ---

/// One server search result (`capabilities/sjel-status/src/status/search.rs`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ServerHit {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// A `sjel://` deep link, opened through [`crate::route::parse_deep_link`].
    pub link: String,
}

pub type SearchHit = ServerHit;

// --- Formatting Helpers ---

/// Format bytes into human readable binary units.
pub fn format_bytes(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "—".to_string();
    };
    if bytes == 0 {
        return "0 B".to_string();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let b = bytes as f64;
    if b >= TB {
        format!("{:.1} TB", b / TB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Format seconds elapsed into concise human label.
pub fn format_age_seconds(seconds: Option<u64>) -> String {
    let Some(s) = seconds else {
        return "never".to_string();
    };
    if s < 60 {
        "just now".to_string()
    } else if s < 3_600 {
        format!("{}m ago", s / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3_600)
    } else {
        format!("{}d ago", s / 86_400)
    }
}

/// Format duration in seconds between epochs.
pub fn format_duration(started: i64, finished: Option<i64>) -> String {
    match finished {
        Some(f) if f >= started => format!("{}s", f - started),
        Some(_) => "0s".to_string(),
        None => "active".to_string(),
    }
}

/// Run attempt outcome: "ok", "running", or "failed".
pub fn run_outcome(exit_code: Option<i64>, finished_epoch: Option<i64>) -> &'static str {
    match exit_code {
        Some(0) => "ok",
        Some(_) => "failed",
        None if finished_epoch.is_none() => "running",
        None => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(None), "—");
        assert_eq!(format_bytes(Some(0)), "0 B");
        assert_eq!(format_bytes(Some(512)), "512 B");
        assert_eq!(format_bytes(Some(1024)), "1.0 KB");
        assert_eq!(format_bytes(Some(1536)), "1.5 KB");
        assert_eq!(format_bytes(Some(1048576)), "1.0 MB");
        assert_eq!(format_bytes(Some(1073741824)), "1.0 GB");
    }

    #[test]
    fn test_format_age_seconds() {
        assert_eq!(format_age_seconds(None), "never");
        assert_eq!(format_age_seconds(Some(30)), "just now");
        assert_eq!(format_age_seconds(Some(120)), "2m ago");
        assert_eq!(format_age_seconds(Some(7200)), "2h ago");
        assert_eq!(format_age_seconds(Some(172800)), "2d ago");
    }

    #[test]
    fn test_duration_and_outcome() {
        assert_eq!(format_duration(100, Some(115)), "15s");
        assert_eq!(format_duration(100, None), "active");

        assert_eq!(run_outcome(Some(0), Some(115)), "ok");
        assert_eq!(run_outcome(Some(1), Some(115)), "failed");
        assert_eq!(run_outcome(None, None), "running");
        assert_eq!(run_outcome(None, Some(115)), "failed");
    }
}

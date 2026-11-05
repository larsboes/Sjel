//! Pure wire models, backup records, agent gates, health models, and route constants for Sjel status.
//!
//! Shared across `crates/sjel-status`, `crates/backup`, and `apps/gpui/core`. Zero platform dependencies (pure `serde`).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub use setup_api::*;

// --- Route Constants ---

pub const ROUTE_STATUS_HEALTH: &str = "/api/sjel-status/health";
pub const ROUTE_STATUS_ROUTES: &str = "/api/sjel-status/routes";
pub const ROUTE_STATUS_NOW: &str = "/api/sjel-status/now";
pub const ROUTE_STATUS_SEARCH: &str = "/api/sjel-status/search";
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

// --- Macmon, Storage, Updates, Host Watch & Runtime ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct MacmonSample {
    #[serde(default)]
    pub all_power: f64,
    #[serde(default)]
    pub cpu_power: f64,
    #[serde(default)]
    pub gpu_power: f64,
    #[serde(default)]
    pub ram_power: f64,
    #[serde(default)]
    pub sys_power: f64,
    #[serde(default)]
    pub cpu_usage_pct: f64,
    #[serde(default)]
    pub pcpu_usage: [f64; 2],
    #[serde(default)]
    pub ecpu_usage: [f64; 2],
    #[serde(default)]
    pub memory: MacmonMemory,
    #[serde(default)]
    pub temp: MacmonTemperature,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct MacmonMemory {
    #[serde(default)]
    pub ram_total: f64,
    #[serde(default)]
    pub ram_usage: f64,
    #[serde(default)]
    pub swap_total: f64,
    #[serde(default)]
    pub swap_usage: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct MacmonTemperature {
    #[serde(default)]
    pub cpu_temp_avg: f64,
    #[serde(default)]
    pub gpu_temp_avg: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Temperature {
    Cool,
    Warm,
    Hot,
}

pub fn temperature(celsius: f64) -> Temperature {
    if celsius >= 80.0 {
        Temperature::Hot
    } else if celsius >= 60.0 {
        Temperature::Warm
    } else {
        Temperature::Cool
    }
}

impl MacmonSample {
    pub fn ram_ratio(&self) -> f32 {
        ratio(self.memory.ram_usage, self.memory.ram_total)
    }

    pub fn swap_ratio(&self) -> f32 {
        ratio(self.memory.swap_usage, self.memory.swap_total)
    }

    pub fn under_pressure(&self) -> bool {
        self.memory.swap_total > 0.0 && self.memory.swap_usage > self.memory.swap_total * 0.5
    }
}

fn ratio(part: f64, whole: f64) -> f32 {
    if whole <= 0.0 {
        return 0.0;
    }
    (part / whole).clamp(0.0, 1.0) as f32
}

pub fn gigabytes(bytes: f64) -> String {
    format!("{:.1} GB", bytes / 1024.0 / 1024.0 / 1024.0)
}

pub fn watts(value: f64) -> String {
    format!("{value:.2} W")
}

pub fn percent(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageReport {
    #[serde(default)]
    pub disk: Disk,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub classes: Vec<StorageClass>,
    #[serde(default)]
    pub protected: Vec<StorageProtected>,
    #[serde(default)]
    pub expected_service: Vec<ExpectedService>,
    #[serde(default)]
    pub named_bytes: Option<u64>,
    #[serde(default)]
    pub unattributed_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Disk {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub free: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageClass {
    pub name: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub applicable: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub measured: Option<bool>,
    #[serde(default)]
    pub present: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageProtected {
    pub path: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub present: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedService {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub presence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    #[default]
    Ok,
    Warn,
    Critical,
}

pub fn severity(state: &str) -> Severity {
    match state {
        "critical" => Severity::Critical,
        "warn" => Severity::Warn,
        _ => Severity::Ok,
    }
}

pub fn storage_bytes(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StorageView {
    pub used_pct: f32,
    pub used: String,
    pub total: String,
    pub free: String,
    pub severity: Severity,
    pub state_label: String,
    pub classes: Vec<StorageClass>,
    pub scale_bytes: u64,
    pub reclaimable_label: String,
    pub protected: Vec<StorageProtected>,
    pub protected_absent: usize,
    pub protected_absent_paths: Vec<String>,
    pub protected_unreadable: usize,
    pub unmeasured: Vec<StorageClass>,
    pub classes_absent: Vec<StorageClass>,
    pub services_absent: Vec<String>,
    pub reconciliation: Option<(String, String)>,
}

impl StorageReport {
    pub fn view(&self) -> StorageView {
        let sized: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| class.bytes > 0)
            .cloned()
            .collect();
        let mut classes = sized;
        classes.sort_by_key(|class| std::cmp::Reverse(class.bytes));

        let mut classes_absent: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| class.present == Some(false))
            .cloned()
            .collect();
        classes_absent.sort_by(|a, b| a.name.cmp(&b.name));

        let mut unmeasured: Vec<StorageClass> = self
            .classes
            .iter()
            .filter(|class| {
                class.present != Some(false) && class.measured == Some(false) && class.bytes == 0
            })
            .cloned()
            .collect();
        unmeasured.sort_by(|a, b| a.name.cmp(&b.name));

        let mut protected = self.protected.clone();
        protected.sort_by_key(|entry| std::cmp::Reverse(entry.bytes));

        let reclaimable: u64 = classes
            .iter()
            .filter(|class| class.applicable)
            .map(|class| class.bytes)
            .sum();

        let scale_bytes = classes
            .iter()
            .filter(|class| class.measured != Some(false))
            .map(|class| class.bytes)
            .max()
            .or_else(|| classes.first().map(|class| class.bytes))
            .unwrap_or(0)
            .max(1);

        let mut services_absent: Vec<String> = self
            .expected_service
            .iter()
            .filter(|service| service.presence.as_deref() == Some("absent"))
            .map(|service| service.name.clone())
            .collect();
        services_absent.sort();

        StorageView {
            used_pct: if self.disk.total > 0 {
                (self.disk.used as f64 / self.disk.total as f64) as f32
            } else {
                0.0
            },
            used: storage_bytes(self.disk.used),
            total: storage_bytes(self.disk.total),
            free: storage_bytes(self.disk.free),
            severity: severity(&self.state),
            state_label: self.state.clone(),
            classes,
            scale_bytes,
            reclaimable_label: storage_bytes(reclaimable),
            protected_absent: protected
                .iter()
                .filter(|entry| entry.present == Some(false))
                .count(),
            protected_absent_paths: protected
                .iter()
                .filter(|entry| entry.present == Some(false))
                .map(|entry| entry.path.clone())
                .collect(),
            protected_unreadable: protected
                .iter()
                .filter(|entry| entry.bytes == 0 && entry.present != Some(false))
                .count(),
            protected,
            unmeasured,
            classes_absent,
            services_absent,
            reconciliation: match (self.named_bytes, self.unattributed_bytes) {
                (Some(named), Some(unattributed)) => {
                    Some((storage_bytes(named), storage_bytes(unattributed)))
                }
                _ => None,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageHistory {
    #[serde(default)]
    pub retention_days: u32,
    #[serde(default)]
    pub samples: Vec<StorageSample>,
    #[serde(default)]
    pub absent: Vec<StorageAbsence>,
    #[serde(default)]
    pub classes: Vec<StorageClassSample>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageSample {
    pub hour: String,
    #[serde(default)]
    pub used: i64,
    #[serde(default)]
    pub unattributed: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageAbsence {
    pub kind: String,
    pub name: String,
    pub since: String,
    #[serde(default)]
    pub capped: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageClassSample {
    pub name: String,
    #[serde(default)]
    pub bytes: i64,
}

pub const TREND_WINDOW_HOURS: usize = 168;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StorageTrend {
    pub enough: bool,
    pub delta_label: String,
    pub unattributed_delta_label: String,
    pub span_label: String,
    pub bars: Vec<f32>,
}

impl StorageHistory {
    pub fn trend(&self) -> Option<StorageTrend> {
        let window = self.samples.len().min(TREND_WINDOW_HOURS);
        let samples = self
            .samples
            .get(self.samples.len().saturating_sub(window)..)?;
        let first = samples.first()?;
        let last = samples.last()?;
        let enough = samples.len() >= 2;
        let values: Vec<i64> = samples.iter().map(|sample| sample.used).collect();
        Some(StorageTrend {
            enough,
            delta_label: signed_storage_bytes(last.used - first.used),
            unattributed_delta_label: signed_storage_bytes(last.unattributed - first.unattributed),
            span_label: span_label(hours_between(&first.hour, &last.hour)),
            bars: bars(&values),
        })
    }

    pub fn class_deltas(&self) -> BTreeMap<String, String> {
        let mut by_name: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for entry in &self.classes {
            by_name
                .entry(entry.name.clone())
                .or_default()
                .push(entry.bytes);
        }
        by_name
            .into_iter()
            .filter_map(|(name, values)| {
                if values.len() < 2 {
                    return None;
                }
                let window = values.len().min(TREND_WINDOW_HOURS);
                let windowed = &values[values.len() - window..];
                let delta = windowed.last()? - windowed.first()?;
                Some((name, signed_storage_bytes(delta)))
            })
            .collect()
    }
}

fn bars(values: &[i64]) -> Vec<f32> {
    let min = values.iter().copied().min().unwrap_or(0);
    let max = values.iter().copied().max().unwrap_or(0);
    let range = max - min;
    values
        .iter()
        .map(|value| match range {
            0 => 0.5,
            range => {
                let share = (*value - min) as f32 / range as f32;
                0.25 + share * 0.75
            }
        })
        .collect()
}

fn parse_stamp_epoch_hours(stamp: &str) -> Option<i64> {
    let day = civil_date::unix_day_of_iso(stamp)?;
    let time_part = stamp.get(11..)?;
    let hour: i64 = time_part.get(..2)?.parse().ok()?;
    let min: i64 = time_part.get(3..5)?.parse().ok()?;
    let mut total_mins = day * 24 * 60 + hour * 60 + min;
    if let Some(pos) = time_part.rfind(['+', '-']) {
        if pos >= 5 {
            let sign = if time_part.as_bytes()[pos] == b'+' {
                -1
            } else {
                1
            };
            let tz = &time_part[pos + 1..];
            if let (Some(th), Some(tm)) = (
                tz.get(..2).and_then(|s| s.parse::<i64>().ok()),
                tz.get(3..5).and_then(|s| s.parse::<i64>().ok()),
            ) {
                total_mins += sign * (th * 60 + tm);
            }
        }
    }
    Some(total_mins / 60)
}

fn hours_between(a: &str, b: &str) -> i64 {
    match (parse_stamp_epoch_hours(a), parse_stamp_epoch_hours(b)) {
        (Some(from), Some(to)) => (to - from).max(0),
        _ => 0,
    }
}

fn span_label(hours: i64) -> String {
    if hours < 24 {
        format!("{hours}h")
    } else {
        format!("{}d", hours / 24)
    }
}

pub fn signed_storage_bytes(delta: i64) -> String {
    let size = storage_bytes(delta.unsigned_abs());
    if size == "0 MB" {
        return "±0 MB".to_owned();
    }
    format!("{}{size}", if delta > 0 { "+" } else { "-" })
}

pub fn day_label(stamp: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let bytes = stamp.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return stamp.to_owned();
    }
    let (Ok(month), Ok(day)) = (stamp[5..7].parse::<usize>(), stamp[8..10].parse::<usize>()) else {
        return stamp.to_owned();
    };
    match MONTHS.get(month.wrapping_sub(1)) {
        Some(name) if (1..=31).contains(&day) => format!("{day} {name}"),
        _ => stamp.to_owned(),
    }
}

pub fn dated_absences(
    names: &[String],
    kind: &str,
    history: Option<&StorageHistory>,
) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            let found = history.and_then(|history| {
                history
                    .absent
                    .iter()
                    .find(|entry| entry.kind == kind && &entry.name == name)
            });
            match found {
                None => name.clone(),
                Some(row) => format!(
                    "{name} (absent since {}{})",
                    day_label(&row.since),
                    if row.capped { " or earlier" } else { "" }
                ),
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdatesReport {
    #[serde(default)]
    pub offline: bool,
    #[serde(rename = "lastApply", default)]
    pub last_apply: Option<ApplyReceipt>,
    #[serde(default)]
    pub surfaces: Vec<UpdateSurface>,
    #[serde(default)]
    pub rows: Vec<UpdateRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Owner {
    Scheduled,
    Manual,
    Unowned,
    SelfManaged,
}

impl Owner {
    pub const ORDER: [Self; 4] = [
        Self::Scheduled,
        Self::Manual,
        Self::Unowned,
        Self::SelfManaged,
    ];

    pub fn parse(value: &str) -> Self {
        match value {
            "scheduled" => Self::Scheduled,
            "manual" => Self::Manual,
            "self" => Self::SelfManaged,
            _ => Self::Unowned,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdateSurface {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub owner: String,
    #[serde(rename = "ownerDetail", default)]
    pub owner_detail: String,
    #[serde(default)]
    pub actionable: bool,
    #[serde(default)]
    pub why: String,
}

impl UpdateSurface {
    pub fn owner(&self) -> Owner {
        Owner::parse(&self.owner)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateStatus {
    Current,
    Stale,
    Unknown,
    NotApplicable,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdateRow {
    pub surface: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub installed: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl UpdateRow {
    pub fn status(&self) -> UpdateStatus {
        match self.status.as_str() {
            "current" => UpdateStatus::Current,
            "stale" => UpdateStatus::Stale,
            "n/a" => UpdateStatus::NotApplicable,
            _ => UpdateStatus::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApplyReceipt {
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub steps: Option<u64>,
    #[serde(default)]
    pub failed: Option<u64>,
    #[serde(rename = "stillStale", default)]
    pub still_stale: Option<u64>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub audit: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateGroup {
    pub surface: UpdateSurface,
    pub rows: Vec<UpdateRow>,
    pub actionable: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdatesView {
    pub groups: Vec<UpdateGroup>,
    pub stale: usize,
    pub unknown: usize,
    pub last_apply: Option<ApplyReceipt>,
    pub busy: bool,
}

impl UpdatesReport {
    pub fn view(&self) -> UpdatesView {
        let mut groups = Vec::new();
        for owner in Owner::ORDER {
            for surface in self
                .surfaces
                .iter()
                .filter(|surface| surface.owner() == owner)
            {
                let rows: Vec<UpdateRow> = self
                    .rows
                    .iter()
                    .filter(|row| row.surface == surface.id)
                    .cloned()
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let actionable = if surface.actionable {
                    rows.iter()
                        .filter(|row| row.status() == UpdateStatus::Stale && row.action.is_some())
                        .count()
                } else {
                    0
                };
                groups.push(UpdateGroup {
                    surface: surface.clone(),
                    rows,
                    actionable,
                });
            }
        }

        let stale = self
            .rows
            .iter()
            .filter(|row| row.status() == UpdateStatus::Stale)
            .count();
        let unknown = self
            .rows
            .iter()
            .filter(|row| row.status() == UpdateStatus::Unknown)
            .count();
        let busy = self
            .last_apply
            .as_ref()
            .and_then(|receipt| receipt.state.as_deref())
            == Some("running");
        UpdatesView {
            groups,
            stale,
            unknown,
            last_apply: self.last_apply.clone(),
            busy,
        }
    }
}

pub fn version_label(row: &UpdateRow) -> String {
    let Some(installed) = row.installed.as_deref() else {
        return row.latest.clone().unwrap_or_default();
    };
    match row.latest.as_deref() {
        Some(latest) if latest != installed => format!("{installed} → {latest}"),
        _ => installed.to_owned(),
    }
}

pub fn audit_label(verdict: Option<&str>) -> String {
    match verdict {
        None => String::new(),
        Some("clean") => " · audit clean".to_owned(),
        Some(verdict) => format!(" · audit {verdict} — run tools/audit"),
    }
}

pub fn audit_needs_attention(verdict: Option<&str>) -> bool {
    matches!(verdict, Some(verdict) if verdict != "clean")
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Findings {
    #[serde(default)]
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub first_seen: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeStatus {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub configured: bool,
    #[serde(default = "auto_selection")]
    pub selection: String,
    #[serde(default = "normal_selection")]
    pub effective: String,
    #[serde(default = "unknown_power")]
    pub power: String,
    #[serde(default)]
    pub power_fresh: bool,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub detail: Option<String>,
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            device: String::new(),
            configured: false,
            selection: auto_selection(),
            effective: normal_selection(),
            power: unknown_power(),
            power_fresh: false,
            allow: Vec::new(),
            revision: 0,
            detail: None,
        }
    }
}

fn auto_selection() -> String {
    "auto".to_owned()
}

fn normal_selection() -> String {
    "normal".to_owned()
}

fn unknown_power() -> String {
    "unknown".to_owned()
}

pub const RUNTIME_SELECTIONS: [(&str, &str); 3] = [
    ("auto", "Auto"),
    ("normal", "Normal"),
    ("on-the-go", "On the go"),
];

pub const RUNTIME_CATEGORIES: [(&str, &str); 5] = [
    ("other-local-models", "Other local models"),
    ("remote-models", "Remote models"),
    ("bulk-indexing", "Bulk indexing"),
    ("transcription", "Transcription"),
    ("media-conversion", "Media conversion"),
];

pub fn selection_label(selection: &str) -> &'static str {
    RUNTIME_SELECTIONS
        .iter()
        .find(|(id, _)| *id == selection)
        .map_or("Auto", |(_, label)| label)
}

pub fn category_label(category: &str) -> &'static str {
    RUNTIME_CATEGORIES
        .iter()
        .find(|(id, _)| *id == category)
        .map_or("Unknown category", |(_, label)| label)
}

pub fn power_label(power: &str) -> &'static str {
    match power {
        "ac" => "AC",
        "battery" => "Battery",
        "no-battery" => "No battery",
        _ => "Power unknown",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeView {
    pub label: &'static str,
    pub source: String,
    pub on_the_go: bool,
    pub afm_only: bool,
    pub exceptions: Vec<&'static str>,
}

impl RuntimeView {
    pub fn summary(&self) -> String {
        let exceptions = if self.exceptions.is_empty() {
            "No active exceptions".to_owned()
        } else {
            format!("Allowed: {}", self.exceptions.join(", "))
        };
        let afm = if self.afm_only { " · AFM only" } else { "" };
        format!("{} · {}{afm} · {exceptions}", self.label, self.source)
    }
}

impl RuntimeStatus {
    pub fn view(&self) -> RuntimeView {
        let on_the_go = self.effective == "on-the-go";
        let exceptions: Vec<&'static str> = if on_the_go {
            self.allow
                .iter()
                .map(|category| category_label(category))
                .collect()
        } else {
            Vec::new()
        };
        let afm_only = on_the_go
            && !self.allow.iter().any(|category| {
                matches!(category.as_str(), "other-local-models" | "remote-models")
            });
        let source = if !self.configured {
            "Not configured".to_owned()
        } else if self.selection == "auto" {
            format!(
                "Auto · {}{}",
                power_label(&self.power),
                if self.power_fresh { "" } else { " (stale)" }
            )
        } else {
            "Manual".to_owned()
        };
        RuntimeView {
            label: selection_label(&self.effective),
            source,
            on_the_go,
            afm_only,
            exceptions,
        }
    }
}

// --- CV & Career Applications ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvLogEntry {
    pub date: String,
    pub status: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvApplication {
    pub company: String,
    pub role: String,
    #[serde(default)]
    pub url: Option<String>,
    pub profile: String,
    pub lang: String,
    pub status: String,
    #[serde(default)]
    pub log: Vec<CvLogEntry>,
    #[serde(default)]
    pub notes: Option<String>,
    pub slug: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvPdf {
    pub name: String,
    pub path: String,
    pub mtime: u64,
    pub stale: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvView {
    pub master: String,
    pub master_mtime: u64,
    pub profiles: Vec<String>,
    pub statuses: Vec<String>,
    pub applications: Vec<CvApplication>,
    pub pdfs: Vec<CvPdf>,
    #[serde(default)]
    pub linkedin_pulled: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvWriteRequest {
    pub action: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvWriteResult {
    pub action: String,
    pub detail: String,
    #[serde(default)]
    pub warnings: String,
}

pub fn slug_for(company: &str, role: &str) -> String {
    let raw = format!("{company}-{role}").to_lowercase();
    let mut result = String::new();
    let mut prev_dash = false;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            prev_dash = false;
        } else if !prev_dash && !result.is_empty() {
            result.push('-');
            prev_dash = true;
        }
    }

    let trimmed = result.trim_end_matches('-');
    trimmed.chars().take(48).collect()
}

pub fn last_status_date(app: &CvApplication) -> &str {
    app.log.last().map(|l| l.date.as_str()).unwrap_or("")
}

pub fn pdf_for_profile<'a>(view: &'a CvView, profile: &str, lang: &str) -> Option<&'a CvPdf> {
    let target_name = format!("CV_{profile}_{lang}.pdf");
    view.pdfs.iter().find(|pdf| pdf.name == target_name)
}

pub fn count_stale_pdfs(view: &CvView) -> usize {
    view.pdfs.iter().filter(|p| p.stale).count()
}

pub fn filter_applications<'a>(view: &'a CvView, status: &str) -> Vec<&'a CvApplication> {
    view.applications
        .iter()
        .filter(|app| app.status == status)
        .collect()
}

// --- Packs & Harnesses ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackUnitView {
    pub pack: String,
    pub skill: String,
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackStrayView {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessView {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub cli: String,
    #[serde(default)]
    pub units: Vec<PackUnitView>,
    #[serde(default)]
    pub unowned: Vec<PackStrayView>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedView {
    pub id: String,
    pub label: String,
    pub why: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacksView {
    #[serde(default)]
    pub measured_at: String,
    #[serde(default)]
    pub harnesses: Vec<HarnessView>,
    #[serde(default)]
    pub unsupported: Vec<UnsupportedView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackWriteRequest {
    pub action: String,
    pub harness: String,
    pub packs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PackWriteResult {
    pub action: String,
    pub harness: String,
    pub packs: Vec<String>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatrixRow {
    pub pack: String,
    pub skill: String,
}

pub fn matrix_rows(view: &PacksView) -> Vec<MatrixRow> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();

    for harness in view.harnesses.iter().filter(|h| h.installed) {
        for unit in &harness.units {
            let key = (unit.pack.clone(), unit.skill.clone());
            if seen.insert(key.clone()) {
                rows.push(MatrixRow {
                    pack: key.0,
                    skill: key.1,
                });
            }
        }
    }
    rows
}

pub fn all_packs(rows: &[MatrixRow]) -> Vec<String> {
    let mut packs = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        if seen.insert(row.pack.clone()) {
            packs.push(row.pack.clone());
        }
    }
    packs
}

pub fn status_of<'a>(harness: &'a HarnessView, pack: &str, skill: &str) -> &'a str {
    harness
        .units
        .iter()
        .find(|u| u.pack == pack && u.skill == skill)
        .map(|u| u.status.as_str())
        .unwrap_or("not-deployed")
}

pub fn needs_attention(status: &str) -> bool {
    status != "current" && status != "not-deployed"
}

pub fn row_needs_attention(view: &PacksView, row: &MatrixRow) -> bool {
    view.harnesses
        .iter()
        .filter(|h| h.installed)
        .any(|h| needs_attention(status_of(h, &row.pack, &row.skill)))
}

pub fn deployed_units(harness: &HarnessView) -> Vec<&PackUnitView> {
    harness
        .units
        .iter()
        .filter(|u| u.status != "not-deployed")
        .collect()
}

pub fn held_packs(harness: &HarnessView) -> Vec<String> {
    let mut packs = Vec::new();
    let mut seen = HashSet::new();
    for u in deployed_units(harness) {
        if seen.insert(u.pack.clone()) {
            packs.push(u.pack.clone());
        }
    }
    packs
}

pub fn installed_harnesses(view: &PacksView) -> Vec<&HarnessView> {
    view.harnesses.iter().filter(|h| h.installed).collect()
}

pub fn orphaned_harnesses(view: &PacksView) -> Vec<&HarnessView> {
    view.harnesses
        .iter()
        .filter(|h| !h.installed && !deployed_units(h).is_empty())
        .collect()
}

// --- Research & Benchmarks ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpstreamRow {
    pub name: String,
    pub url: String,
    pub verdict: String,
    pub license: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SystemRow {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    pub kind: String,
    pub local: bool,
    pub why: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectGroup {
    pub id: String,
    pub label: String,
    pub blurb: String,
    pub rows: Vec<UpstreamRow>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkCase {
    pub id: String,
    #[serde(default)]
    pub expected: serde_json::Value,
    #[serde(default)]
    pub got: serde_json::Value,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub ms: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkLatency {
    pub p50: f64,
    pub p95: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkResult {
    pub suite: String,
    pub date: String,
    pub backend: String,
    pub model: String,
    pub host: String,
    pub n: i64,
    pub correct: i64,
    pub accuracy: f64,
    pub latency_ms: BenchmarkLatency,
    #[serde(default)]
    pub cases: Vec<BenchmarkCase>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkRun {
    pub suite: String,
    pub file: String,
    pub result: BenchmarkResult,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkSuite {
    pub name: String,
    pub runs: Vec<BenchmarkRun>,
    pub case_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResearchArticle {
    pub slug: String,
    pub file: String,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ResearchView {
    pub articles: Vec<ResearchArticle>,
    pub upstreams: Vec<UpstreamRow>,
    pub systems: Vec<SystemRow>,
    pub suites: Vec<BenchmarkSuite>,
}

pub fn parse_single_line_toml(content: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_map: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(name) = current_name.take() {
                sections.push((name, std::mem::take(&mut current_map)));
            }
            let name = trimmed[1..trimmed.len() - 1].trim().to_string();
            current_name = Some(name);
            continue;
        }

        if let Some((key, val_raw)) = trimmed.split_once('=') {
            let key = key.trim().to_string();
            let val = val_raw.trim();
            let val_unquoted = if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                if val.len() >= 2 {
                    &val[1..val.len() - 1]
                } else {
                    val
                }
            } else {
                val
            };
            current_map.insert(key, val_unquoted.to_string());
        }
    }

    if let Some(name) = current_name {
        sections.push((name, current_map));
    }

    sections
}

pub fn parse_upstreams_toml(content: &str) -> Vec<UpstreamRow> {
    parse_single_line_toml(content)
        .into_iter()
        .map(|(name, map)| UpstreamRow {
            name,
            url: map.get("url").cloned().unwrap_or_default(),
            verdict: map.get("verdict").cloned().unwrap_or_default(),
            license: map.get("license").cloned().unwrap_or_default(),
            summary: map.get("summary").cloned().unwrap_or_default(),
        })
        .collect()
}

pub fn parse_systems_toml(content: &str) -> Vec<SystemRow> {
    parse_single_line_toml(content)
        .into_iter()
        .map(|(name, map)| {
            let raw_url = map.get("url").cloned().unwrap_or_default();
            let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
                Some(raw_url)
            } else {
                None
            };
            let local = map.get("local").map(|v| v == "yes").unwrap_or(false);
            SystemRow {
                name,
                url,
                kind: map.get("kind").cloned().unwrap_or_default(),
                local,
                why: map.get("why").cloned().unwrap_or_default(),
            }
        })
        .collect()
}

pub fn to_article(file: &str, markdown: &str) -> ResearchArticle {
    let slug = if file == "README.md" {
        "index".to_string()
    } else {
        file.strip_suffix(".md").unwrap_or(file).to_string()
    };

    let mut title = file.to_string();
    let mut body_lines = Vec::new();
    let mut found_title = false;

    for line in markdown.lines() {
        if !found_title && line.starts_with("# ") {
            title = line[2..].trim().to_string();
            found_title = true;
        } else {
            body_lines.push(line);
        }
    }

    ResearchArticle {
        slug,
        file: file.to_string(),
        title,
        body: body_lines.join("\n").trim().to_string(),
    }
}

pub fn group_upstreams(rows: &[UpstreamRow]) -> Vec<ProjectGroup> {
    let mut built_on = Vec::new();
    let mut learned_from = Vec::new();
    let mut watching = Vec::new();
    let mut declined = Vec::new();
    let mut other = Vec::new();

    for row in rows {
        match row.verdict.as_str() {
            "adopt" | "adapt" | "contribute" | "overlay" | "fork" | "build" => {
                built_on.push(row.clone());
            }
            "inspiration" | "quarry" => {
                learned_from.push(row.clone());
            }
            "watch" => {
                watching.push(row.clone());
            }
            "reject" => {
                declined.push(row.clone());
            }
            _ => {
                other.push(row.clone());
            }
        }
    }

    let sort_group = |items: &mut Vec<UpstreamRow>| {
        items.sort_by(|a, b| a.name.cmp(&b.name));
    };
    sort_group(&mut built_on);
    sort_group(&mut learned_from);
    sort_group(&mut watching);
    sort_group(&mut declined);
    sort_group(&mut other);

    let mut groups = vec![
        ProjectGroup {
            id: "built-on".to_string(),
            label: "Built on".to_string(),
            blurb: "Code Sjel uses, contributes to, or carries a reviewed delta for.".to_string(),
            rows: built_on,
        },
        ProjectGroup {
            id: "learned-from".to_string(),
            label: "Learned from".to_string(),
            blurb: "Projects whose ideas or parts Sjel took, without depending on their code."
                .to_string(),
            rows: learned_from,
        },
        ProjectGroup {
            id: "watching".to_string(),
            label: "Watching".to_string(),
            blurb:
                "Noted and not yet read. A watch row grants nothing until an audit gives it a verdict."
                    .to_string(),
            rows: watching,
        },
        ProjectGroup {
            id: "declined".to_string(),
            label: "Declined".to_string(),
            blurb: "Read and declined, with the reason kept so nobody has to read them again."
                .to_string(),
            rows: declined,
        },
    ];

    if !other.is_empty() {
        groups.push(ProjectGroup {
            id: "other".to_string(),
            label: "Other verdicts".to_string(),
            blurb: "Projects with unique verdicts.".to_string(),
            rows: other,
        });
    }

    groups
}

pub fn group_suites(runs: &[BenchmarkRun]) -> Vec<BenchmarkSuite> {
    let mut by_suite: BTreeMap<String, Vec<BenchmarkRun>> = BTreeMap::new();
    for run in runs {
        by_suite
            .entry(run.suite.clone())
            .or_default()
            .push(run.clone());
    }

    let mut suites = Vec::new();
    for (name, mut suite_runs) in by_suite {
        suite_runs.sort_by(|a, b| {
            b.result
                .accuracy
                .partial_cmp(&a.result.accuracy)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.result
                        .latency_ms
                        .p50
                        .partial_cmp(&b.result.latency_ms.p50)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut case_ids = Vec::new();
        for r in &suite_runs {
            for c in &r.result.cases {
                if !case_ids.contains(&c.id) {
                    case_ids.push(c.id.clone());
                }
            }
        }

        suites.push(BenchmarkSuite {
            name,
            runs: suite_runs,
            case_ids,
        });
    }

    suites
}

// --- Self-Model ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfCode {
    #[serde(default)]
    pub files: usize,
    #[serde(default)]
    pub nodes: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfService {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfUnit {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub service: Option<SelfService>,
    #[serde(default)]
    pub code: Option<SelfCode>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfCoupling {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfUpstream {
    pub name: String,
    pub verdict: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfModel {
    #[serde(default)]
    pub schema: usize,
    #[serde(default)]
    pub generator: String,
    #[serde(default)]
    pub units: Vec<SelfUnit>,
    #[serde(default)]
    pub coupling: Vec<SelfCoupling>,
    #[serde(default)]
    pub upstreams: Vec<SelfUpstream>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SelfModelResponse {
    pub model: SelfModel,
    #[serde(default)]
    pub live: BTreeMap<String, Option<bool>>,
}

pub fn coupled_names(model: &SelfModel) -> HashSet<String> {
    let mut names = HashSet::new();
    for c in &model.coupling {
        names.insert(c.from.clone());
        names.insert(c.to.clone());
    }
    names
}

pub fn isolated_units<'a>(model: &'a SelfModel) -> Vec<&'a SelfUnit> {
    let coupled = coupled_names(model);
    model
        .units
        .iter()
        .filter(|u| !coupled.contains(&u.name))
        .collect()
}

pub fn coupled_units<'a>(model: &'a SelfModel) -> Vec<&'a SelfUnit> {
    let coupled = coupled_names(model);
    model
        .units
        .iter()
        .filter(|u| coupled.contains(&u.name))
        .collect()
}

pub fn units_by_kind<'a>(model: &'a SelfModel, kind: &str) -> Vec<&'a SelfUnit> {
    model.units.iter().filter(|u| u.kind == kind).collect()
}

pub fn outgoing_couplings<'a>(model: &'a SelfModel, unit: &str) -> Vec<&'a SelfCoupling> {
    model.coupling.iter().filter(|c| c.from == unit).collect()
}

pub fn incoming_couplings<'a>(model: &'a SelfModel, unit: &str) -> Vec<&'a SelfCoupling> {
    model.coupling.iter().filter(|c| c.to == unit).collect()
}

pub fn unit_degree(model: &SelfModel, unit: &str) -> usize {
    model
        .coupling
        .iter()
        .filter(|c| c.from == unit || c.to == unit)
        .count()
}

pub fn unit_liveness(res: &SelfModelResponse, unit: &str) -> Option<bool> {
    res.live.get(unit).copied().flatten()
}

// --- Credentials ---

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CredentialRow {
    pub capability: String,
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub env_fallback: Option<String>,
    #[serde(default)]
    pub in_keychain: bool,
    #[serde(default)]
    pub env_file: bool,
    #[serde(default)]
    pub error: Option<String>,
}

impl CredentialRow {
    pub fn state(&self) -> &'static str {
        match (self.in_keychain, self.env_file) {
            (true, _) => "In the keychain",
            (false, true) => "Old env file only — connect to move it to the keychain",
            (false, false) => "Not connected",
        }
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

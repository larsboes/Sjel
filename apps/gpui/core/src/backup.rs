//! Backup data structures, targets, rehearsal verification, and run history.

use serde::{Deserialize, Serialize};

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

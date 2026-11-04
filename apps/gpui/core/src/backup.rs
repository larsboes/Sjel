//! Backup data structures, targets, rehearsal verification, and run history.

pub use status_api::{
    format_age_seconds, format_bytes, format_duration, run_outcome, ArchiveIdentity,
    BackupRunAttemptRecord, BackupRunsResponse, BackupTargetRecord, BackupTargetsResponse,
    VerifyTargetResult,
};

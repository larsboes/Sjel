use std::path::PathBuf;

pub const RUNNING_COMMIT: &str = env!("SJEL_BUILD_COMMIT");
pub const BUILD_TIME: &str = env!("SJEL_BUILD_TIME");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSummary {
    pub sha: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub running_commit: &'static str,
    pub build_time: &'static str,
    pub latest_commit: String,
    pub commits_behind: usize,
    pub commits: Vec<CommitSummary>,
    pub repo_root: PathBuf,
    pub has_uncommitted_changes: bool,
}

impl UpdateInfo {
    /// True when the checkout has commits this build lacks. Uncommitted changes do not count:
    /// agents edit this checkout all day, so they would keep the pill lit for nothing.
    pub fn is_update_available(&self) -> bool {
        self.commits_behind > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebuildState {
    Idle,
    Building,
    Success(String),
    Failed(BuildFailure),
}

/// One compiler error as `cargo build --message-format=json` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// `E0624`, or empty when rustc named no code.
    pub code: String,
    pub file: String,
    pub line: u32,
    /// The message itself, without the rendered block around it.
    pub message: String,
}

impl Diagnostic {
    /// `E0624 · ui/src/inventory.rs:366`, or the file alone when there is no code.
    pub fn location(&self) -> String {
        match (self.code.is_empty(), self.line) {
            (false, _) => format!("{} · {}:{}", self.code, self.file, self.line),
            (true, 0) => self.file.clone(),
            (true, _) => format!("{}:{}", self.file, self.line),
        }
    }
}

/// Why a rebuild failed: the first error cargo reported, and the whole output for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFailure {
    pub first: Option<Diagnostic>,
    pub log: String,
}

impl BuildFailure {
    /// A failure that never reached cargo: the script or the process itself refused.
    pub fn without_diagnostics(log: impl Into<String>) -> Self {
        Self {
            first: None,
            log: log.into(),
        }
    }

    /// The one line a panel shows before the log.
    pub fn headline(&self) -> String {
        match &self.first {
            Some(diagnostic) => format!("Rebuild failed — {}", diagnostic.location()),
            None => "Rebuild failed".to_owned(),
        }
    }
}

/// What one run of cargo's JSON output holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BuildOutput {
    pub errors: Vec<Diagnostic>,
    /// The records rendered back to text, so the log a person reads is a compiler log and not
    /// JSON. Lines cargo or the script did not wrap in a record stay as they are.
    pub log: String,
}

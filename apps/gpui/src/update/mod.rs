//! In-app update detection and rebuild triggering for Sjel Desktop.
//!
//! Compares the running binary's build commit against the local Sjel repository HEAD,
//! fetches recent commits, and can execute `apps/gpui/install` in the background to rebuild
//! and install the updated release bundle into `~/Applications/Sjel Desktop.app`.

pub mod diagnostics;
pub mod git;
pub mod pill;
pub mod rebuild;
pub mod types;

pub use diagnostics::read_cargo_output;
pub use git::{check_git_updates, find_repo_root};
pub use pill::header_pill;
pub use rebuild::{execute_rebuild, launch_other_node, restart_local_shell};
pub use types::{
    BuildFailure, BuildOutput, CommitSummary, Diagnostic, RebuildState, UpdateInfo, BUILD_TIME,
    RUNNING_COMMIT,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_update_available_detection() {
        let same = UpdateInfo {
            running_commit: "abc1234",
            build_time: "2026-10-22",
            latest_commit: "abc1234".to_string(),
            commits_behind: 0,
            commits: vec![],
            repo_root: PathBuf::from("/tmp"),
            has_uncommitted_changes: false,
        };
        assert!(!same.is_update_available());

        let behind = UpdateInfo {
            running_commit: "abc1234",
            build_time: "2026-10-22",
            latest_commit: "def5678".to_string(),
            commits_behind: 3,
            commits: vec![CommitSummary {
                sha: "def5678".into(),
                message: "latest feature".into(),
            }],
            repo_root: PathBuf::from("/tmp"),
            has_uncommitted_changes: false,
        };
        assert!(behind.is_update_available());

        let uncommitted = UpdateInfo {
            running_commit: "abc1234",
            build_time: "2026-10-22",
            latest_commit: "abc1234".to_string(),
            commits_behind: 0,
            commits: vec![],
            repo_root: PathBuf::from("/tmp"),
            has_uncommitted_changes: true,
        };
        assert!(!uncommitted.is_update_available());
    }

    #[test]
    fn test_find_repo_root() {
        let root = find_repo_root();
        assert!(root.is_some());
        let root = root.unwrap();
        assert!(root.join("apps/gpui").exists());
    }

    #[test]
    fn test_check_git_updates_in_repo() {
        let root = find_repo_root().expect("repo root found");
        let info = check_git_updates(&root);
        assert!(info.is_some());
        let info = info.unwrap();
        assert!(!info.latest_commit.is_empty());
        assert_eq!(info.repo_root, root);
    }

    #[test]
    fn test_a_compiler_error_becomes_a_location_a_panel_can_show() {
        // Records as rustc emits them, with the explanation dropped and the script's own lines
        // between them: stdout carries both.
        let stdout = concat!(
            r#"{"reason":"compiler-message","message":{"level":"error","message":"cannot find value `y` in this scope","code":{"code":"E0425"},"rendered":"error[E0425]: cannot find value `y` in this scope\n --> src/main.rs:1:45","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":45,"is_primary":true}]}}"#,
            "\n",
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused import","code":null,"rendered":"warning: unused import","spans":[]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":false}"#,
            "\n",
            "apps/gpui/install: signing",
            "\n",
            r#"{"reason":"compiler-message","message":{"level":"error","message":"no method named `missing` found","code":{"code":"E0599"},"rendered":"error[E0599]: no method named `missing` found","spans":[{"file_name":"ui/src/inventory.rs","line_start":360,"is_primary":false},{"file_name":"ui/src/inventory.rs","line_start":366,"is_primary":true}]}}"#,
        );

        let read = read_cargo_output(stdout);
        assert_eq!(
            read.errors.len(),
            2,
            "warnings, finished records and echoes are not errors"
        );
        assert_eq!(read.errors[0].location(), "E0425 · src/main.rs:1");
        assert_eq!(
            read.errors[0].message,
            "cannot find value `y` in this scope"
        );
        // The primary span wins over the first one: that is the line the compiler points at.
        assert_eq!(read.errors[1].location(), "E0599 · ui/src/inventory.rs:366");

        // The log a person opens is the rendering, not the JSON, and what was never a record stays.
        assert!(read.log.contains("error[E0425]: cannot find value"));
        assert!(read.log.contains("apps/gpui/install: signing"));
        assert!(!read.log.contains("\"reason\""));
    }

    #[test]
    fn test_a_failure_without_diagnostics_still_names_the_cause() {
        let refused = BuildFailure::without_diagnostics("no code-signing identity in the keychain");
        assert_eq!(refused.headline(), "Rebuild failed");
        assert_eq!(refused.first, None);
        assert_eq!(refused.log, "no code-signing identity in the keychain");

        let uncoded = BuildFailure {
            first: Some(Diagnostic {
                code: String::new(),
                file: "apps/gpui/install".into(),
                line: 0,
                message: "the script refused".into(),
            }),
            log: String::new(),
        };
        assert_eq!(uncoded.headline(), "Rebuild failed — apps/gpui/install");
    }
}

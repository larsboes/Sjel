//! In-app update detection and rebuild triggering for Sjel Desktop.
//!
//! Compares the running binary's build commit against the local Sjel repository HEAD,
//! fetches recent commits, and can execute `apps/gpui/install` in the background to rebuild
//! and install the updated release bundle into `~/Applications/Sjel Desktop.app`.

use std::path::{Path, PathBuf};
use std::process::Command;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable, IconName, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, Anchor, Entity, FontWeight};

use crate::settings::Section;
use crate::workspace::Workspace;

use crate::theme::Palette;

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

/// Reads cargo's `--message-format=json-render-diagnostics` output. It arrives on stdout together
/// with whatever else the install script prints, so a line that is not a record is kept as text.
pub fn read_cargo_output(stdout: &str) -> BuildOutput {
    let mut output = BuildOutput::default();
    for line in stdout.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            output.log.push_str(line);
            output.log.push('\n');
            continue;
        };
        if record["reason"] != "compiler-message" {
            continue;
        }
        let message = &record["message"];
        if let Some(rendered) = message["rendered"].as_str() {
            output.log.push_str(rendered);
        }
        if message["level"] != "error" {
            continue;
        }
        let span = message["spans"]
            .as_array()
            .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true))
            .or_else(|| message["spans"].as_array().and_then(|spans| spans.first()));
        let text = |value: &serde_json::Value, key: &str| {
            value[key].as_str().unwrap_or_default().to_owned()
        };
        output.errors.push(Diagnostic {
            code: message["code"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            file: span.map(|span| text(span, "file_name")).unwrap_or_default(),
            line: span
                .and_then(|span| span["line_start"].as_u64())
                .unwrap_or(0) as u32,
            message: text(message, "message"),
        });
    }
    output
}

/// Discovers the Sjel repository root.
pub fn find_repo_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("SJEL_ROOT") {
        let p = PathBuf::from(root);
        if p.join(".git").exists() || p.join("apps/gpui").exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            if dir.join(".git").exists() && dir.join("apps/gpui").exists() {
                return Some(dir.to_path_buf());
            }
            cur = dir.parent();
        }
    }
    // The conventional checkout, `~/Developer/Sjel`, for an app bundle launched from outside it.
    let default_repo = PathBuf::from(std::env::var_os("HOME")?).join("Developer/Sjel");
    if default_repo.join(".git").exists() {
        return Some(default_repo);
    }
    None
}

/// Checks the repository for updates.
pub fn check_git_updates(repo_root: &Path) -> Option<UpdateInfo> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let latest_commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let mut commits_behind = 0;
    let mut commits = Vec::new();

    if RUNNING_COMMIT != latest_commit {
        let rev_spec = format!("{}..HEAD", RUNNING_COMMIT);
        if let Ok(count_out) = Command::new("git")
            .args(["rev-list", "--count", &rev_spec])
            .current_dir(repo_root)
            .output()
        {
            if count_out.status.success() {
                if let Ok(count_str) = String::from_utf8(count_out.stdout) {
                    commits_behind = count_str.trim().parse::<usize>().unwrap_or(0);
                }
            }
        }

        let log_res = Command::new("git")
            .args(["log", "--oneline", "-n", "6", &rev_spec])
            .current_dir(repo_root)
            .output();

        let log_out = match log_res {
            Ok(ref o) if o.status.success() && !o.stdout.is_empty() => Some(o),
            _ => None,
        };

        let log_str = if let Some(o) = log_out {
            String::from_utf8_lossy(&o.stdout).to_string()
        } else {
            Command::new("git")
                .args(["log", "--oneline", "-n", "5", "HEAD"])
                .current_dir(repo_root)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        };

        for line in log_str.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((sha, msg)) = line.split_once(' ') {
                commits.push(CommitSummary {
                    sha: sha.to_string(),
                    message: msg.to_string(),
                });
            }
        }

        if commits_behind == 0 && !commits.is_empty() {
            commits_behind = commits.len();
        }
    }

    let status_out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .ok();
    let has_uncommitted_changes = status_out.map(|o| !o.stdout.is_empty()).unwrap_or(false);

    Some(UpdateInfo {
        running_commit: RUNNING_COMMIT,
        build_time: BUILD_TIME,
        latest_commit,
        commits_behind,
        commits,
        repo_root: repo_root.to_path_buf(),
        has_uncommitted_changes,
    })
}

/// Executes the install script.
///
/// The build runs with `--message-format=json-render-diagnostics`, so a failure comes back as a
/// record per error instead of one log: the panel can name the first one without a human reading
/// 200 lines. The rendering stays inside each record, so the log is still readable.
pub fn execute_rebuild(repo_root: &Path) -> Result<String, BuildFailure> {
    let install_script = repo_root.join("apps/gpui/install");
    if !install_script.exists() {
        return Err(BuildFailure::without_diagnostics(format!(
            "Installer script not found at {}",
            install_script.display()
        )));
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let path_env = format!(
        "{}/.local/bin:{}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        home, home
    );

    let output = Command::new("/bin/bash")
        .arg(&install_script)
        .current_dir(repo_root)
        .env("PATH", path_env)
        .env("SJEL_BUILD_MESSAGE_FORMAT", "json-render-diagnostics")
        .output()
        .map_err(|e| {
            BuildFailure::without_diagnostics(format!("Failed to spawn installer: {e}"))
        })?;

    if output.status.success() {
        return Ok("Sjel Desktop rebuilt and installed successfully!".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // Cargo writes build records to stdout; anything else the script says goes to stderr.
    let read = read_cargo_output(&stdout);
    let first = read.errors.into_iter().next();
    let log = match (read.log.trim().is_empty(), stderr.trim().is_empty()) {
        (true, true) => "The installer failed without output.".to_owned(),
        (true, false) => stderr,
        (false, true) => read.log,
        (false, false) => format!("{}\n{stderr}", read.log),
    };
    Err(BuildFailure { first, log })
}

/// Rebuilds and restarts the local shell outside its own authenticated API.
pub fn restart_local_shell(repo_root: &Path) -> Result<(), String> {
    let runner = repo_root.join("tools/service-runner.sh");
    if !runner.is_file() {
        return Err(format!("Service runner not found at {}", runner.display()));
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let path_env = format!(
        "{}/.local/bin:{}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        home, home
    );
    let output = Command::new(runner)
        .args(["restart", "sjel-status"])
        .current_dir(repo_root)
        .env("PATH", path_env)
        .output()
        .map_err(|error| format!("Could not start the Sjel service runner: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() {
        "The Sjel service runner could not restart the local shell.".into()
    } else {
        detail
    })
}

/// The header's update control: a small pill that opens a popover with the newest commits,
/// Rebuild, and a link to Settings › Updates. The caller shows it only when commits are waiting.
pub fn header_pill(
    info: &UpdateInfo,
    building: bool,
    palette: Palette,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let label = if building {
        "Building…".to_owned()
    } else {
        format!("{} new", info.commits_behind)
    };
    let info = info.clone();
    Popover::new("update-popover")
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new("update-pill")
                .small()
                .outline()
                .icon(IconName::ArrowUp)
                .loading(building)
                .label(label)
                .tooltip("Sjel Desktop update"),
        )
        .content(move |_, _, cx| {
            let details = workspace.clone();
            let rebuild = workspace.clone();
            let behind = info.commits_behind;
            div()
                .w(px(320.))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Update available"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{behind} commit{} newer than this build ({})",
                                    if behind == 1 { "" } else { "s" },
                                    info.running_commit
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(info.commits.iter().take(3).map(|commit| {
                            div()
                                .flex()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(rgb(palette.accent))
                                        .child(commit.sha.clone()),
                                )
                                .child(div().min_w_0().truncate().child(commit.message.clone()))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("update-details")
                                .ghost()
                                .small()
                                .label("Details")
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.dismiss(window, cx);
                                    details.update(cx, |view, cx| {
                                        view.open_settings(Some(Section::Updates), cx)
                                    });
                                })),
                        )
                        .child(
                            Button::new("update-rebuild")
                                .primary()
                                .small()
                                .label("Rebuild")
                                .disabled(building)
                                .tooltip("Builds, installs, and restarts Sjel Desktop")
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.dismiss(window, cx);
                                    rebuild.update(cx, |view, cx| view.trigger_rebuild(window, cx));
                                })),
                        ),
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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

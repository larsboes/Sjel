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
    Failed(String),
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
pub fn execute_rebuild(repo_root: &Path) -> Result<String, String> {
    let install_script = repo_root.join("apps/gpui/install");
    if !install_script.exists() {
        return Err(format!(
            "Installer script not found at {:?}",
            install_script
        ));
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
        .output()
        .map_err(|e| format!("Failed to spawn installer: {}", e))?;

    if output.status.success() {
        Ok("Sjel Desktop rebuilt and installed successfully!".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
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
}

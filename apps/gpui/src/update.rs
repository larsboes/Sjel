//! In-app update detection and rebuild triggering for Sjel Desktop.
//!
//! Compares the running binary's build commit against the local Sjel repository HEAD,
//! fetches recent commits, and can execute `apps/gpui/install` in the background to rebuild
//! and install the updated release bundle into `~/Applications/Sjel Desktop.app`.

use std::path::{Path, PathBuf};
use std::process::Command;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, IconName, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, App, ClipboardItem, FontWeight, Window};

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
}

impl UpdateInfo {
    pub fn is_update_available(&self) -> bool {
        self.commits_behind > 0
            || (self.running_commit != "dev"
                && !self.running_commit.is_empty()
                && self.running_commit != self.latest_commit)
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

    Some(UpdateInfo {
        running_commit: RUNNING_COMMIT,
        build_time: BUILD_TIME,
        latest_commit,
        commits_behind,
        commits,
        repo_root: repo_root.to_path_buf(),
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

/// Relaunches Sjel Desktop application.
pub fn relaunch_app() {
    let _ = Command::new("open")
        .args(["-n", "-a", "Sjel Desktop"])
        .spawn();
    std::process::exit(0);
}

/// Renders the floating update popover card.
pub fn update_popover<FRebuild, FClose, FRelaunch>(
    info: &UpdateInfo,
    state: &RebuildState,
    palette: Palette,
    on_rebuild: FRebuild,
    on_close: FClose,
    on_relaunch: FRelaunch,
) -> impl IntoElement
where
    FRebuild: Fn(&mut Window, &mut App) + 'static + Clone,
    FClose: Fn(&mut Window, &mut App) + 'static + Clone,
    FRelaunch: Fn(&mut Window, &mut App) + 'static + Clone,
{
    let close = on_close.clone();
    let mut card = div()
        .id("update-popover")
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .w(px(360.))
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border));

    // Header
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .pb_1()
        .border_b_1()
        .border_color(rgba(palette.border))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child("App Update Available"),
                )
                .when(info.commits_behind > 0, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .px_2()
                            .py(px(1.))
                            .rounded_full()
                            .bg(rgba(palette.amber & 0xffffff | 0x24000000))
                            .text_color(rgb(palette.amber))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("+{} commits", info.commits_behind)),
                    )
                }),
        )
        .child(
            div()
                .id("close-update")
                .text_xs()
                .cursor_pointer()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgba(palette.raised))
                .text_color(rgb(palette.muted))
                .hover(|s| s.text_color(rgb(palette.text)))
                .on_click(move |_, window, app| close(window, app))
                .child("✕"),
        );
    card = card.child(header);

    // Commit diff summary
    let meta_row = div()
        .flex()
        .items_center()
        .justify_between()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(format!("Running {}", info.running_commit))
        .child(format!("Latest {}", info.latest_commit));
    card = card.child(meta_row);

    // Recent commits list
    if !info.commits.is_empty() {
        let mut list_box = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border & 0xffffff | 0x30000000));

        for commit in info.commits.iter().take(4) {
            let row = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .px_1()
                        .rounded(px(3.))
                        .bg(rgba(palette.surface))
                        .text_color(rgb(palette.accent))
                        .child(commit.sha.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .truncate()
                        .child(commit.message.clone()),
                );
            list_box = list_box.child(row);
        }
        card = card.child(list_box);
    }

    // State actions
    match state {
        RebuildState::Idle => {
            let rebuild = on_rebuild.clone();
            let action_bar = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("rebuild-now")
                        .primary()
                        .small()
                        .icon(IconName::RotateCw)
                        .label("Rebuild & Install")
                        .tooltip("Runs apps/gpui/install in the background")
                        .on_click(move |_, window, app| rebuild(window, app)),
                )
                .child(
                    Button::new("copy-command")
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .label("Copy Command")
                        .tooltip("Copy build command to clipboard")
                        .on_click(|_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                "cd apps/gpui && ./install".to_string(),
                            ));
                        }),
                );
            card = card.child(action_bar);
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Rebuilds release bundle to ~/Applications/Sjel Desktop.app"),
            );
        }
        RebuildState::Building => {
            let building_box = div()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(rgba(palette.accent & 0xffffff | 0x18000000))
                .child(
                    Button::new("building-indicator")
                        .ghost()
                        .small()
                        .loading(true)
                        .label("Rebuilding Sjel Desktop...")
                        .disabled(true),
                );
            card = card.child(building_box);
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Cargo release build in progress. You can keep using Sjel."),
            );
        }
        RebuildState::Success(msg) => {
            let relaunch = on_relaunch.clone();
            let success_box = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(rgba(palette.green & 0xffffff | 0x18000000))
                .border_1()
                .border_color(rgba(palette.green & 0xffffff | 0x40000000))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.green))
                        .child(format!("✓ {}", msg)),
                )
                .child(
                    Button::new("relaunch-now")
                        .primary()
                        .small()
                        .label("Restart Sjel Desktop Now")
                        .on_click(move |_, window, app| relaunch(window, app)),
                );
            card = card.child(success_box);
        }
        RebuildState::Failed(err) => {
            let rebuild = on_rebuild.clone();
            let err_box = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(rgba(palette.red & 0xffffff | 0x18000000))
                .border_1()
                .border_color(rgba(palette.red & 0xffffff | 0x40000000))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.red))
                        .child(format!("Rebuild failed: {}", err)),
                )
                .child(
                    Button::new("retry-rebuild")
                        .primary()
                        .small()
                        .label("Retry Rebuild")
                        .on_click(move |_, window, app| rebuild(window, app)),
                );
            card = card.child(err_box);
        }
    }

    card
}

/// Renders the clickable header update pill when an update is available.
pub fn update_badge(commits_behind: usize, palette: Palette) -> impl IntoElement {
    div()
        .id("update-available-badge")
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(3.))
        .rounded_full()
        .bg(rgba(palette.amber & 0xffffff | 0x24000000))
        .border_1()
        .border_color(rgba(palette.amber & 0xffffff | 0x60000000))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(palette.amber & 0xffffff | 0x38000000)))
        .child(div().size(px(6.)).rounded_full().bg(rgb(palette.amber)))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(if commits_behind > 0 {
                    format!("Update ({} behind)", commits_behind)
                } else {
                    "Update Available".to_string()
                }),
        )
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
        };
        assert!(behind.is_update_available());
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

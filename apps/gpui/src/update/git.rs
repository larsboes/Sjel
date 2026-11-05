use std::path::{Path, PathBuf};
use std::process::Command;

use super::types::{CommitSummary, UpdateInfo, BUILD_TIME, RUNNING_COMMIT};

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

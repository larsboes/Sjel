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

/// Locate the source checkout recorded by the desktop installer.
pub fn find_build_root() -> Option<PathBuf> {
    match std::env::var_os("SJEL_DESKTOP_SOURCE_ROOT") {
        Some(root) => {
            let root = PathBuf::from(root);
            root.join("apps/gpui/install").is_file().then_some(root)
        }
        None => find_repo_root(),
    }
}

fn git(repo_root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repo_root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(super) fn can_rebuild(repo_root: &Path, running: &str, target: &str) -> bool {
    running == "dev" || git(repo_root, &["merge-base", "--is-ancestor", running, target]).is_some()
}

/// Checks the source checkout for commits that follow this build.
pub fn check_git_updates(repo_root: &Path) -> Option<UpdateInfo> {
    let target = git(repo_root, &["rev-parse", "HEAD"])?;
    let latest_commit = git(repo_root, &["rev-parse", "--short", &target])?;
    let can_rebuild = can_rebuild(repo_root, RUNNING_COMMIT, &target);
    let mut commits_behind = 0;
    let mut commits = Vec::new();

    if can_rebuild && RUNNING_COMMIT != "dev" {
        let range = format!("{RUNNING_COMMIT}..{target}");
        commits_behind = git(repo_root, &["rev-list", "--count", &range])?
            .parse()
            .ok()?;
        commits = git(repo_root, &["log", "--oneline", "-n", "6", &range])?
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(sha, message)| CommitSummary {
                sha: sha.to_owned(),
                message: message.to_owned(),
            })
            .collect();
    }

    let has_uncommitted_changes =
        git(repo_root, &["status", "--porcelain"]).is_some_and(|status| !status.is_empty());

    Some(UpdateInfo {
        running_commit: RUNNING_COMMIT,
        build_time: BUILD_TIME,
        latest_commit,
        commits_behind,
        commits,
        repo_root: repo_root.to_path_buf(),
        has_uncommitted_changes,
        can_rebuild,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_only_accepts_the_current_revision_or_a_descendant() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(can_rebuild(root, "HEAD", "HEAD"));
        assert!(can_rebuild(root, "HEAD^", "HEAD"));
        assert!(!can_rebuild(root, "HEAD", "HEAD^"));
        assert!(!can_rebuild(root, "missing-build-revision", "HEAD"));
    }
}

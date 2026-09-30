//! `prune` — repo-scoped, policy-free reclaim.
//!
//! `report` and `apply` answer "what is filling this machine" and act on the overlay's
//! policy. This verb answers a narrower question with no policy at all: what can this
//! checkout give back right now. Nothing here reads `storage-policy.toml`, so it works on a
//! clone with no overlay, which is exactly when someone needs the disk back.
//!
//! Three refusals hold it inside the checkout:
//!
//! 1. Every path is canonicalised and must sit under the repo root or the Cargo target dir.
//!    A symlink pointing out of the tree fails this after resolution, not before.
//! 2. Every path must be ignored by git. The `--node-modules` list is derived from
//!    `git status --ignored --short -z` rather than a hardcoded name, and the guard re-asks
//!    `git ls-files` per path so a tracked file can never be an argument.
//! 3. The repo root and the target dir are never themselves removed — only their contents,
//!    or `cargo clean` for the target dir, which is cargo's own verb for it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::measure::{fmt_bytes, Walk};

/// The three ignored directory names this verb removes.
///
/// Every one regrows from a lockfile or a build: `bun install` writes `node_modules`,
/// `vite build` writes `dist`, `svelte-kit sync` writes `.svelte-kit`. Nothing here holds a
/// value that is not derivable from a tracked file.
pub const PURGEABLE: [&str; 3] = ["node_modules", ".svelte-kit", "dist"];

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Ignored purgeable directories, read out of `git status --ignored --short -z`.
///
/// `-z` rather than the default: git quotes a path containing a space or a non-ASCII byte
/// in the plain format, and a NUL-separated record needs no unquoting at all. Each record
/// is two status characters, a space, then the path.
///
/// git reports the OUTERMOST ignored directory, so a `node_modules` nested inside another
/// arrives as part of its parent and needs no separate entry.
pub fn ignored_purgeable(status_z: &str) -> Vec<String> {
    status_z
        .split('\0')
        .filter(|rec| rec.starts_with("!! "))
        .map(|rec| rec[3..].trim_end_matches('/'))
        .filter(|p| {
            p.rsplit('/')
                .next()
                .is_some_and(|last| PURGEABLE.contains(&last))
        })
        .map(str::to_string)
        .collect()
}

fn git(repo_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Is this path one `prune` may remove? The answer has to survive a symlink, so it is asked
/// of the canonical path and of git, never of the string that was passed in.
pub fn removable(path: &Path, repo_root: &Path, target_dir: &Path) -> Result<PathBuf, String> {
    let real = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let repo = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.into());
    let target = target_dir
        .canonicalize()
        .unwrap_or_else(|_| target_dir.into());
    if !(real.starts_with(&repo) || real.starts_with(&target)) {
        return Err(format!(
            "{} resolves outside the repo ({}) and the target dir ({}) — refusing",
            real.display(),
            repo.display(),
            target.display()
        ));
    }
    if real == repo || real == target {
        return Err(format!("{} is the root itself — refusing", real.display()));
    }
    // The second half of refusal 2: the `--node-modules` list is derived from what git
    // calls ignored, and this re-asks per path so nothing tracked can be an argument.
    // A path under an external `CARGO_TARGET_DIR` is in no index at all — git exits
    // non-zero, `git` returns `None`, and "not tracked" is the correct answer.
    let tracked = git(&repo, &["ls-files", "--", &real.to_string_lossy()]).unwrap_or_default();
    if !tracked.trim().is_empty() {
        return Err(format!("{} is tracked by git — refusing", real.display()));
    }
    Ok(real)
}

fn measure(paths: Vec<PathBuf>) -> Vec<Candidate> {
    paths
        .into_iter()
        .map(|path| {
            let bytes = Walk::default().size(&path);
            Candidate { path, bytes }
        })
        .collect()
}

/// `target/<profile>/incremental` for every profile that has one.
///
/// Always safe and always regrows: incremental state is a cache of the last compilation of
/// unchanged code, so removing it costs one full rebuild and nothing else. On 2026-09-03 it
/// was 7 GB of the 21 GB this workspace's target dir had reached.
pub fn incremental_dirs(target_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(target_dir) else {
        return vec![];
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path().join("incremental"))
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

pub fn node_module_dirs(repo_root: &Path) -> Vec<PathBuf> {
    let status = git(repo_root, &["status", "--ignored", "--short", "-z"]).unwrap_or_default();
    ignored_purgeable(&status)
        .into_iter()
        .map(|rel| repo_root.join(rel))
        .collect()
}

pub struct Plan {
    pub incremental: Vec<Candidate>,
    pub node_modules: Vec<Candidate>,
    /// The whole target dir, when `--target` was asked for. Removed by `cargo clean`, not
    /// by this process.
    pub target: Option<Candidate>,
    /// Target dirs cargo does not resolve to here. `cargo clean` run against the primary
    /// directory cannot reach them, so each is cleaned through its own manifest with
    /// `--target-dir` naming it, which outranks both `CARGO_TARGET_DIR` and the
    /// machine-wide `[build] target-dir` in the cargo config.
    pub secondary_targets: Vec<crate::target::SecondaryTarget>,
}

impl Plan {
    pub fn build(
        repo_root: &Path,
        target_dir: &Path,
        want_incremental: bool,
        want_target: bool,
        want_node_modules: bool,
    ) -> Plan {
        Plan {
            incremental: if want_incremental {
                measure(incremental_dirs(target_dir))
            } else {
                vec![]
            },
            node_modules: if want_node_modules {
                measure(node_module_dirs(repo_root))
            } else {
                vec![]
            },
            target: want_target.then(|| Candidate {
                path: target_dir.to_path_buf(),
                bytes: Walk::default().size(target_dir),
            }),
            secondary_targets: if want_target {
                crate::target::secondary_targets(repo_root, target_dir)
            } else {
                vec![]
            },
        }
    }

    pub fn bytes(&self) -> u64 {
        self.incremental.iter().map(|c| c.bytes).sum::<u64>()
            + self.node_modules.iter().map(|c| c.bytes).sum::<u64>()
            + self.target.as_ref().map_or(0, |c| c.bytes)
            + self.secondary_targets.iter().map(|s| s.bytes).sum::<u64>()
    }
}

/// Run the plan. Returns the lines to print and the number of refusals, which the caller
/// turns into an exit code.
pub fn run(
    plan: &Plan,
    repo_root: &Path,
    target_dir: &Path,
    dry_run: bool,
) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut refused = 0;

    for group in [
        ("incremental", &plan.incremental),
        ("node_modules", &plan.node_modules),
    ] {
        for candidate in group.1 {
            match removable(&candidate.path, repo_root, target_dir) {
                Err(why) => {
                    lines.push(format!("  refused  {why}"));
                    refused += 1;
                }
                Ok(real) => {
                    if dry_run {
                        lines.push(format!(
                            "  {:>9}  {} (dry run)",
                            fmt_bytes(candidate.bytes),
                            real.display()
                        ));
                        continue;
                    }
                    match std::fs::remove_dir_all(&real) {
                        Ok(()) => lines.push(format!(
                            "  {:>9}  {}",
                            fmt_bytes(candidate.bytes),
                            real.display()
                        )),
                        Err(e) => {
                            lines.push(format!("  failed   {}: {e}", real.display()));
                            refused += 1;
                        }
                    }
                }
            }
        }
    }

    if let Some(target) = &plan.target {
        // `cargo clean` rather than a recursive delete, for the reason the policy schema
        // already states: never `rm -rf` what a tool can clean itself. Cargo owns the
        // layout, including the lock file it keeps in there.
        if dry_run {
            lines.push(format!(
                "  {:>9}  {} via cargo clean (dry run)",
                fmt_bytes(target.bytes),
                target.path.display()
            ));
        } else {
            let status = Command::new("cargo")
                .arg("clean")
                .arg("--manifest-path")
                .arg(repo_root.join("Cargo.toml"))
                .status();
            match status {
                Ok(s) if s.success() => lines.push(format!(
                    "  {:>9}  {} via cargo clean",
                    fmt_bytes(target.bytes),
                    target.path.display()
                )),
                Ok(s) => {
                    lines.push(format!("  failed   cargo clean exited {s}"));
                    refused += 1;
                }
                Err(e) => {
                    lines.push(format!("  failed   cargo clean: {e}"));
                    refused += 1;
                }
            }
        }
    }

    for secondary in &plan.secondary_targets {
        if dry_run {
            lines.push(format!(
                "  {:>9}  {} via cargo clean (dry run)",
                fmt_bytes(secondary.bytes),
                secondary.path
            ));
            continue;
        }
        // `--target-dir` rather than a scrubbed environment: CARGO_TARGET_DIR is not the only
        // thing that can redirect this. The machine-wide `[build] target-dir` in
        // ~/.cargo/config.toml points at the primary dir, so removing the variable would
        // make cargo clean THAT, report success, and leave this directory untouched. The
        // CLI option outranks both the variable and the config.
        let status = Command::new("cargo")
            .arg("clean")
            .arg("--manifest-path")
            .arg(&secondary.manifest)
            .arg("--target-dir")
            .arg(&secondary.path)
            .status();
        match status {
            Ok(s) if s.success() => lines.push(format!(
                "  {:>9}  {} via cargo clean",
                fmt_bytes(secondary.bytes),
                secondary.path
            )),
            Ok(s) => {
                lines.push(format!(
                    "  failed   cargo clean exited {s} for {}",
                    secondary.path
                ));
                refused += 1;
            }
            Err(e) => {
                lines.push(format!("  failed   cargo clean: {e}"));
                refused += 1;
            }
        }
    }

    (lines, refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purgeable_directories_are_read_out_of_the_ignored_status() {
        let status = concat!(
            "!! dashboard/node_modules/\0",
            "!! dashboard/.svelte-kit/\0",
            "!! capabilities/comms/ui/dist/\0",
            "!! target/\0",
            "!! .env\0",
            " M README.md\0",
        );
        assert_eq!(
            ignored_purgeable(status),
            vec![
                "dashboard/node_modules",
                "dashboard/.svelte-kit",
                "capabilities/comms/ui/dist",
            ]
        );
    }

    #[test]
    fn a_path_containing_a_space_survives_because_the_format_is_nul_separated() {
        let status = "!! my dir/node_modules/\0";
        assert_eq!(ignored_purgeable(status), vec!["my dir/node_modules"]);
    }

    #[test]
    fn a_file_that_merely_ends_in_a_purgeable_name_is_not_a_directory_to_remove() {
        // `dist` matches; `predist` and `dist.tar` do not. The comparison is the whole last
        // segment, never a suffix.
        let status = "!! build/predist\0!! build/dist.tar\0!! build/dist\0";
        assert_eq!(ignored_purgeable(status), vec!["build/dist"]);
    }

    #[test]
    fn nothing_ignored_means_nothing_to_prune() {
        assert!(ignored_purgeable("").is_empty());
        assert!(ignored_purgeable(" M tools/storage/src/prune.rs\0").is_empty());
    }

    #[test]
    fn incremental_dirs_are_found_per_profile_and_only_when_they_exist() {
        let root = crate::testutil::tempdir("prune-incremental");
        std::fs::create_dir_all(root.join("debug/incremental")).unwrap();
        std::fs::create_dir_all(root.join("release/deps")).unwrap();
        assert_eq!(
            incremental_dirs(&root),
            vec![root.join("debug").join("incremental")]
        );
    }

    #[test]
    fn a_path_outside_the_repo_and_the_target_dir_is_refused() {
        let root = crate::testutil::tempdir("prune-scope");
        let repo = root.join("repo");
        let target = root.join("target");
        let outside = root.join("elsewhere");
        for d in [&repo, &target, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let err = removable(&outside, &repo, &target).unwrap_err();
        assert!(err.contains("outside the repo"), "{err}");
    }

    #[test]
    fn a_symlink_pointing_out_of_the_repo_is_refused_after_resolution_not_before() {
        // The failure this guards: `dashboard/node_modules` being a symlink to a shared
        // store, and the prefix check passing on the un-resolved path.
        let root = crate::testutil::tempdir("prune-symlink");
        let repo = root.join("repo");
        let target = root.join("target");
        let outside = root.join("elsewhere");
        for d in [&repo, &target, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let link = repo.join("node_modules");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err = removable(&link, &repo, &target).unwrap_err();
        assert!(err.contains("outside the repo"), "{err}");
    }

    #[test]
    fn the_repo_root_and_the_target_dir_are_never_themselves_removable() {
        let root = crate::testutil::tempdir("prune-roots");
        let repo = root.join("repo");
        let target = root.join("target");
        for d in [&repo, &target] {
            std::fs::create_dir_all(d).unwrap();
        }
        assert!(removable(&repo, &repo, &target)
            .unwrap_err()
            .contains("root itself"));
        assert!(removable(&target, &repo, &target)
            .unwrap_err()
            .contains("root itself"));
    }

    #[test]
    fn a_path_inside_the_repo_is_removable() {
        let root = crate::testutil::tempdir("prune-inside");
        let repo = root.join("repo");
        let target = root.join("target");
        let inside = repo.join("dashboard/node_modules");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        assert!(removable(&inside, &repo, &target).is_ok());
    }

    #[test]
    fn a_missing_path_is_refused_rather_than_removed_blind() {
        let root = crate::testutil::tempdir("prune-missing");
        let repo = root.join("repo");
        let target = root.join("target");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        assert!(removable(&repo.join("gone"), &repo, &target).is_err());
    }

    #[test]
    fn a_dry_run_removes_nothing() {
        let root = crate::testutil::tempdir("prune-dry");
        let repo = root.join("repo");
        let target = repo.join("target");
        let incremental = target.join("debug/incremental");
        std::fs::create_dir_all(&incremental).unwrap();
        std::fs::write(incremental.join("f"), vec![0u8; 16 * 1024]).unwrap();

        let plan = Plan::build(&repo, &target, true, false, false);
        assert_eq!(plan.incremental.len(), 1);
        assert!(plan.bytes() >= 16 * 1024);
        let (lines, refused) = run(&plan, &repo, &target, true);
        assert_eq!(refused, 0);
        assert!(lines[0].contains("dry run"), "{lines:?}");
        assert!(incremental.exists(), "a dry run must leave the tree alone");

        let (_, refused) = run(&plan, &repo, &target, false);
        assert_eq!(refused, 0);
        assert!(!incremental.exists(), "the real run must remove it");
    }
}

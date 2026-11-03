//! `prune` — repo-scoped, policy-free reclaim.
//!
//! `report` and `apply` answer "what is filling this machine" and act on the overlay's
//! policy. This verb answers a narrower question: what can this checkout give back right
//! now. Nothing here reads `storage-policy.toml`, so it works on a clone with no overlay,
//! which is exactly when someone needs the disk back — the caller translates the overlay
//! into an [`Authority`] and hands it over, and a machine with no policy passes none.
//!
//! Four refusals hold it:
//!
//! 1. Every path is canonicalised and must sit under the repo root or the Cargo target dir.
//!    A symlink pointing out of the tree fails this after resolution, not before.
//! 2. Every path must be ignored by git. The `--node-modules` list is derived from
//!    `git status --ignored --short -z` rather than a hardcoded name, and the guard re-asks
//!    `git ls-files` per path so a tracked file can never be an argument.
//! 3. The repo root and the target dir are never themselves removed — only their contents,
//!    or `cargo clean` for the target dir, which is cargo's own verb for it.
//! 4. A directory a live process has open, because removing it breaks that process or the
//!    build it is in; `inuse.rs` carries what that cost on 2026-10-12.
//!
//! And a fifth when the machine has a policy: a path the policy names, or that *holds* a
//! path it names, is the policy's to decide and not this verb's. On 2026-10-13 the policy
//! class `rust-shared-target` called `~/.cargo-target` report-only because `release/` holds a
//! binary that session was running, while `--target` offered to `cargo clean` all 18.1 GB of
//! it. Two answers to one question, and the sweep was the wrong one — so the policy's answer
//! wins and `--force` is the only way past it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::guard::{git, Caller, Guard};
use crate::measure::{fmt_bytes, Walk};
use crate::reclaim::Outcome;

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
}

/// The bytes a set of candidates gives back, and what it leaves out because a larger
/// candidate in that same set covers it.
///
/// `incremental_dirs` reads `target/<profile>/incremental`, so under `--target` every
/// incremental candidate is inside the target candidate, and summing both reported 210.9 GB
/// reclaimable for roughly 150 GB on 2026-10-11 — a 40% overstatement of the one number that
/// decides whether anyone acts. A secondary target dir nested inside another is the same
/// shape, so the rule is written on the paths rather than special cased for the pair that
/// happened to fail. Two candidates on one path are one removal: without that each would see
/// the other as a non-strict prefix and both would survive.
///
/// Pure, and the caller passes the candidates that survived its refusals — an incremental dir
/// stops being nested the moment the target dir holding it is refused.
fn totals(candidates: &[(u64, PathBuf)]) -> (u64, u64) {
    let mut all = candidates.to_vec();
    all.sort_by(|a, b| a.1.cmp(&b.1));
    all.dedup_by(|a, b| a.1 == b.1);
    let nested: Vec<bool> = all
        .iter()
        .map(|(_, p)| all.iter().any(|(_, q)| q != p && p.starts_with(q)))
        .collect();
    let naive: u64 = all.iter().map(|(bytes, _)| bytes).sum();
    let top: u64 = all
        .iter()
        .zip(&nested)
        .filter_map(|((bytes, _), nested)| (!nested).then_some(bytes))
        .sum();
    (top, naive.saturating_sub(top))
}

/// Run the plan.
///
/// Every refusal is decided before anything is removed, so a dry run reports the plan that
/// would really run rather than the one the flags asked for.
pub fn run(plan: &Plan, repo_root: &Path, guard: &Guard, dry_run: bool) -> Outcome {
    let mut lines = Vec::new();
    let mut refused = 0;
    // Rebuilt as candidates survive, so the total below the lines cannot claim bytes a
    // refusal left on disk. The nesting rule is applied to the survivors, because an
    // incremental dir stops being nested once the target dir holding it has been refused.
    let mut survivors: Vec<(u64, PathBuf)> = Vec::new();

    for group in [
        ("incremental", &plan.incremental),
        ("node_modules", &plan.node_modules),
    ] {
        for candidate in group.1 {
            let real = match guard.check(&candidate.path, Caller::Checkout) {
                Err(why) => {
                    // `check` returns a bare sentence for a structural refusal and a formatted
                    // one for a judgement call, so the marker is added only when it is missing.
                    lines.push(if why.starts_with("  refused") {
                        why
                    } else {
                        format!("  refused  {why}")
                    });
                    refused += 1;
                    continue;
                }
                Ok(real) => real,
            };
            if dry_run {
                lines.push(format!(
                    "  {:>9}  {} (dry run)",
                    fmt_bytes(candidate.bytes),
                    real.display()
                ));
                survivors.push((candidate.bytes, real));
                continue;
            }
            match std::fs::remove_dir_all(&real) {
                Ok(()) => {
                    lines.push(format!(
                        "  {:>9}  {}",
                        fmt_bytes(candidate.bytes),
                        real.display()
                    ));
                    survivors.push((candidate.bytes, real));
                }
                Err(e) => {
                    lines.push(format!("  failed   {}: {e}", real.display()));
                    refused += 1;
                }
            }
        }
    }

    if let Some(target) = &plan.target {
        // `cargo clean` rather than a recursive delete, for the reason the policy schema
        // already states: never `rm -rf` what a tool can clean itself. Cargo owns the
        // layout, including the lock file it keeps in there.
        if let Err(why) = guard.check(&target.path, Caller::Target) {
            lines.push(why);
            refused += 1;
        } else if dry_run {
            lines.push(format!(
                "  {:>9}  {} via cargo clean (dry run)",
                fmt_bytes(target.bytes),
                target.path.display()
            ));
            survivors.push((target.bytes, target.path.clone()));
        } else {
            let status = Command::new("cargo")
                .arg("clean")
                .arg("--manifest-path")
                .arg(repo_root.join("Cargo.toml"))
                .status();
            match status {
                Ok(s) if s.success() => {
                    lines.push(format!(
                        "  {:>9}  {} via cargo clean",
                        fmt_bytes(target.bytes),
                        target.path.display()
                    ));
                    survivors.push((target.bytes, target.path.clone()));
                }
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
        let path = PathBuf::from(&secondary.path);
        if let Err(why) = guard.check(&path, Caller::Target) {
            lines.push(why);
            refused += 1;
            continue;
        }
        if dry_run {
            lines.push(format!(
                "  {:>9}  {} via cargo clean (dry run)",
                fmt_bytes(secondary.bytes),
                secondary.path
            ));
            survivors.push((secondary.bytes, path));
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
            Ok(s) if s.success() => {
                lines.push(format!(
                    "  {:>9}  {} via cargo clean",
                    fmt_bytes(secondary.bytes),
                    secondary.path
                ));
                survivors.push((secondary.bytes, path));
            }
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

    let (bytes, nested_bytes) = totals(&survivors);
    Outcome {
        lines,
        refused,
        skipped: 0,
        bytes,
        nested_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::Authority;
    use crate::inuse::{self, Usage};

    #[test]
    fn purgeable_directories_are_read_out_of_the_ignored_status() {
        let status = concat!(
            "!! dashboard/node_modules/\0",
            "!! dashboard/.svelte-kit/\0",
            "!! capabilities/comms/ui/dist/\0",
            "!! target/\0",
            "!! notes.txt\0",
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
        assert!(ignored_purgeable(" M tools/storage/src/checkout.rs\0").is_empty());
    }

    #[test]
    fn incremental_dirs_are_found_per_profile_and_only_when_they_exist() {
        let root = crate::testutil::tempdir("checkout-incremental");
        std::fs::create_dir_all(root.join("debug/incremental")).unwrap();
        std::fs::create_dir_all(root.join("release/deps")).unwrap();
        assert_eq!(
            incremental_dirs(&root),
            vec![root.join("debug").join("incremental")]
        );
    }

    #[test]
    fn a_candidate_inside_another_candidate_is_counted_once() {
        // The measured failure: `--incremental --target` reported 210.9 GB would be freed where
        // the truth was about 150 GB, because the incremental subtotal was added on top of the
        // target directory that already contains it — a 40% overstatement of the one number a
        // reader decides on.
        const GB_U: u64 = 1024 * 1024 * 1024;
        let with_target = [
            (60 * GB_U, PathBuf::from("/t/debug/incremental")),
            (GB_U / 2, PathBuf::from("/repo/dashboard/node_modules")),
            (140 * GB_U, PathBuf::from("/t")),
        ];
        // The incremental dir is still a line `run` prints; only the arithmetic drops it.
        let (bytes, nested) = totals(&with_target);
        assert_eq!(nested, 60 * GB_U);
        assert_eq!(bytes, 140 * GB_U + GB_U / 2);

        // Without the target dir the incremental dir is the whole plan, and nothing is nested.
        let (bytes, nested) = totals(&with_target[..2]);
        assert_eq!(nested, 0);
        assert_eq!(bytes, 60 * GB_U + GB_U / 2);
    }

    #[test]
    fn a_sibling_whose_name_shares_a_prefix_is_not_nested() {
        // `/t/debug` is inside `/t`; `/tb` is not. `Path::starts_with` compares components,
        // and this test is what fails if that ever becomes a string comparison.
        let candidates = [
            (1024, PathBuf::from("/t/debug/incremental")),
            (2048, PathBuf::from("/tb/node_modules")),
            (4096, PathBuf::from("/t")),
        ];
        let (bytes, nested) = totals(&candidates);
        assert_eq!(nested, 1024);
        assert_eq!(bytes, 4096 + 2048);
    }

    /// Run `body` with a live process whose working directory is inside `dir`, so the probe has
    /// something real to find and the test does not depend on whatever happens to be running on
    /// the machine.
    ///
    /// `usage` is a snapshot, so this waits for the holder to appear before calling `body`
    /// rather than assuming a process that has just been spawned is already visible. The holder
    /// is killed and reaped on every path.
    fn with_holder(dir: &Path, body: impl FnOnce()) {
        let mut child = Command::new("sleep")
            .arg("30")
            .current_dir(dir)
            .spawn()
            .expect("sleep");
        let mut visible = false;
        for _ in 0..40 {
            if !matches!(inuse::usage(dir), Usage::Free) {
                visible = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if visible {
            body();
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(visible, "the holder never showed up in {}", dir.display());
    }

    /// A one-candidate plan over `modules`, for the tests below.
    fn held_plan(modules: &Path) -> Plan {
        Plan {
            incremental: vec![],
            node_modules: vec![Candidate {
                path: modules.to_path_buf(),
                bytes: 4096,
            }],
            target: None,
            secondary_targets: vec![],
        }
    }

    /// A repo with one `node_modules` holding one file, and the paths to hand a Guard.
    fn fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = crate::testutil::tempdir(name);
        let repo = root.join("repo");
        let target = repo.join("target");
        let modules = repo.join("node_modules");
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(modules.join("f"), vec![0u8; 4096]).unwrap();
        (repo, target, modules)
    }

    #[test]
    fn a_candidate_a_live_process_holds_is_refused() {
        let (repo, target, modules) = fixture("checkout-held");
        let authority = Authority::default();
        with_holder(&modules, || {
            let guard = Guard::new(&authority, &repo, &target, false);
            let outcome = run(&held_plan(&modules), &repo, &guard, false);
            assert_eq!(outcome.refused, 1, "{:?}", outcome.lines);
            // The number a reader acts on. A refusal that still counted the bytes would print
            // "4 KB freed" for a directory that is still there.
            assert_eq!(outcome.bytes, 0);
            assert!(modules.exists(), "a held directory must survive");
            assert!(
                outcome.lines[0].contains("is held by 1 live process"),
                "{:?}",
                outcome.lines
            );
        });
    }

    #[test]
    fn force_removes_what_the_probe_refused() {
        let (repo, target, modules) = fixture("checkout-forced");
        let authority = Authority::default();
        with_holder(&modules, || {
            let guard = Guard::new(&authority, &repo, &target, true);
            let outcome = run(&held_plan(&modules), &repo, &guard, false);
            assert_eq!(outcome.refused, 0, "{:?}", outcome.lines);
            assert_eq!(outcome.bytes, 4096);
            assert!(!modules.exists(), "--force is the way past the probe");
        });
    }

    #[test]
    fn the_policy_refuses_what_force_removes() {
        let (repo, target, modules) = fixture("checkout-authority");
        let plan = held_plan(&modules);

        let named = Authority::new(
            vec![],
            vec![(modules.clone(), "the policy protects it".to_string())],
        );
        let guard = Guard::new(&named, &repo, &target, false);
        let outcome = run(&plan, &repo, &guard, false);
        assert_eq!(outcome.refused, 1, "{:?}", outcome.lines);
        assert_eq!(outcome.bytes, 0);
        assert!(modules.exists(), "the policy's answer must win");

        let guard = Guard::new(&named, &repo, &target, true);
        let outcome = run(&plan, &repo, &guard, false);
        assert_eq!(outcome.refused, 0, "{:?}", outcome.lines);
        assert_eq!(outcome.bytes, 4096);
        assert!(!modules.exists(), "--force is the way past the policy");
    }

    #[test]
    fn a_dry_run_removes_nothing() {
        let root = crate::testutil::tempdir("checkout-dry");
        let repo = root.join("repo");
        let target = repo.join("target");
        let incremental = target.join("debug/incremental");
        std::fs::create_dir_all(&incremental).unwrap();
        std::fs::write(incremental.join("f"), vec![0u8; 16 * 1024]).unwrap();

        let plan = Plan::build(&repo, &target, true, false, false);
        assert_eq!(plan.incremental.len(), 1);
        assert!(plan.incremental[0].bytes >= 16 * 1024);
        let authority = Authority::default();
        let guard = Guard::new(&authority, &repo, &target, false);

        let outcome = run(&plan, &repo, &guard, true);
        assert_eq!(outcome.refused, 0);
        assert!(outcome.lines[0].contains("dry run"), "{:?}", outcome.lines);
        assert!(incremental.exists(), "a dry run must leave the tree alone");

        let outcome = run(&plan, &repo, &guard, false);
        assert_eq!(outcome.refused, 0);
        assert!(!incremental.exists(), "the real run must remove it");
    }
}

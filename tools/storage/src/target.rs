//! `target` — the Cargo build cache measured against PRD §9's R6.
//!
//! R6, ratified as Q53 on 2026-08-28: build artifacts are not state, and `target/debug`
//! may not exceed `target/release` by more than 3×. The rule is deliberately a ratio and
//! not a GB figure — `target/release` is an internal control with the same crates, the
//! same machine and the same moment, differing only in profile, so it survives a bigger
//! disk. Q53 names `tools/doctor` as the checker; doctor runs `sjel-storage target --json`
//! and reports what comes back.
//!
//! The second question this answers is the one that produced the mess. Measured 2026-09-03,
//! before a `cargo clean`: `target/` held 21 GB, of which `debug/deps` was 13 GB and
//! `debug/incremental` 7 GB, accumulated across toolchain rolls. A clean full
//! `cargo build --workspace` rebuilt the same tree as 4.7 GB in 49 s. The bulk was
//! artifacts from rustc versions no longer installed, which no ratio detects, because both
//! profiles carry the same rot.
//!
//! What detects it is `target/.rustc_info.json`: cargo caches the `rustc -vV` output it saw
//! there. When the recorded commit hash differs from the `rustc -vV` this machine runs
//! today, every artifact beside it was produced by a compiler that is gone and a clean is
//! warranted.
//!
//! The limit of that claim, observed on 2026-09-05 when rustup rolled stable from 1.98.0
//! (88d9e12ae) to 1.98.1 (48a229cea) mid-session: cargo REWRITES `.rustc_info.json` on its
//! first run under the new compiler, and it does not delete the previous generation's
//! output from `deps/`. So the mismatch is visible only between the roll and the next
//! cargo invocation, and a match afterwards means "cargo has run since the roll", not "the
//! tree is clean". A match is therefore reported as a fact, never as a clean bill of
//! health. Sizes and the R6 ratio are what stay true either way.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::measure::Walk;

/// PRD §9 R6. Three, not "a bit over three": the rule is the number.
pub const R6_MAX_RATIO: f64 = 3.0;

/// A Cargo target dir inside the repo that is not the one cargo resolves to here.
///
/// Cargo stamps its target dir with a `CACHEDIR.TAG`, so this finds them by that marker
/// rather than by guessing directory names. The case that produced it, measured 2026-09-30:
/// Xcode's build phase launches `cargo` without the shell environment, so `CARGO_TARGET_DIR`
/// is unset and cargo falls back to the workspace-local `target/`. `prune --target` runs
/// `cargo clean`, which honours that same variable, so it could only ever clean the one
/// directory — and `dashboard/src-tauri/target` reached 15 GB unremarked.
#[derive(Debug, Clone, Serialize)]
pub struct SecondaryTarget {
    pub path: String,
    /// The workspace manifest whose `cargo clean` is what removes this directory.
    pub manifest: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Buckets {
    pub deps: u64,
    pub incremental: u64,
    pub build: u64,
    pub fingerprint: u64,
    pub binaries: u64,
    pub other: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub name: String,
    pub present: bool,
    pub bytes: u64,
    /// Entries in `.fingerprint`, which is one per compilation unit cargo has built into
    /// this profile.
    ///
    /// R6 is a ratio against an internal control, and Q53's argument for `target/release`
    /// being that control is "same crates, same machine, same moment". This number is what
    /// says whether that held. A checkout with a full debug build and a partial release
    /// build reports a large ratio without anything being wrong, and the unit counts are
    /// the only thing on the report that shows it. Deliberately NOT a gate: a threshold on
    /// it would be the guess wearing a gate's clothes that §9 refuses.
    pub units: usize,
    pub buckets: Buckets,
}

#[derive(Debug, Clone, Serialize)]
pub struct Toolchain {
    /// The rustc commit hash cargo recorded in `target/.rustc_info.json`.
    pub recorded: Option<String>,
    /// The rustc commit hash this machine runs now.
    pub current: Option<String>,
    /// False only when both hashes are known and differ. An unknown hash is not a finding.
    pub matches: bool,
    /// What a clean would reclaim if the mismatch is real: deps plus fingerprints across
    /// every profile, which is where a superseded toolchain's output accumulates.
    pub stale_candidate_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetReport {
    pub target_dir: String,
    /// `CARGO_TARGET_DIR` or `repo` — which answer produced `target_dir`.
    pub source: String,
    pub bytes: u64,
    pub profiles: Vec<Profile>,
    /// debug ÷ release. `None` when release has never been built, which is not a failure:
    /// a ratio with no denominator is unknown, not exceeded.
    pub ratio: Option<f64>,
    pub r6_max_ratio: f64,
    /// `ok`, `over`, or `unknown`.
    pub r6: String,
    pub toolchain: Toolchain,
    /// Cargo target dirs inside the repo that cargo does not resolve to. Empty on a checkout
    /// where `CARGO_TARGET_DIR` covers everything, which is the intended state.
    pub secondary: Vec<SecondaryTarget>,
    pub secondary_bytes: u64,
}

/// Where this workspace's build cache is. `CARGO_TARGET_DIR` wins because that is what
/// cargo itself obeys; a report on `<repo>/target` while cargo writes elsewhere would be a
/// report on an empty directory.
pub fn target_dir(repo_root: &Path) -> (PathBuf, &'static str) {
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(v) if !v.trim().is_empty() => (PathBuf::from(v), "CARGO_TARGET_DIR"),
        _ => (repo_root.join("target"), "repo"),
    }
}

/// Every Cargo target dir under `repo_root` that is not `primary`, largest first.
///
/// The repo's own `target/` is deliberately never one of them. That directory is not an
/// escapee — it is where a workspace build puts the binaries the supervisor runs
/// (`capabilities/calendar/service.toml` names `target/release/calendar-server`), which is why
/// `tools/cargo-hermetic` refuses to point `CARGO_TARGET_DIR` at anything inside the checkout.
/// Reporting it as reclaimable would invite exactly the mistake that guard exists to prevent.
/// What this is for is a *nested* workspace: `dashboard/src-tauri/target` is a second target dir
/// under the repo root whose builds the checkout's own `target/` knows nothing about.
///
/// The walk is bounded and skips the two subtrees that cannot hold a workspace and are
/// expensive to descend: `.git` and `node_modules`. A directory that *is* a target dir is
/// recorded and not descended into, because cargo keeps no workspace inside one.
pub fn secondary_targets(repo_root: &Path, primary: &Path) -> Vec<SecondaryTarget> {
    let primary = std::fs::canonicalize(primary).unwrap_or_else(|_| primary.to_path_buf());
    let deployment = canonical(repo_root.join("target"));
    let mut out = Vec::new();
    scan(repo_root, &primary, deployment.as_deref(), &mut out, 0);
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

/// `canonicalize` when the path exists, because a path that does not exist is not a directory
/// this walk can meet, and the caller compares rather than opens.
fn canonical(path: PathBuf) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Deep enough for a capability, a Pack or a nested app; shallow enough to stay cheap on
/// every `report`.
const MAX_SCAN_DEPTH: usize = 8;

fn scan(dir: &Path, primary: &Path, deployment: Option<&Path>, out: &mut Vec<SecondaryTarget>, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if real == primary || Some(real.as_path()) == deployment {
            continue;
        }
        if name == "target" && path.join("CACHEDIR.TAG").is_file() {
            out.push(SecondaryTarget {
                path: path.display().to_string(),
                manifest: path
                    .parent()
                    .map(|p| p.join("Cargo.toml").display().to_string())
                    .unwrap_or_default(),
                bytes: Walk::default().size(&path),
            });
            continue;
        }
        scan(&path, primary, deployment, out, depth + 1);
    }
}

/// The `commit-hash:` line of a `rustc -vV` block, wherever it came from.
pub fn commit_hash(vv: &str) -> Option<String> {
    vv.lines()
        .find_map(|l| l.strip_prefix("commit-hash:"))
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

/// The rustc version block cargo cached in `target/.rustc_info.json`.
///
/// The file is cargo's own cache and its layout is not a stable interface, so this reads it
/// structurally rather than by key path: every value under `outputs` that carries a
/// `stdout` starting with `rustc `, of which there is one. A layout change makes this
/// return `None`, which reports as unknown rather than as a false mismatch.
pub fn recorded_rustc(info_json: &str) -> Option<String> {
    let doc: serde_json::Value = serde_json::from_str(info_json).ok()?;
    doc.get("outputs")?
        .as_object()?
        .values()
        .filter_map(|v| v.get("stdout")?.as_str())
        .find(|s| s.starts_with("rustc "))
        .map(str::to_string)
}

fn profile(dir: &Path, name: &str) -> Profile {
    let root = dir.join(name);
    if !root.is_dir() {
        return Profile {
            name: name.to_string(),
            present: false,
            bytes: 0,
            units: 0,
            buckets: Buckets {
                deps: 0,
                incremental: 0,
                build: 0,
                fingerprint: 0,
                binaries: 0,
                other: 0,
            },
        };
    }
    // One walk per profile, so a hard link is charged once across the whole profile. The
    // bucket order below is therefore also the attribution order: cargo hard-links the
    // finished binary from `deps/` up to the profile root, `deps` is walked first, so
    // `binaries` reports what is uniquely there — which is what deleting it would free.
    let mut walk = Walk::default();
    let deps = walk.size(&root.join("deps"));
    let incremental = walk.size(&root.join("incremental"));
    let build = walk.size(&root.join("build"));
    let fingerprint = walk.size(&root.join(".fingerprint"));
    let binaries: u64 = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.path().is_dir())
        .map(|e| walk.size(&e.path()))
        .sum();
    // Last, with the same walk: everything the buckets above already counted contributes
    // zero, so `other` is exactly the remainder and the six sum to the profile total.
    let other = walk.size(&root);
    let units = std::fs::read_dir(root.join(".fingerprint"))
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    Profile {
        name: name.to_string(),
        present: true,
        bytes: deps + incremental + build + fingerprint + binaries + other,
        units,
        buckets: Buckets {
            deps,
            incremental,
            build,
            fingerprint,
            binaries,
            other,
        },
    }
}

/// R6's verdict for a debug/release pair.
pub fn r6_verdict(debug: u64, release: u64) -> (Option<f64>, &'static str) {
    if release == 0 {
        return (None, "unknown");
    }
    let ratio = debug as f64 / release as f64;
    (
        Some(ratio),
        if ratio > R6_MAX_RATIO { "over" } else { "ok" },
    )
}

pub fn measure(repo_root: &Path) -> TargetReport {
    let (dir, source) = target_dir(repo_root);
    let profiles: Vec<Profile> = ["debug", "release"]
        .iter()
        .map(|name| profile(&dir, name))
        .collect();
    let get = |name: &str| {
        profiles
            .iter()
            .find(|p| p.name == name)
            .map_or(0, |p| p.bytes)
    };
    let (ratio, r6) = r6_verdict(get("debug"), get("release"));

    let recorded = std::fs::read_to_string(dir.join(".rustc_info.json"))
        .ok()
        .and_then(|json| recorded_rustc(&json))
        .and_then(|vv| commit_hash(&vv));
    let current = Command::new("rustc")
        .arg("-vV")
        .output()
        .ok()
        .and_then(|o| commit_hash(&String::from_utf8_lossy(&o.stdout)));
    let matches = match (&recorded, &current) {
        (Some(a), Some(b)) => a == b,
        // Not knowing is not a finding. Saying otherwise would make a fresh checkout, where
        // there is no `.rustc_info.json` yet, report a stale toolchain.
        _ => true,
    };
    let stale_candidate_bytes = profiles
        .iter()
        .map(|p| p.buckets.deps + p.buckets.fingerprint)
        .sum();
    let secondary = secondary_targets(repo_root, &dir);
    let secondary_bytes = secondary.iter().map(|s| s.bytes).sum();

    TargetReport {
        target_dir: dir.display().to_string(),
        source: source.to_string(),
        bytes: profiles.iter().map(|p| p.bytes).sum(),
        profiles,
        ratio,
        r6_max_ratio: R6_MAX_RATIO,
        r6: r6.to_string(),
        toolchain: Toolchain {
            recorded,
            current,
            matches,
            stale_candidate_bytes,
        },
        secondary,
        secondary_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VV: &str = concat!(
        "rustc 1.98.0 (88d9e12ae 2026-08-18)\n",
        "binary: rustc\n",
        "commit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea\n",
        "commit-date: 2026-08-18\n",
        "host: aarch64-apple-darwin\n",
        "release: 1.98.0\n",
        "LLVM version: 22.1.8\n",
    );

    #[test]
    fn the_commit_hash_is_read_from_a_rustc_version_block() {
        assert_eq!(
            commit_hash(VV).as_deref(),
            Some("88d9e12ae178fab0fb5cc050a94da85685d449ea")
        );
    }

    #[test]
    fn a_version_block_without_a_commit_hash_is_unknown_not_empty() {
        // A distribution-built rustc reports `commit-hash: unknown` or omits the line.
        assert_eq!(commit_hash("rustc 1.98.0\nbinary: rustc\n"), None);
        assert_eq!(commit_hash("commit-hash:   \n"), None);
    }

    #[test]
    fn the_recorded_rustc_is_found_by_shape_not_by_key_path() {
        // Shape copied from a real target/.rustc_info.json: opaque numeric keys, and a
        // second output that is a target-spec dump rather than a version block.
        let info = r#"{"rustc_fingerprint":11420747473765439537,"outputs":{
          "7971740275564407648":{"success":true,"stdout":"___\nlib___.rlib\npacked\n","stderr":""},
          "8622506321572056276":{"success":true,"stdout":"rustc 1.98.0 (88d9e12ae 2026-08-18)\ncommit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea\n","stderr":""}
        }}"#;
        assert_eq!(
            recorded_rustc(info).and_then(|vv| commit_hash(&vv)),
            Some("88d9e12ae178fab0fb5cc050a94da85685d449ea".to_string())
        );
    }

    #[test]
    fn an_unreadable_rustc_info_is_unknown_rather_than_a_mismatch() {
        assert_eq!(recorded_rustc("not json"), None);
        assert_eq!(recorded_rustc(r#"{"outputs":{}}"#), None);
    }

    #[test]
    fn r6_compares_debug_against_release_and_three_is_the_line() {
        assert_eq!(r6_verdict(3 * 1000, 1000).1, "ok");
        // 14.7× is what this workspace measured on 2026-08-29 before the
        // `[profile.dev.package."*"]` stanza in Cargo.toml; 1.6× is what it measured after.
        assert_eq!(r6_verdict(147 * 100, 1000).1, "over");
        assert_eq!(r6_verdict(16 * 100, 1000).1, "ok");
    }

    #[test]
    fn r6_with_no_release_build_is_unknown_not_exceeded() {
        // A checkout that has only ever run `cargo build` has no denominator. Reporting
        // that as a violation would fire on the one state where nothing is wrong.
        assert_eq!(r6_verdict(20 * 1000, 0), (None, "unknown"));
    }

    #[test]
    fn target_dir_follows_cargo_rather_than_the_repo_layout() {
        let repo = Path::new("/tmp/axon-fixture");
        // Read through the process environment, so this asserts the fallback only; the
        // CARGO_TARGET_DIR arm is exercised by every run of the test suite itself, which
        // cargo sets.
        if std::env::var_os("CARGO_TARGET_DIR").is_none() {
            assert_eq!(
                target_dir(repo),
                (PathBuf::from("/tmp/axon-fixture/target"), "repo")
            );
        }
    }

    #[test]
    fn an_absent_profile_measures_zero_and_says_it_is_absent() {
        let root = crate::testutil::tempdir("target-absent");
        let p = profile(&root, "release");
        assert!(!p.present);
        assert_eq!(p.bytes, 0);
    }

    #[test]
    fn the_buckets_sum_to_the_profile_total() {
        let root = crate::testutil::tempdir("target-buckets");
        let debug = root.join("debug");
        for bucket in ["deps", "incremental", "build", ".fingerprint", "unexpected"] {
            std::fs::create_dir_all(debug.join(bucket)).unwrap();
            std::fs::write(debug.join(bucket).join("f"), vec![0u8; 16 * 1024]).unwrap();
        }
        std::fs::write(debug.join("sjel-storage"), vec![0u8; 16 * 1024]).unwrap();

        let p = profile(&root, "debug");
        assert_eq!(p.units, 1, "one .fingerprint entry is one compilation unit");
        let b = &p.buckets;
        assert_eq!(
            b.deps + b.incremental + b.build + b.fingerprint + b.binaries + b.other,
            p.bytes
        );
        assert!(b.deps >= 16 * 1024, "deps: {}", b.deps);
        assert!(b.binaries >= 16 * 1024, "binaries: {}", b.binaries);
        // A directory cargo grows that this tool has no bucket for still lands somewhere.
        assert!(b.other >= 16 * 1024, "other: {}", b.other);
    }

    #[test]
    fn a_second_target_dir_inside_the_repo_is_found_and_the_primary_is_not() {
        let repo = crate::testutil::tempdir("secondary-target");
        let primary = repo.join("target");
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::write(primary.join("CACHEDIR.TAG"), b"").unwrap();
        std::fs::write(primary.join("blob"), vec![0u8; 1024]).unwrap();

        // The nested workspace whose target dir Xcode's build phase produces when
        // CARGO_TARGET_DIR is not in the environment.
        let nested = repo.join("dashboard").join("src-tauri");
        std::fs::create_dir_all(nested.join("target")).unwrap();
        std::fs::write(nested.join("Cargo.toml"), b"[workspace]\n").unwrap();
        std::fs::write(nested.join("target").join("CACHEDIR.TAG"), b"").unwrap();
        std::fs::write(nested.join("target").join("blob"), vec![0u8; 16 * 1024]).unwrap();

        // A directory that merely bears the name is not a target dir. Cargo's own
        // CACHEDIR.TAG is the marker, not the name, which is what keeps node_modules-style
        // names out of the answer.
        std::fs::create_dir_all(repo.join("looks-like").join("target")).unwrap();

        let found = secondary_targets(&repo, &primary);
        assert_eq!(found.len(), 1, "found: {found:?}");
        assert!(found[0].path.ends_with("dashboard/src-tauri/target"), "{}", found[0].path);
        assert!(
            found[0].manifest.ends_with("dashboard/src-tauri/Cargo.toml"),
            "{}",
            found[0].manifest
        );
        assert!(found[0].bytes >= 16 * 1024, "bytes: {}", found[0].bytes);

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_repo_with_only_the_primary_target_dir_reports_none() {
        let repo = crate::testutil::tempdir("secondary-target-none");
        let primary = repo.join("target");
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::write(primary.join("CACHEDIR.TAG"), b"").unwrap();
        // The intended state: CARGO_TARGET_DIR covers everything, so nothing is orphaned.
        assert!(secondary_targets(&repo, &primary).is_empty());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn the_checkouts_own_target_dir_is_never_reported_as_reclaimable() {
        let repo = crate::testutil::tempdir("deployment-target");
        // Where a workspace build puts the binaries the supervisor runs.
        let deployment = repo.join("target");
        std::fs::create_dir_all(deployment.join("release")).unwrap();
        std::fs::write(deployment.join("CACHEDIR.TAG"), b"").unwrap();
        std::fs::write(
            deployment.join("release").join("calendar-server"),
            vec![0u8; 4096],
        )
        .unwrap();

        // The nested workspace, which is the case this actually looks for.
        let nested = repo.join("dashboard").join("src-tauri");
        std::fs::create_dir_all(nested.join("target")).unwrap();
        std::fs::write(nested.join("target").join("CACHEDIR.TAG"), b"").unwrap();

        // cargo-hermetic points CARGO_TARGET_DIR somewhere outside the checkout, which is the
        // state in which the repo's own target/ would otherwise be mistaken for an escapee.
        // tools/cargo-hermetic refuses that directory for this very reason.
        let elsewhere = crate::testutil::tempdir("deployment-primary");
        let found = secondary_targets(&repo, &elsewhere);
        assert_eq!(found.len(), 1, "found: {found:?}");
        assert!(
            found[0].path.ends_with("dashboard/src-tauri/target"),
            "{}",
            found[0].path
        );

        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }
}

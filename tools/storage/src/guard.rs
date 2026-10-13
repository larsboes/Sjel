//! Every reason a path may not be removed, asked in one place.
//!
//! `apply` and `prune` each grew their own answer to "may I delete this", and they disagreed.
//! `apply` ran a class's reclaim verb without asking whether a process was using the path at
//! all; `prune` refused paths the policy owned, which `apply` existed to reclaim. One verb
//! means one place to ask, and this is it — resolution, scope, git, the policy, and the
//! live-process probe, in the order a reader would ask them.
//!
//! Two kinds of refusal, and only the second is negotiable:
//!
//! * **Structural** — the path cannot be resolved, or sits outside the checkout, or is
//!   tracked by git, or *is* the repo root. `--force` does not reach these. Each one means the
//!   tool does not know what it is being asked to remove, and a flag is the wrong answer to
//!   not knowing.
//! * **Judgement** — the policy protects it, the policy owns it and something other than that
//!   class is asking, or a live process has it open. These are the calls the policy and the
//!   machine have already made, so the reader is told the reason and `--force` is the way past
//!   when they disagree.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::inuse::{self, Usage};
use crate::policy::{expand_glob, is_applicable, Policy, StorageClass};

/// Paths the policy answers for, resolved from the overlay by the caller.
///
/// Split in two because one path means different things depending on who is asking. A
/// protection is absolute: nothing removes it. A path a *class* names is the policy's own
/// business — that class removing it is the policy acting, and anything else removing it is a
/// sweep over something it does not own. That distinction is what closed the 2026-10-13
/// contradiction, where the policy called `~/.cargo-target` report-only while `reclaim --repo --target`
/// offered to `cargo clean` all of it.
#[derive(Debug, Default, Clone)]
pub struct Authority {
    protected: Vec<(PathBuf, String)>,
    named: Vec<(PathBuf, String)>,
}

impl Authority {
    /// Translate the overlay into the two lists, each entry carrying the sentence that
    /// explains it.
    ///
    /// Built here rather than where the policy is read, because this is the only place that
    /// knows what the two lists mean — and because that meaning is the fix for the 2026-10-13
    /// contradiction, so it should not be duplicated at a call site.
    pub fn from_policy(policy: &Policy, home: &str) -> Self {
        let expand = |pattern: &str, why: &str| -> Vec<(PathBuf, String)> {
            expand_glob(pattern, home)
                .into_iter()
                .map(|path| (PathBuf::from(path), why.to_string()))
                .collect()
        };
        let mut protected: Vec<(PathBuf, String)> = Vec::new();
        for entry in &policy.protected {
            let why = format!("the policy protects it\n{}", entry.reason.trim());
            protected.extend(expand(&entry.path, &why));
        }
        let mut named: Vec<(PathBuf, String)> = Vec::new();
        for class in &policy.classes {
            // The remedy differs, and naming it is the point: a reader told only "refused"
            // will reach for `--force`, which is usually the answer the policy exists to
            // argue against.
            let why = match class.note.as_deref().map(str::trim) {
                Some(note) if !note.is_empty() => format!("{}\n{note}", class_verdict(class)),
                _ => class_verdict(class),
            };
            for pattern in &class.paths {
                named.extend(expand(pattern, &why));
            }
        }
        Authority::new(protected, named)
    }

    pub fn new(protected: Vec<(PathBuf, String)>, named: Vec<(PathBuf, String)>) -> Self {
        Self {
            protected: canonical(protected),
            named: canonical(named),
        }
    }

    /// Why a protection covers this candidate, or `None` when none does.
    fn protected_owns(&self, candidate: &Path) -> Option<&str> {
        owns(&self.protected, candidate)
    }

    /// Why a class names this candidate, or `None` when none does.
    fn named_owns(&self, candidate: &Path) -> Option<&str> {
        owns(&self.named, candidate)
    }
}

/// Paths are compared canonical, so a policy path written through a symlink still matches the
/// directory a removal would actually take. `/tmp` and `/private/tmp` are the pair on this
/// machine, and a comparison that missed it would refuse nothing while reading as a policy
/// that had nothing to say.
fn canonical(entries: Vec<(PathBuf, String)>) -> Vec<(PathBuf, String)> {
    entries
        .into_iter()
        .map(|(path, reason)| (path.canonicalize().unwrap_or(path), reason))
        .collect()
}

/// Containment, not identity, and it is not a convenience: a candidate that *contains* a named
/// path would delete it, which is the same act as naming it directly. A candidate *inside* one
/// is a different act — that is what keeps `reclaim --incremental` usable inside a report-only
/// target directory.
fn owns<'a>(entries: &'a [(PathBuf, String)], candidate: &Path) -> Option<&'a str> {
    let real = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    entries
        .iter()
        .find(|(path, _)| path.starts_with(&real))
        .map(|(_, reason)| reason.as_str())
}

/// Who is asking. The only thing that differs between the two candidate sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// The policy's own classes. They live wherever the machine keeps them, so the checkout's
    /// scope rules do not apply, and a path a class names is this caller acting on it.
    Policy,
    /// A node inside the checkout's build output. It must be inside the repo or the target
    /// dir, must not be tracked by git, and must defer to the policy.
    Checkout,
    /// The resolved Cargo target dir itself, or a secondary one. Inside the target dir by
    /// definition, and removed by `cargo clean` — which is cargo's own verb for exactly this
    /// directory — so the rule that the target dir is not itself a candidate does not apply.
    Target,
}

pub struct Guard<'a> {
    authority: &'a Authority,
    repo_root: PathBuf,
    target_dir: PathBuf,
    force: bool,
}

impl<'a> Guard<'a> {
    pub fn new(authority: &'a Authority, repo_root: &Path, target_dir: &Path, force: bool) -> Self {
        Self {
            authority,
            repo_root: repo_root.to_path_buf(),
            target_dir: target_dir.to_path_buf(),
            force,
        }
    }

    /// May this candidate be removed? `Ok` is the resolved path; `Err` is the sentence to
    /// print, in the same voice the reader gets from every other refusal.
    pub fn check(&self, candidate: &Path, caller: Caller) -> Result<PathBuf, String> {
        let real = candidate
            .canonicalize()
            .map_err(|e| format!("{}: {e}", candidate.display()))?;

        // Structural, and `--force` never reaches here.
        if caller != Caller::Policy {
            let repo = self
                .repo_root
                .canonicalize()
                .unwrap_or_else(|_| self.repo_root.clone());
            let target = self
                .target_dir
                .canonicalize()
                .unwrap_or_else(|_| self.target_dir.clone());
            if !(real.starts_with(&repo) || real.starts_with(&target)) {
                return Err(format!(
                    "{} resolves outside the repo ({}) and the target dir ({}) — refusing",
                    real.display(),
                    repo.display(),
                    target.display()
                ));
            }
            // The repo root is never a candidate. The target dir is one only through
            // `cargo clean`, which is `Caller::Target`; a recursive delete of it is not.
            if real == repo || (real == target && caller == Caller::Checkout) {
                return Err(format!("{} is the root itself — refusing", real.display()));
            }
            // The `--node-modules` list is derived from what git calls ignored, and this
            // re-asks per path so nothing tracked can be an argument. A path under an external
            // `CARGO_TARGET_DIR` is in no index at all — git exits non-zero, `git` returns
            // `None`, and "not tracked" is the correct answer.
            let tracked =
                git(&repo, &["ls-files", "--", &real.to_string_lossy()]).unwrap_or_default();
            if !tracked.trim().is_empty() {
                return Err(format!("{} is tracked by git — refusing", real.display()));
            }
        }

        // Judgement, and all three give way to `--force`.
        if !self.force {
            if let Some(reason) = self.authority.protected_owns(&real) {
                return Err(with_reason(
                    &real,
                    "protected by the policy",
                    Some(reason),
                    "pass --force to remove it anyway",
                ));
            }
            if caller != Caller::Policy {
                if let Some(reason) = self.authority.named_owns(&real) {
                    return Err(with_reason(
                        &real,
                        "the policy's to decide",
                        Some(reason),
                        "pass --force to remove it anyway",
                    ));
                }
            }
            if let Some(line) = self.holds(&real) {
                return Err(line);
            }
        }
        Ok(real)
    }

    /// The live-process question on its own, or `None` when the path is free.
    ///
    /// `--force` skips the question rather than answering it, which is what the flag is for:
    /// the probe is a snapshot, and the reader is the authority on their own machine. The
    /// process asking is subtracted from its own answer — `CARGO_TARGET_DIR` here is the shared
    /// target dir, so otherwise every run would refuse itself, which is not a reason.
    pub fn holds(&self, path: &Path) -> Option<String> {
        if self.force || !path.exists() {
            return None;
        }
        match inuse::usage(path) {
            Usage::Free => None,
            Usage::Held(holders) => {
                let holders: Vec<_> = holders
                    .into_iter()
                    .filter(|h| h.pid != std::process::id())
                    .collect();
                if holders.is_empty() {
                    return None;
                }
                let noun = if holders.len() == 1 {
                    "process"
                } else {
                    "processes"
                };
                let mut line =
                    format!("  refused  {} is held by {} live {noun}:", path.display(), holders.len());
                for h in &holders {
                    line.push_str(&format!(
                        "\n             pid {}  {}  {} {}",
                        h.pid, h.command, h.kind, h.file
                    ));
                }
                line.push_str("\n             stop it, or pass --force to remove it anyway");
                Some(line)
            }
            Usage::Unverifiable(why) => Some(format!(
                "  refused  {} — could not ask what is using it ({why}); pass --force to remove it anyway",
                path.display()
            )),
        }
    }
}

/// The house form for a refusal: what is not being done, why, and how to overrule it. The
/// reason keeps its own line breaks, because a policy note is written as a paragraph and
/// re-wrapping it turns a five-sentence justification into a wall nobody reads — including the
/// sentence saying what to do instead.
fn with_reason(path: &Path, verdict: &str, reason: Option<&str>, escape: &str) -> String {
    let mut line = format!("  refused  {} is {verdict}:", path.display());
    for part in reason
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        // Only trailing whitespace goes: a note lines its continuation up under the heading it
        // belongs to, and stripping the leading spaces would flatten that.
        line.push_str(&format!("\n             {}", part.trim_end()));
    }
    line.push_str(&format!("\n             {escape}"));
    line
}

/// What the policy says to do about a class a sweep was asked to remove.
fn class_verdict(class: &StorageClass) -> String {
    if is_applicable(class) {
        format!(
            "the policy class `{}` reclaims it — use `sjel storage reclaim`",
            class.name
        )
    } else {
        format!(
            "the policy class `{}` owns it and is report-only",
            class.name
        )
    }
}

/// `git`, or `None` when it could not answer. Shared with `checkout`, which reads the ignored
/// set the same way rather than keeping a second copy of this.
pub(crate) fn git(repo_root: &Path, args: &[&str]) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(protected: &[&str], named: &[&str]) -> Authority {
        Authority::new(
            protected
                .iter()
                .map(|p| (PathBuf::from(p), "the policy protects it".to_string()))
                .collect(),
            named
                .iter()
                .map(|p| (PathBuf::from(p), "a class names it".to_string()))
                .collect(),
        )
    }

    #[test]
    fn containment_is_the_rule_not_identity() {
        // `rm -rf /t` deletes `/t/release`, which is the thing the protection was written for.
        let a = authority(&["/t/release"], &[]);
        assert!(a.protected_owns(Path::new("/t")).is_some());
        // A subdirectory of the protected path does not contain it, so it stays removable —
        // which is what keeps `reclaim --incremental` usable inside a report-only target dir.
        assert!(a
            .protected_owns(Path::new("/t/debug/incremental"))
            .is_none());
        // A sibling sharing a prefix is not containment either.
        assert!(a.protected_owns(Path::new("/tb")).is_none());
    }

    #[test]
    fn only_a_sweep_defers_to_a_path_a_class_names() {
        let a = authority(&[], &["/t"]);
        // The class reclaiming its own path is the policy acting.
        assert!(a.named_owns(Path::new("/t")).is_some());
        assert!(a.named_owns(Path::new("/t/debug")).is_none());
    }

    #[test]
    fn a_protection_outranks_the_caller() {
        // Nothing removes a protection, including the policy's own class.
        let a = authority(&["/t/release"], &["/t"]);
        assert!(a.protected_owns(Path::new("/t")).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_checkout_is_refused_after_resolution() {
        // The failure this guards: `dashboard/node_modules` being a symlink to a shared store,
        // and the prefix check passing on the un-resolved path — which would delete the store.
        let root = crate::testutil::tempdir("guard-symlink");
        let repo = root.join("repo");
        let target = root.join("target");
        let outside = root.join("elsewhere");
        for d in [&repo, &target, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let link = repo.join("node_modules");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let a = Authority::default();
        let guard = Guard::new(&a, &repo, &target, false);
        assert!(guard
            .check(&link, Caller::Checkout)
            .unwrap_err()
            .contains("outside the repo"));
    }

    #[test]
    fn a_missing_path_is_refused_rather_than_removed_blind() {
        let root = crate::testutil::tempdir("guard-missing");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let a = Authority::default();
        let guard = Guard::new(&a, &repo, &repo.join("target"), false);
        let err = guard
            .check(&repo.join("gone"), Caller::Checkout)
            .unwrap_err();
        assert!(err.contains("gone"), "{err}");
    }

    #[test]
    fn a_path_outside_the_checkout_is_refused_for_a_sweep_and_allowed_for_a_class() {
        let root = crate::testutil::tempdir("guard-scope");
        let repo = root.join("repo");
        let target = root.join("target");
        let outside = root.join("elsewhere");
        for d in [&repo, &target, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let a = Authority::default();
        let guard = Guard::new(&a, &repo, &target, false);
        assert!(guard
            .check(&outside, Caller::Checkout)
            .unwrap_err()
            .contains("outside the repo"));
        // A policy class lives wherever the machine keeps it. This is the rule that let a
        // class name `~/.cargo-target` while a sweep over the checkout could not.
        assert!(guard.check(&outside, Caller::Policy).is_ok());
    }

    #[test]
    fn the_repo_root_is_never_itself_removable() {
        let root = crate::testutil::tempdir("guard-root");
        let repo = root.join("repo");
        let target = root.join("target");
        for d in [&repo, &target] {
            std::fs::create_dir_all(d).unwrap();
        }
        let a = Authority::default();
        let guard = Guard::new(&a, &repo, &target, false);
        assert!(guard
            .check(&repo, Caller::Checkout)
            .unwrap_err()
            .contains("root itself"));
        // The target dir is a candidate only through `cargo clean`, which is `Caller::Target`;
        // a recursive delete of it is not.
        assert!(guard
            .check(&target, Caller::Checkout)
            .unwrap_err()
            .contains("root itself"));
        assert!(guard.check(&target, Caller::Target).is_ok());
    }

    #[test]
    fn force_never_reaches_the_structural_refusals() {
        let root = crate::testutil::tempdir("guard-force");
        let repo = root.join("repo");
        let target = root.join("target");
        let outside = root.join("elsewhere");
        for d in [&repo, &target, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let a = authority(&["/t"], &["/t"]);
        let guard = Guard::new(&a, &repo, &target, true);
        // Out of scope stays out of scope: a flag is the wrong answer to not knowing what is
        // being removed.
        assert!(guard.check(&outside, Caller::Checkout).is_err());
        assert!(guard.check(&repo, Caller::Checkout).is_err());
        // The judgement calls do give way.
        let inside = repo.join("node_modules");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(guard.check(&inside, Caller::Checkout).is_ok());
    }
}

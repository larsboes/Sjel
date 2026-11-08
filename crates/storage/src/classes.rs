//! Policy class measurement and guarded reclaim.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::guard::{Caller, Guard};
use crate::inuse::{self, Usage};
use crate::measure::{fmt_bytes, size_of, Walk};
use crate::policy::{expand_glob, is_applicable, reclaim_argv, Policy, StorageClass};
use crate::reclaim::Outcome;

/// One class, with the paths its globs actually resolved to and what they hold.
pub struct Measured {
    pub class: StorageClass,
    pub paths: Vec<String>,
    pub bytes: u64,
    /// False when `du` met a directory it could not read, so `bytes` is a lower bound.
    pub complete: bool,
    pub error: String,
}

/// The walk `report` prints and `reclaim` acts on. One function, so the two cannot disagree.
pub fn measure(policy: &Policy, home: &str) -> Vec<Measured> {
    measure_selected(policy, home, None)
}

/// Measure only the requested class; unrelated paths are not walked.
pub fn measure_selected(policy: &Policy, home: &str, only: Option<&str>) -> Vec<Measured> {
    let mut measured: Vec<Measured> = policy
        .classes
        .iter()
        .filter(|class| only.is_none_or(|name| class.name == name))
        .map(|class| {
            let paths: Vec<String> = class
                .paths
                .iter()
                .flat_map(|p| expand_glob(p, home))
                .collect();
            let sizing = size_of(&paths);
            Measured {
                bytes: sizing.bytes,
                complete: sizing.complete,
                error: sizing.error,
                class: class.clone(),
                paths,
            }
        })
        .collect();
    measured.sort_by_key(|m| std::cmp::Reverse(m.bytes));
    measured
}

/// What reclaim intends to do about one class, or why it will not.
pub struct Step {
    pub class: StorageClass,
    pub paths: Vec<String>,
    pub bytes: u64,
    /// Present when the class is reclaimable in principle but will not act now, in the words
    /// the reader gets. Printed rather than dropped: a scheduled run that silently did nothing
    /// is indistinguishable from one that had nothing to do.
    pub skip: Option<String>,
}

/// The classes this run would act on, largest first, with a skip line for each one a gate held
/// back.
///
/// `scheduled` is the unattended rule and the only reason `cap_gb` exists. A class with neither
/// bound is left alone there — not because it is unsafe, but because the policy never said what
/// size was acceptable for it, and guessing on a schedule is how a cache you wanted gets
/// deleted at 3am.
pub fn steps(measured: &[Measured], scheduled: bool, only: Option<&str>) -> Vec<Step> {
    measured
        .iter()
        .filter(|m| only.is_none_or(|name| name == m.class.name))
        .filter(|m| is_applicable(&m.class))
        .map(|m| {
            let skip = if m.bytes == 0 {
                Some("nothing measured — no paths resolved".to_string())
            } else if scheduled && !m.class.bounded() {
                Some("not bounded by a cap or an age, so a schedule may not act on it".to_string())
            } else if m.class.cap_gb.is_some() && !m.class.over_cap(m.bytes) {
                Some(format!(
                    "under its cap of {} GB",
                    trim_zero(m.class.cap_gb.unwrap_or_default())
                ))
            } else {
                None
            };
            Step {
                class: m.class.clone(),
                paths: m.paths.clone(),
                bytes: m.bytes,
                skip,
            }
        })
        .collect()
}

/// `50` rather than `50.0`, because a policy that says `cap_gb = 50` should read back as `50`.
fn trim_zero(gb: f64) -> String {
    if gb.fract() == 0.0 {
        format!("{gb:.0}")
    } else {
        format!("{gb}")
    }
}

/// The line naming a class on a plan, matching the checkout half's columns.
fn step_line(step: &Step) -> String {
    match step.class.older_than_days {
        Some(days) => format!(
            "  {:>9}  {} — entries older than {days}d",
            fmt_bytes(step.bytes),
            step.class.name
        ),
        None => format!(
            "  {:>9}  {} — {}",
            fmt_bytes(step.bytes),
            step.class.name,
            step.class
                .reclaim
                .as_deref()
                .unwrap_or("no reclaim command")
        ),
    }
}

/// How long a class held only by a running build is waited for, and the interval between looks.
///
/// A debug build of this workspace is minutes rather than hours, so this covers the lock a build
/// has left rather than a whole build. Giving up here is not a lost night: the schedule comes
/// round again with nothing held.
const BUILD_RELEASE_WAIT: Duration = Duration::from_secs(600);
const BUILD_RELEASE_POLL: Duration = Duration::from_secs(15);

/// The commands that release the directory on their own, when the build they are in finishes.
///
/// `cargo` and `rustc` are the build itself; `clippy-driver` and `rustdoc` are runs of it.
/// `rust-analyzer` is deliberately absent: it holds `debug/` for as long as the editor is open,
/// which is the shape of a server rather than of a build.
const TRANSIENT_HOLDERS: [&str; 4] = ["cargo", "rustc", "clippy-driver", "rustdoc"];

/// Whether a hold is one a job may wait out.
///
/// Every holder has to be the build itself. One server in the list is enough to make the whole
/// wait pointless, and the answer is the reason this is a separate function from the sleeping:
/// it is the rule, and the rule is what a fixture can pin down.
fn worth_waiting_for(holders: &[inuse::Holder]) -> bool {
    !holders.is_empty()
        && holders
            .iter()
            .all(|h| TRANSIENT_HOLDERS.contains(&h.command.as_str()))
}

/// `guard.check`, retried for as long as a build — and only a build — is what holds the path.
///
/// The refusal this exists for is on record: `/tmp/sjel-cargo-trim-schedule.log` at 00:54 on
/// 2026-10-29, where `~/.cargo-target/debug` was held by a live `cargo` and `rustc`, the run
/// refused both the class and its nested incremental path, and the cache stayed at 1.6× its cap
/// until a person reclaimed by hand. The guard was right; the job was given one moment to act
/// and no other.
///
/// A holder that will not lift is not worth waiting for. A `*-server` exec'ing its own image out
/// of `debug/` — which this machine has done — holds the directory until an operator stops it,
/// so a wait entered on that would sit here until its own deadline to report what the first
/// answer already said.
///
/// A dry run never waits: it reports the plan the next real run would follow, and a report that
/// slept would be a report nobody runs.
fn check_settling(guard: &Guard, path: &Path, dry_run: bool) -> Result<PathBuf, String> {
    let first = guard.check(path, Caller::Policy);
    if dry_run || first.is_ok() {
        return first;
    }
    let err = first.err().unwrap_or_default();
    let deadline = SystemTime::now() + BUILD_RELEASE_WAIT;
    loop {
        match inuse::usage(path) {
            // Every holder lifts on its own, so the wait is worth having.
            Usage::Held(holders) if worth_waiting_for(&holders) => {}
            // Nothing holds it any more. The guard refused on a snapshot that has stopped being
            // true, so ask it again rather than trusting the older answer.
            Usage::Free => return guard.check(path, Caller::Policy),
            // A holder that will not lift, or a question that cannot be asked. Both are
            // answers, and neither is a reason to sleep.
            _ => return Err(err),
        }
        if SystemTime::now() >= deadline {
            return Err(format!(
                "{err}\n             waited {}s for the build holding it and it did not finish",
                BUILD_RELEASE_WAIT.as_secs()
            ));
        }
        std::thread::sleep(BUILD_RELEASE_POLL);
    }
}

/// Act on the policy's classes. Every refusal is decided before anything is removed, so a dry
/// run reports the plan that would really run.
pub fn run(
    measured: &[Measured],
    guard: &Guard,
    scheduled: bool,
    only: Option<&str>,
    dry_run: bool,
) -> Outcome {
    let mut lines = Vec::new();
    let mut refused = 0;
    let mut skipped = 0;
    let mut bytes = 0u64;

    for step in steps(measured, scheduled, only) {
        if let Some(why) = &step.skip {
            lines.push(format!("  skipped  {} — {why}", step.class.name));
            skipped += 1;
            continue;
        }
        if step.class.older_than_days.is_some() {
            let aged = act_on_aged(&step.class, &step.paths, guard, dry_run);
            bytes += aged.bytes;
            refused += aged.refused;
            skipped += aged.skipped;
            lines.extend(aged.lines);
            continue;
        }
        // The guard runs on the class's own paths before anything is selected from them: a
        // directory a build is writing into is refused as a whole rather than per entry, which
        // is both cheaper and the conservative answer.
        let mut allowed: Vec<String> = Vec::new();
        let mut held = false;
        for path in &step.paths {
            // A class's path can be absent before the guard ever sees it, and that is nothing
            // to do rather than a refusal. Both ways it happens are normal: a sibling class in
            // the same plan removed its parent — `rust-debug-cache` holds `~/.cargo-target/debug`
            // and `rust-incremental` names `debug/incremental` inside it, the nesting the policy
            // declares on purpose — and this machine never built for one of the targets a path
            // globs. `canonicalize` resolves neither, so the guard answers `not found`, and
            // counting that as a refusal made a run that had freed 73.9 GB exit 1, which the
            // doctor reads as a producer that failed.
            //
            // `symlink_metadata`, not `exists`: a dangling symlink is still an entry on disk,
            // so it reaches the guard, because refusing what it cannot resolve is the guard's
            // job. Only a path that is really gone is skipped here.
            if std::fs::symlink_metadata(path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            match check_settling(guard, Path::new(path), dry_run) {
                Ok(real) => allowed.push(real.to_string_lossy().into_owned()),
                Err(why) => {
                    // Prefixed like every other line: a bare refusal reads as one of the plan's
                    // own lines, which is how four of them behind a 73.9 GB reclaim read as
                    // unexplained noise rather than as the reason the run exited 1.
                    lines.push(format!("  held     {why}"));
                    held = true;
                }
            }
        }
        if held {
            refused += 1;
            continue;
        }
        if allowed.is_empty() {
            continue;
        }
        let Some(argv) = reclaim_argv(&step.class, &allowed) else {
            continue;
        };
        match run_argv(&argv, dry_run) {
            Ok(()) => {
                bytes += step.bytes;
                lines.push(format!("  {}", argv.join(" ")));
                lines.push(format!("             {}", step_line(&step).trim()));
            }
            Err(why) => {
                // A refusal frees nothing, but a *partial* removal is not a refusal and must not
                // be reported as one. `rm -rf` exits 1 on `Directory not empty` after deleting the
                // bulk of a tree, which is exactly what anything re-creating entries mid-delete
                // produces — a build, or Spotlight indexing what is being removed. Measuring the
                // class again is what separates the two: on 2026-10-29 a run reported 6.4 GB freed
                // while 88 GB had actually gone, because the class was dropped from the total
                // whole.
                let after = size_of(&allowed).bytes;
                let freed = step.bytes.saturating_sub(after);
                if freed == 0 {
                    // A removal that did not happen is not freed bytes, so it never reaches the
                    // total — the same rule the checkout half follows.
                    lines.push(format!("  failed   {why}"));
                } else {
                    bytes += freed;
                    lines.push(format!(
                        "  {:>9}  {} — removed before the command failed, the rest is still there",
                        fmt_bytes(freed),
                        step.class.name
                    ));
                    lines.push(format!("  failed   {why}"));
                }
                refused += 1;
            }
        }
    }

    Outcome {
        lines,
        refused,
        skipped,
        bytes,
        nested_bytes: 0,
    }
}

/// Reclaim idle aged children independently, preserving active siblings.
fn act_on_aged(
    class: &StorageClass,
    paths: &[String],
    guard: &Guard<'_>,
    dry_run: bool,
) -> Outcome {
    let days = class.older_than_days.unwrap_or_default();
    let mut outcome = Outcome::default();
    for path in paths {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            _ => {
                outcome.refused += 1;
                outcome.lines.push(format!(
                    "  refused  {path} — aged root must be a non-symlink directory"
                ));
                continue;
            }
        }
        let entries = match aged_entries(Path::new(path), days, class.entry_pattern.as_deref()) {
            Ok(entries) => entries,
            Err(error) => {
                outcome.refused += 1;
                outcome.lines.push(format!("  refused  {path} — {error}"));
                continue;
            }
        };
        if entries.is_empty() {
            outcome.skipped += 1;
            outcome
                .lines
                .push(format!("  skipped  {path} — no entries older than {days}d"));
        }
        for (entry, held) in entries {
            let before = match std::fs::symlink_metadata(&entry) {
                Ok(metadata) => metadata,
                Err(error) => {
                    outcome.refused += 1;
                    outcome
                        .lines
                        .push(format!("  refused  {} — {error}", entry.display()));
                    continue;
                }
            };
            if !old_enough(&before, days) {
                outcome.skipped += 1;
                outcome.lines.push(format!(
                    "  skipped  {} — entry is no longer aged",
                    entry.display()
                ));
                continue;
            }
            match guard.idle_entry(&entry) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    outcome.skipped += 1;
                    outcome.lines.push(format!(
                        "  skipped  {} — held by a live process",
                        entry.display()
                    ));
                    continue;
                }
                Err(reason) => {
                    outcome.refused += 1;
                    outcome.lines.push(format!("  refused  {reason}"));
                    continue;
                }
            }
            if !std::fs::symlink_metadata(&entry).is_ok_and(|after| same_entry(&before, &after)) {
                outcome.refused += 1;
                outcome.lines.push(format!(
                    "  refused  {} — entry changed during the process probe",
                    entry.display()
                ));
                continue;
            }
            let result = if dry_run {
                Ok(())
            } else {
                std::fs::remove_dir_all(&entry).or_else(|_| std::fs::remove_file(&entry))
            };
            match result {
                Ok(()) => {
                    outcome.bytes += held;
                    outcome.lines.push(format!(
                        "  {:>9}  {} — {}",
                        fmt_bytes(held),
                        class.name,
                        entry.display()
                    ));
                }
                Err(error) => {
                    let after = size_of(&[entry.to_string_lossy().into_owned()]).bytes;
                    outcome.bytes += held.saturating_sub(after);
                    outcome.refused += 1;
                    outcome
                        .lines
                        .push(format!("  failed   {} — {error}", entry.display()));
                }
            }
        }
    }
    outcome
}

fn same_entry(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return false;
        }
    }
    !after.is_symlink()
        && before.file_type() == after.file_type()
        && before.modified().ok() == after.modified().ok()
        && before.created().ok() == after.created().ok()
        && before.len() == after.len()
}

fn old_enough(metadata: &std::fs::Metadata, days: u32) -> bool {
    SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(days) * 86_400))
        .is_some_and(|cutoff| {
            !metadata.is_symlink()
                && aged_times(metadata.modified().ok(), metadata.created().ok(), cutoff)
        })
}

fn aged_times(
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    cutoff: SystemTime,
) -> bool {
    modified.is_some_and(|modified| modified < cutoff)
        && !created.is_some_and(|created| created >= cutoff)
}

/// Select non-symlink children whose modification and creation times exceed the age.
fn aged_entries(
    path: &Path,
    days: u32,
    pattern: Option<&str>,
) -> Result<Vec<(PathBuf, u64)>, std::io::Error> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if pattern.is_some_and(|pattern| {
            !entry
                .file_name()
                .to_str()
                .is_some_and(|name| crate::policy::segment_matches(pattern, name))
        }) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !old_enough(&metadata, days) {
            continue;
        }
        out.push((entry.path(), Walk::default().size(&entry.path())));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn run_argv(argv: &[String], dry_run: bool) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = crate::measure::first_line(&String::from_utf8_lossy(&out.stderr));
    Err(format!(
        "{} exited {} ({err})",
        argv.join(" "),
        out.status.code().unwrap_or(-1)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::Authority;
    use crate::policy::is_applicable;

    fn class(toml_text: &str) -> StorageClass {
        toml::from_str(toml_text).unwrap()
    }

    fn measured(class: StorageClass, bytes: u64) -> Measured {
        Measured {
            class,
            paths: vec!["/t".to_string()],
            bytes,
            complete: true,
            error: String::new(),
        }
    }

    #[test]
    fn a_report_only_class_is_never_a_step() {
        let m = vec![measured(
            class("name = \"c\"\npaths = [\"/t\"]\nreclaim = \"rm -rf\"\n"),
            1024,
        )];
        assert!(steps(&m, false, None).is_empty());
    }

    #[test]
    fn a_cap_holds_a_class_back_under_it_and_releases_it_over() {
        let capped = || {
            class(
                "name = \"c\"\npaths = [\"/t\"]\nreclaim = \"rm -rf\"\napply = true\ncap_gb = 50\n",
            )
        };
        let under = steps(&[measured(capped(), 5 * crate::policy::GB)], false, None);
        assert!(under[0].skip.is_some(), "under the cap is a skip");
        let over = steps(&[measured(capped(), 51 * crate::policy::GB)], false, None);
        assert!(over[0].skip.is_none(), "over the cap acts");
    }

    #[test]
    fn a_schedule_acts_only_on_bounded_classes() {
        let unbounded = measured(
            class("name = \"c\"\npaths = [\"/t\"]\nreclaim = \"rm -rf\"\napply = true\n"),
            crate::policy::GB,
        );
        let by_age = measured(
            class("name = \"a\"\npaths = [\"/t\"]\napply = true\nolder_than_days = 3\n"),
            crate::policy::GB,
        );
        let both = vec![unbounded, by_age];

        // Asked for by a person, both are in scope.
        assert_eq!(steps(&both, false, None).len(), 2);
        // On a schedule, only the one whose own declaration says when to stop.
        let scheduled = steps(&both, true, None);
        assert!(scheduled[0].skip.is_some());
        assert!(scheduled[1].skip.is_none());
    }

    #[test]
    fn older_than_days_is_itself_a_reclaim_so_it_needs_no_command() {
        // A class that removes aged entries does not also need `reclaim`: the age is the
        // mechanism. `apply = true` with neither is still a policy bug.
        let age_only = class("name = \"c\"\npaths = [\"/t\"]\napply = true\nolder_than_days = 3\n");
        assert!(is_applicable(&age_only));
        let neither = class("name = \"c\"\npaths = [\"/t\"]\napply = true\n");
        assert!(!is_applicable(&neither));
    }

    #[test]
    fn only_selects_one_class_by_name() {
        let a = measured(
            class("name = \"a\"\npaths = [\"/t\"]\nreclaim = \"rm -rf\"\napply = true\n"),
            1024,
        );
        let b = measured(
            class("name = \"b\"\npaths = [\"/t\"]\nreclaim = \"rm -rf\"\napply = true\n"),
            1024,
        );
        let picked = steps(&[a, b], false, Some("b"));
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].class.name, "b");
    }

    #[test]
    fn a_class_path_that_is_already_gone_is_not_a_refusal() {
        // The measured failure (2026-10-17, the daily trim): `rust-debug-cache` holds
        // `~/.cargo-target/debug` and `rust-incremental` names `debug/incremental` inside it.
        // Removing the parent left the child's paths unresolvable, the guard answered `not
        // found`, and the single refusal made a run that had freed 73.9 GB exit 1.
        let root = crate::testutil::tempdir("classes-gone");
        let gone = root.join("debug").join("incremental");
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::write(gone.join("entry"), vec![0u8; 4096]).unwrap();
        let a = Authority::default();
        let guard = Guard::new(&a, &root, &root, false);
        let m = vec![Measured {
            class: class("name = \"c\"\npaths = [\"/t\"]\napply = true\nolder_than_days = 3\n"),
            paths: vec![gone.to_string_lossy().into_owned()],
            // As `measure` found it, before the sibling class removed the parent.
            bytes: 4096,
            complete: true,
            error: String::new(),
        }];
        // `rust-debug-cache` removing `debug` in the same plan, which is what happened.
        std::fs::remove_dir_all(root.join("debug")).unwrap();

        let out = run(&m, &guard, false, None, false);
        assert_eq!(out.refused, 0, "{:?}", out.lines);
        assert_eq!(out.skipped, 0, "nothing was held back: {:?}", out.lines);

        // The same class with its path present still acts, so the skip above is not a blanket
        // silence: an age of zero keeps every entry whose mtime is in the past.
        let live = root.join("live");
        std::fs::create_dir_all(live.join("entry")).unwrap();
        let m = vec![Measured {
            class: class("name = \"c\"\npaths = [\"/t\"]\napply = true\nolder_than_days = 0\n"),
            paths: vec![live.to_string_lossy().into_owned()],
            bytes: 4096,
            complete: true,
            error: String::new(),
        }];

        let out = run(&m, &guard, false, None, false);
        assert_eq!(out.refused, 0, "{:?}", out.lines);
        assert_eq!(out.skipped, 0, "{:?}", out.lines);
        assert!(!live.join("entry").exists(), "{:?}", out.lines);
    }

    #[test]
    fn aged_entries_takes_only_what_is_older_than_the_age() {
        let root = crate::testutil::tempdir("classes-aged");
        let fresh = root.join("fresh");
        let stale = root.join("stale");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(fresh.join("f"), vec![0u8; 4096]).unwrap();
        std::fs::write(stale.join("f"), vec![0u8; 4096]).unwrap();

        // Zero days keeps nothing: everything was modified before "now".
        assert_eq!(aged_entries(&root, 0, None).unwrap().len(), 2);
        assert_eq!(aged_entries(&root, 0, Some("sta*")).unwrap().len(), 1);
        // A century of age keeps everything, so the filter is on mtime and not on the walk.
        assert!(aged_entries(&root, 36_500, None).unwrap().is_empty());
    }

    #[test]
    fn aged_reclaim_preserves_active_children_and_removes_idle_siblings() {
        let root = crate::testutil::tempdir("aged-siblings");
        let active = root.join("active");
        let idle = root.join("idle");
        std::fs::create_dir(&active).unwrap();
        std::fs::create_dir(&idle).unwrap();
        std::fs::write(idle.join("data"), vec![0u8; 4096]).unwrap();
        let mut child = Command::new("sleep")
            .arg("30")
            .current_dir(&active)
            .spawn()
            .unwrap();
        let authority = Authority::default();
        let guard = Guard::new(&authority, &root, &root, false);
        let measured = vec![Measured {
            class: class(
                "name = \"aged\"\npaths = [\"/unused\"]\napply = true\nolder_than_days = 0\n",
            ),
            paths: vec![root.to_string_lossy().into_owned()],
            bytes: 4096,
            complete: true,
            error: String::new(),
        }];
        let dry = run(&measured, &guard, true, None, true);
        let dry_preserved_both = active.exists() && idle.join("data").exists();
        let applied = run(&measured, &guard, true, None, false);
        let active_preserved = active.exists();
        let idle_removed = !idle.exists();
        let _ = child.kill();
        let _ = child.wait();
        assert!(dry_preserved_both, "{:?}", dry.lines);
        assert!(active_preserved && idle_removed, "{:?}", applied.lines);
        assert_eq!(applied.refused, 0, "{:?}", applied.lines);
        assert_eq!(applied.skipped, 1, "{:?}", applied.lines);
        assert!(applied.bytes >= 4096);
    }

    #[test]
    fn aged_reclaim_does_not_follow_root_or_child_symlinks() {
        let root = crate::testutil::tempdir("aged-links");
        let outside = root.join("outside");
        let entries = root.join("entries");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&entries).unwrap();
        std::fs::write(outside.join("keep"), "keep").unwrap();
        std::os::unix::fs::symlink(&outside, entries.join("link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("root-link")).unwrap();
        let authority = Authority::default();
        let guard = Guard::new(&authority, &root, &root, false);
        let aged = class("name = \"aged\"\npaths = []\napply = true\nolder_than_days = 0\n");
        let result = act_on_aged(
            &aged,
            &[
                entries.to_string_lossy().into_owned(),
                root.join("root-link").to_string_lossy().into_owned(),
            ],
            &guard,
            false,
        );
        assert!(outside.join("keep").exists());
        assert!(entries.join("link").is_symlink());
        assert_eq!(result.refused, 1);
    }

    #[test]
    fn a_replaced_entry_is_not_the_entry_that_was_probed() {
        let root = crate::testutil::tempdir("aged-replaced");
        let path = root.join("entry");
        std::fs::write(&path, "old").unwrap();
        let before = path.metadata().unwrap();
        std::fs::rename(&path, root.join("previous")).unwrap();
        std::fs::write(&path, "new").unwrap();
        assert!(!same_entry(&before, &path.metadata().unwrap()));
    }

    #[test]
    fn a_fresh_copy_with_an_old_modification_time_is_not_aged() {
        let now = SystemTime::now();
        let cutoff = now - Duration::from_secs(2 * 86_400);
        let old = now - Duration::from_secs(3 * 86_400);
        assert!(!aged_times(Some(old), Some(now), cutoff));
        assert!(!aged_times(None, Some(old), cutoff));
        assert!(aged_times(Some(old), None, cutoff));
    }

    #[test]
    fn an_unknown_class_does_not_succeed_silently() {
        let root = crate::testutil::tempdir("unknown-class");
        let result = crate::reclaim::run(
            Some(&Policy::default()),
            "",
            &root,
            &crate::reclaim::Args {
                only: Some("missing".into()),
                dry_run: true,
                ..Default::default()
            },
        );
        assert!(result.unwrap_err().contains("unknown storage class"));
    }

    #[test]
    fn a_removal_that_fails_partway_reports_what_it_freed() {
        // The measured failure (2026-10-29): `rm -rf ~/.cargo-target/debug` exited 1 on
        // `Directory not empty` after deleting 82 GB of an 88 GB class, and the run reported
        // `6.4 GB freed, 1 refused` — the class was dropped from the total whole, so the reader
        // was told nothing had happened.
        //
        // A directory nothing may be removed from reproduces the shape. The mode is used rather
        // than `chflags`, which would be macOS-only: this suite runs on the Linux runner too.
        use std::os::unix::fs::PermissionsExt;

        let root = crate::testutil::tempdir("classes-partial");
        let full = root.join("full");
        let blocked = full.join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(full.join("removable"), vec![0u8; 64 * 1024]).unwrap();
        std::fs::write(blocked.join("entry"), vec![0u8; 64 * 1024]).unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o555))
            .expect("make the subdirectory unremovable");

        let a = Authority::default();
        let guard = Guard::new(&a, &root, &root, false);
        let before = crate::measure::size_of(&[full.to_string_lossy().into_owned()]).bytes;
        let paths = vec![full.to_string_lossy().into_owned()];
        let m = vec![Measured {
            class: class("name = \"c\"\npaths = [\"/t\"]\napply = true\nreclaim = \"rm -rf\"\n"),
            paths: paths.clone(),
            bytes: before,
            complete: true,
            error: String::new(),
        }];

        let out = run(&m, &guard, false, None, false);

        // The command did fail, and that is still a refusal for the exit code.
        assert_eq!(out.refused, 1, "{:?}", out.lines);
        // ...but the bytes it did free are reported rather than discarded.
        assert!(out.bytes > 0, "freed bytes were discarded: {:?}", out.lines);
        assert!(
            out.bytes < before,
            "more was freed than existed: {:?}",
            out.lines
        );
        // The half that survived is what was measured, so it is really still there.
        assert!(
            blocked.join("entry").exists(),
            "the blocked file went anyway"
        );
        assert!(!full.join("removable").exists(), "the free file survived");
        assert!(
            out.lines
                .iter()
                .any(|l| l.contains("removed before the command failed")),
            "the partial removal was not named: {:?}",
            out.lines
        );

        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755)).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    fn holder(command: &str) -> inuse::Holder {
        inuse::Holder {
            pid: 1,
            command: command.to_string(),
            kind: "txt".to_string(),
            file: String::new(),
        }
    }

    #[test]
    fn a_hold_is_worth_waiting_for_only_when_every_holder_is_the_build() {
        // The pair on record from the run this wait exists for: `~/.cargo-target/debug` held by
        // a live `cargo` and the `rustc` it started, at 00:53 on 2026-10-29.
        assert!(worth_waiting_for(&[holder("cargo"), holder("rustc")]));
        assert!(worth_waiting_for(&[holder("clippy-driver")]));

        // A server exec'ing its own image out of `debug/` releases it when an operator stops
        // it. This machine has done exactly that, and a job that slept for it would report the
        // first answer ten minutes later.
        assert!(!worth_waiting_for(&[holder("projects-server")]));
        assert!(!worth_waiting_for(&[
            holder("cargo"),
            holder("projects-server")
        ]));

        // `rust-analyzer` reads as a build and is not one: it holds the directory for as long
        // as the editor is open, which is the shape of a server.
        assert!(!worth_waiting_for(&[holder("rust-analyzer")]));

        // Nothing holding it is not a hold to wait out; the caller asks the guard again.
        assert!(!worth_waiting_for(&[]));
    }
}

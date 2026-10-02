//! Remove a destination path the source no longer holds — declared, quarantined, and never on its
//! own authority.
//!
//! `mirror` reports these paths and removes nothing. This is the separate act, and it exists
//! because "the mirror is faithful" and "the mirror is tidy" are different claims: a mirror can be
//! complete and still carry every path a supersede or a rename left behind. On 2026-10-02 `INTENSO`
//! held 1,532 such paths, 71.7 GiB, whose bytes had all moved to a new name on `Extreme`.
//!
//! The guard is the one `supersede` already uses, and it is the whole reason this is not `rm`: a
//! path is removed **only** when its digest is present at a source path that is on disk right now.
//! Bytes that exist nowhere else are reported and left alone, so a destination holding something the
//! source lost keeps holding it. That case is not hypothetical — it is the entire value of having a
//! second copy, and a tool that reclaimed it would be destroying the backup it was pointed at.
//!
//! **The destination is walked, not read from the ledger.** Planning used to iterate
//! `media_locations` rows for the destination volume, which made the verb blind to any tree the
//! ledger did not describe — and silent about it. Measured 2026-10-02: `INTENSO/Inbox` (193 GB) and
//! both `_quarantine` batches (33.7 GB) each answered `candidates: 0, complete: true, issues: []`
//! while holding files, because the Inbox was never a library root and a quarantined path has its
//! row dropped by design. The walk costs nothing where the ledger is current — a destination file
//! whose recorded size and mtime still match is answered from that row without being read — and
//! reads what the ledger cannot answer, which is the rule `mirror` already uses. `from_ledger` and
//! `hashed` are reported so a run that had to read a large unindexed tree says so.
//!
//! Two further rules, both inherited rather than invented:
//!
//! * **The approved list is the gate.** `--apply` acts on a TSV an operator has read, never on the
//!   set the tool just computed for itself. Without `--list`, `--apply` is refused.
//! * **Removal is quarantine, never deletion.** The file is renamed into a quarantine tree and the
//!   journal is written *before* the move, so the run reverses by swapping two paths. Reclaiming the
//!   space is a separate declared step, exactly as it is for `supersede`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::{Ledger, Result};
use crate::{files, hash, stamp};

/// A path under a root, refusing traversal, an absolute escape, and a symlink at any component.
fn safe_relpath(rel: &str) -> Result<PathBuf> {
    let candidate = Path::new(rel);
    if candidate.is_absolute() {
        return Err(format!("absolute path in list: {rel}").into());
    }
    let mut out = PathBuf::new();
    for part in candidate.components() {
        use std::path::Component;
        match part {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            _ => return Err(format!("path escapes its root: {rel}").into()),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("empty path in list: {rel}").into());
    }
    Ok(out)
}

/// Whether the ledger's claim about a path still describes what is on disk.
fn current(root: &Path, relpath: &str, size: i64, mtime_ns: i64) -> bool {
    stamp(&root.join(relpath)).is_ok_and(|s| s == (size, mtime_ns))
}

struct Candidate {
    relpath: String,
    digest: String,
    size: i64,
    survivor: String,
}

pub struct ReclaimOptions<'a> {
    pub from: &'a Path,
    pub from_uuid: &'a str,
    pub to: &'a Path,
    pub to_uuid: &'a str,
    pub paths: &'a [String],
    pub exclude: &'a [String],
    pub list: Option<&'a Path>,
    pub quarantine: &'a Path,
    pub journal: &'a Path,
    pub apply: bool,
}

#[derive(Debug, Serialize)]
pub struct ReclaimReport {
    pub applied: bool,
    pub complete: bool,
    pub from: String,
    pub to: String,
    pub quarantine: String,
    pub journal: String,
    /// Destination paths inside the scope that the source does not hold.
    pub candidates: usize,
    /// Candidates whose bytes survive at a source path present on disk, so removing them is safe.
    pub survivable: usize,
    /// Candidates whose bytes exist nowhere else. Never removed, and never silently.
    pub unique: usize,
    /// Bytes a run would free, or did.
    pub bytes: u64,
    pub verified: usize,
    pub quarantined: usize,
    /// Rows this journal already moved. Re-running an approved list is safe.
    pub already_quarantined: usize,
    pub refused: usize,
    pub quarantined_paths: Vec<String>,
    /// A bounded sample of the paths held back because their bytes are unique.
    pub kept_unique: Vec<String>,
    /// How the plan learned each destination digest: `ledger` when every file was answered from a
    /// current row, `disk` when none were, `mixed` otherwise.
    pub planned_from: &'static str,
    /// Destinations answered from a ledger row without being read.
    pub from_ledger: usize,
    /// Destinations read because the ledger had no current row for them.
    pub hashed: usize,
    /// Destinations that could not be read. Never candidates, and `complete` is false when any.
    pub unreadable: usize,
    /// A bounded sample of the unreadable paths.
    pub unreadable_paths: Vec<String>,
    pub issues: Vec<String>,
}

fn scope(rel: &str, paths: &[String], exclude: &[String]) -> bool {
    let under = |rel: &str, prefix: &str| {
        let prefix = prefix.trim_matches('/');
        !prefix.is_empty() && (rel == prefix || rel.starts_with(&format!("{prefix}/")))
    };
    if exclude.iter().any(|p| under(rel, p)) {
        return false;
    }
    paths.is_empty() || paths.iter().any(|p| under(rel, p))
}

/// Where a digest lives on the source: relpath, and the size and mtime the ledger recorded for it.
type SourceLocations = BTreeMap<String, Vec<(String, i64, i64)>>;

/// What the source volume holds, by path and by digest.
struct SourceState {
    paths: BTreeSet<String>,
    by_digest: SourceLocations,
}

/// What the source still holds, by path and by digest — from the ledger, read only where it cannot
/// answer. Nothing here hashes the source: `index` is the verb that records digests, and a reclaim
/// that re-hashed a 511 GiB library to plan a cleanup would be slower than the cleanup is worth.
fn source_state(ledger: &Ledger, opts: &ReclaimOptions<'_>) -> Result<SourceState> {
    let paths: BTreeSet<String> = files(opts.from)?.into_iter().map(|(rel, _)| rel).collect();
    let mut by_digest: SourceLocations = BTreeMap::new();
    for row in ledger.locations(opts.from_uuid)? {
        if !paths.contains(&row.relpath) {
            continue;
        }
        by_digest
            .entry(row.digest)
            .or_default()
            .push((row.relpath, row.size, row.mtime_ns));
    }
    Ok(SourceState { paths, by_digest })
}

/// The candidate set and the evidence for how each digest was learned. `from_ledger` and `hashed`
/// are the cost of the plan, reported so a large unindexed destination cannot look like a cheap run.
struct Plan {
    candidates: Vec<Candidate>,
    unique: usize,
    from_ledger: usize,
    hashed: usize,
    unreadable: usize,
    unreadable_paths: Vec<String>,
    kept_unique: Vec<String>,
    issues: Vec<String>,
}

fn plan(ledger: &Ledger, opts: &ReclaimOptions<'_>) -> Result<Plan> {
    let source = source_state(ledger, opts)?;
    let mut recorded: BTreeMap<String, (String, i64, i64)> = BTreeMap::new();
    for row in ledger.locations(opts.to_uuid)? {
        recorded.insert(row.relpath, (row.digest, row.size, row.mtime_ns));
    }

    let mut plan = Plan {
        candidates: Vec::new(),
        unique: 0,
        from_ledger: 0,
        hashed: 0,
        unreadable: 0,
        unreadable_paths: Vec::new(),
        kept_unique: Vec::new(),
        issues: Vec::new(),
    };
    // The destination is what the walk finds, never what the ledger says should be there. A tree
    // with no rows is the case that used to return zero candidates and call itself complete.
    for (relpath, path) in files(opts.to)? {
        if source.paths.contains(&relpath) || !scope(&relpath, opts.paths, opts.exclude) {
            continue;
        }
        let observed = stamp(&path)?;
        let digest = match recorded.get(&relpath) {
            // A recorded digest is trusted only while the row still describes the file, the same
            // rule `mirror` uses. A stale row is a statement about a past read.
            Some((digest, size, mtime_ns)) if (*size, *mtime_ns) == observed => {
                plan.from_ledger += 1;
                digest.clone()
            }
            _ => match hash(&path) {
                Ok(digest) => {
                    plan.hashed += 1;
                    digest
                }
                // Unreadable is not a candidate and not a silence: the file is left alone and the
                // run reports itself incomplete, so a partial plan cannot pass as a full one.
                Err(error) => {
                    plan.unreadable += 1;
                    plan.issues.push(format!("{relpath}: unreadable ({error})"));
                    if plan.unreadable_paths.len() < 20 {
                        plan.unreadable_paths.push(relpath);
                    }
                    continue;
                }
            },
        };
        // The survivor has to be on disk, not merely recorded. A row is a statement about a past
        // read; a removal may not rest on one.
        let survivor = source.by_digest.get(&digest).and_then(|rows| {
            rows.iter()
                .find(|(rel, size, mtime_ns)| current(opts.from, rel, *size, *mtime_ns))
                .map(|(rel, _, _)| rel.clone())
        });
        match survivor {
            Some(survivor) => plan.candidates.push(Candidate {
                relpath,
                digest,
                size: observed.0,
                survivor,
            }),
            None => {
                plan.unique += 1;
                if plan.kept_unique.len() < 20 {
                    plan.kept_unique.push(relpath);
                }
            }
        }
    }
    plan.candidates.sort_by(|a, b| a.relpath.cmp(&b.relpath));
    Ok(plan)
}

fn write_list(path: &Path, candidates: &[Candidate]) -> Result<()> {
    // Written whole and renamed into place: a half-written list that looks approved is worse than
    // no list at all.
    let temp = PathBuf::from(format!("{}.part", path.display()));
    let mut file = File::create(&temp)?;
    writeln!(file, "digest\tsize\treclaimable\tsurvivor")?;
    for candidate in candidates {
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            candidate.digest, candidate.size, candidate.relpath, candidate.survivor
        )?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)?;
    Ok(())
}

fn read_list(path: &Path) -> Result<Vec<(String, i64, String, String)>> {
    let file = File::open(path)?;
    let mut rows = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 {
            if !line.starts_with("digest\tsize\treclaimable\tsurvivor") {
                return Err("reclaim list must start with its header row".into());
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(
                format!("reclaim list row {} has {} fields", index + 1, fields.len()).into(),
            );
        }
        rows.push((
            fields[0].to_owned(),
            fields[1].parse()?,
            fields[2].to_owned(),
            fields[3].to_owned(),
        ));
    }
    if rows.is_empty() {
        return Err("reclaim list has no rows".into());
    }
    Ok(rows)
}

fn journalled(journal: &Path) -> Result<BTreeSet<String>> {
    let Ok(file) = File::open(journal) else {
        return Ok(BTreeSet::new());
    };
    let mut seen = BTreeSet::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if let Some(from) = fields.get(1) {
            seen.insert((*from).to_owned());
        }
    }
    Ok(seen)
}

pub fn reclaim(ledger: &Ledger, opts: &ReclaimOptions<'_>) -> Result<ReclaimReport> {
    let from = opts.from.canonicalize()?;
    let to = opts.to.canonicalize()?;
    let mut report = ReclaimReport {
        applied: opts.apply,
        complete: true,
        from: from.display().to_string(),
        to: to.display().to_string(),
        quarantine: opts.quarantine.display().to_string(),
        journal: opts.journal.display().to_string(),
        candidates: 0,
        survivable: 0,
        unique: 0,
        bytes: 0,
        verified: 0,
        quarantined: 0,
        already_quarantined: 0,
        refused: 0,
        quarantined_paths: Vec::new(),
        kept_unique: Vec::new(),
        planned_from: "ledger",
        from_ledger: 0,
        hashed: 0,
        unreadable: 0,
        unreadable_paths: Vec::new(),
        issues: Vec::new(),
    };

    if opts.apply && opts.list.is_none() {
        return Err(
            "--apply needs --list: the set this tool computes for itself is a proposal, and only a list an operator has read is an authorisation"
                .into(),
        );
    }

    if let Some(list) = opts.list {
        if opts.apply {
            let rows = read_list(list)?;
            let already = journalled(opts.journal)?;
            let stamp_dir = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("system clock is before the epoch: {e}"))?
                .as_secs()
                .to_string();
            let source = source_state(ledger, opts)?;
            report.candidates = rows.len();
            for (digest, size, relpath, survivor) in rows {
                let rel = safe_relpath(&relpath)?;
                if already.contains(&relpath) {
                    report.already_quarantined += 1;
                    report.quarantined_paths.push(relpath);
                    continue;
                }
                let path = to.join(&rel);
                // Re-read the row's own file immediately before moving it. An index is a record of
                // a past read, and a removal may not rest on one.
                match hash(&path) {
                    Ok(actual) if actual == digest => report.verified += 1,
                    Ok(actual) => {
                        report.refused += 1;
                        report
                            .issues
                            .push(format!("{relpath}: bytes are {actual}, list says {digest}"));
                        continue;
                    }
                    Err(error) => {
                        report.refused += 1;
                        report
                            .issues
                            .push(format!("{relpath}: unreadable before removal ({error})"));
                        continue;
                    }
                }
                // The survivor is re-checked here rather than trusted from the planning pass. The
                // list may be days old, and the source may have changed since it was written.
                let survives = source.by_digest.get(&digest).is_some_and(|rows| {
                    rows.iter()
                        .any(|(rel, size, mtime_ns)| current(&from, rel, *size, *mtime_ns))
                }) && !survivor.is_empty();
                if !survives {
                    report.refused += 1;
                    report.issues.push(format!(
                        "{relpath}: no surviving copy at {survivor}; left in place"
                    ));
                    continue;
                }
                let target = opts.quarantine.join(&stamp_dir).join(&rel);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Some(parent) = opts.journal.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut journal = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(opts.journal)?;
                if journal.metadata()?.len() == 0 {
                    writeln!(journal, "at\tfrom\tto\tsize\tdigest\tkind")?;
                }
                writeln!(
                    journal,
                    "{stamp_dir}\t{relpath}\t{}\t{size}\t{digest}\treclaimed",
                    target.display()
                )?;
                journal.sync_data()?;
                drop(journal);
                fs::rename(&path, &target)?;
                report.quarantined += 1;
                report.bytes += u64::try_from(size)?;
                report.quarantined_paths.push(relpath);
            }
            report.survivable = report.verified;
            return Ok(report);
        }
    }

    let planned = plan(ledger, opts)?;
    report.candidates = planned.candidates.len();
    report.survivable = planned.candidates.len();
    report.unique = planned.unique;
    report.kept_unique = planned.kept_unique;
    report.from_ledger = planned.from_ledger;
    report.hashed = planned.hashed;
    report.unreadable = planned.unreadable;
    report.unreadable_paths = planned.unreadable_paths;
    report.planned_from = if planned.hashed == 0 {
        "ledger"
    } else if planned.from_ledger == 0 {
        "disk"
    } else {
        "mixed"
    };
    // An unreadable destination file makes the plan partial, and a partial plan is not a complete
    // answer about a tree. Nothing is removed on its account; the exit status says so instead.
    report.complete = planned.unreadable == 0;
    report.issues = planned.issues;
    report.bytes = planned
        .candidates
        .iter()
        .map(|c| u64::try_from(c.size).unwrap_or(0))
        .sum();
    if let Some(list) = opts.list {
        write_list(list, &planned.candidates)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Location;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("reclaim-test-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("src")).unwrap();
            fs::create_dir_all(root.join("dst")).unwrap();
            Self(root)
        }
        fn write(&self, side: &str, rel: &str, body: &str) {
            let path = self.0.join(side).join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        fn record(&self, ledger: &Ledger, uuid: &str, side: &str, rel: &str) {
            let path = self.0.join(side).join(rel);
            let (size, mtime_ns) = stamp(&path).unwrap();
            ledger
                .record(&Location {
                    uuid: uuid.into(),
                    relpath: rel.into(),
                    digest: hash(&path).unwrap(),
                    size,
                    mtime_ns,
                })
                .unwrap();
        }
        fn ledger(&self) -> Ledger {
            let ledger = Ledger::open(&self.0.join("test.db")).unwrap();
            ledger.register("SRC", "src").unwrap();
            ledger.register("DST", "dst").unwrap();
            ledger
        }
        /// The fixture's own paths, leaked so the borrowed options can outlive the locals. A test
        /// that outlives its own process is not a leak worth avoiding.
        fn opts(&self, list: Option<&Path>, apply: bool) -> ReclaimOptions<'static> {
            let leaked = |p: PathBuf| -> &'static Path { Box::leak(p.into_boxed_path()) };
            ReclaimOptions {
                from: leaked(self.0.join("src")),
                from_uuid: "SRC",
                to: leaked(self.0.join("dst")),
                to_uuid: "DST",
                paths: &[],
                exclude: &[],
                list: list.map(|p| leaked(p.to_path_buf())),
                quarantine: leaked(self.0.join("quarantine")),
                journal: leaked(self.0.join("journal.tsv")),
                apply,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_stale_path_whose_bytes_survive_is_reclaimable() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        let report = reclaim(&ledger, &f.opts(None, false)).unwrap();
        assert_eq!(report.candidates, 1);
        assert_eq!(report.unique, 0);
    }

    #[test]
    fn bytes_that_exist_nowhere_else_are_kept() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "only/x.jpg", "unique bytes");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "only/x.jpg");
        let report = reclaim(&ledger, &f.opts(None, false)).unwrap();
        assert_eq!(report.candidates, 0);
        assert_eq!(report.unique, 1);
        assert_eq!(report.kept_unique, vec!["only/x.jpg".to_string()]);
    }

    #[test]
    fn apply_without_a_list_is_refused() {
        let f = Fixture::new();
        let ledger = f.ledger();
        let error = reclaim(&ledger, &f.opts(None, true))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("--apply needs --list"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_dry_run_quarantines_nothing() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        let list = f.0.join("list.tsv");
        let report = reclaim(&ledger, &f.opts(Some(&list), false)).unwrap();
        assert_eq!(report.quarantined, 0);
        assert!(f.0.join("dst/old/x.jpg").exists(), "a dry run moved a file");
        assert!(list.exists(), "the list was not written");
        let body = fs::read_to_string(&list).unwrap();
        assert!(body.starts_with("digest\tsize\treclaimable\tsurvivor"));
    }

    #[test]
    fn an_approved_list_quarantines_and_journals() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        let list = f.0.join("list.tsv");
        reclaim(&ledger, &f.opts(Some(&list), false)).unwrap();
        let report = reclaim(&ledger, &f.opts(Some(&list), true)).unwrap();
        assert_eq!(report.quarantined, 1);
        assert_eq!(report.refused, 0);
        assert!(!f.0.join("dst/old/x.jpg").exists(), "the file stayed");
        let body = fs::read_to_string(f.0.join("journal.tsv")).unwrap();
        assert!(body.contains("old/x.jpg"), "the journal has no row");
    }

    #[test]
    fn a_row_whose_bytes_changed_is_refused() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        let list = f.0.join("list.tsv");
        reclaim(&ledger, &f.opts(Some(&list), false)).unwrap();
        // Same length, different bytes: the list no longer describes the disk.
        f.write("dst", "old/x.jpg", "CORRUPT");
        let report = reclaim(&ledger, &f.opts(Some(&list), true)).unwrap();
        assert_eq!(report.quarantined, 0);
        assert_eq!(report.refused, 1);
        assert!(f.0.join("dst/old/x.jpg").exists());
    }

    #[test]
    fn re_running_an_applied_list_is_safe() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        let list = f.0.join("list.tsv");
        reclaim(&ledger, &f.opts(Some(&list), false)).unwrap();
        reclaim(&ledger, &f.opts(Some(&list), true)).unwrap();
        let again = reclaim(&ledger, &f.opts(Some(&list), true)).unwrap();
        assert_eq!(again.quarantined, 0);
        assert_eq!(again.already_quarantined, 1);
    }

    /// The measured defect: a destination volume with ledger rows, holding a tree the ledger does
    /// not describe. `INTENSO/Inbox` and both `_quarantine` batches each answered `candidates: 0,
    /// complete: true` here, and 226 GB looked like nothing to do.
    #[test]
    fn a_tree_the_ledger_does_not_describe_is_read_from_disk() {
        let f = Fixture::new();
        f.write("src", "lib/x.jpg", "payload");
        f.write("dst", "lib/x.jpg", "payload");
        f.write("dst", "quarantine/leftover.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "lib/x.jpg");
        // The destination volume has rows — the shared path — and still does not describe the tree.
        f.record(&ledger, "DST", "dst", "lib/x.jpg");
        let report = reclaim(&ledger, &f.opts(None, false)).unwrap();
        assert_eq!(report.candidates, 1, "{report:?}");
        assert!(report.kept_unique.is_empty());
        assert_eq!(report.hashed, 1);
        assert_eq!(report.from_ledger, 0);
        assert_eq!(report.planned_from, "disk");
        assert!(report.complete);
    }

    #[test]
    fn a_stale_destination_row_is_read_rather_than_trusted() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        f.write("dst", "old/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        f.record(&ledger, "DST", "dst", "old/x.jpg");
        // Same path, different bytes and length: the row now describes a past read, and a removal
        // resting on it would remove a file whose bytes survive nowhere.
        f.write("dst", "old/x.jpg", "different bytes entirely");
        let report = reclaim(&ledger, &f.opts(None, false)).unwrap();
        assert_eq!(report.candidates, 0);
        assert_eq!(report.unique, 1);
        assert_eq!(report.hashed, 1, "the stale row was trusted");
    }

    #[test]
    fn a_destination_with_nothing_in_it_is_still_complete() {
        let f = Fixture::new();
        f.write("src", "new/x.jpg", "payload");
        let ledger = f.ledger();
        f.record(&ledger, "SRC", "src", "new/x.jpg");
        let report = reclaim(&ledger, &f.opts(None, false)).unwrap();
        assert_eq!(report.candidates, 0);
        assert_eq!(report.unique, 0);
        assert_eq!(report.unreadable, 0);
        assert!(report.complete, "an empty destination is not a failure");
    }
}

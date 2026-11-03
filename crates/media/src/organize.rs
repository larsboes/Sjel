//! Rename-only organization apply — the execution half of the human-reviewed preview.
//!
//! It performs no hashing, opens no ledger, copies no bytes and deletes nothing. Every move is a
//! same-volume rename, so the whole operation is instantaneous and reversible by swapping the two
//! paths of a journal row. That is why this is a separate verb from `ingest`: ingest *writes new
//! bytes* into the library and can therefore be wrong about content; this verb only changes where
//! an existing collection lives.
//!
//! Two refusals are deliberately different in kind, and the difference was measured rather than
//! assumed:
//!
//! * A **conflict** — destination taken, an ancestor that is a symlink or a non-directory, or a
//!   move that would cross devices and therefore copy — means the plan is wrong. Nothing moves at
//!   all, and the report names every conflict.
//! * A **settledness** refusal means one collection is not ready while the rest are. That
//!   collection is skipped and the run continues.
//!
//! The second rule exists because of 2026-09-30: the source tree was being written while the
//! preview read it, and the preview's own before/after check caught the change. A tree still being
//! written must not be reorganized, and a single busy collection must not hold the others hostage.
//!
//! No preview can authorize a move. `execution.file_moves_authorized: true` in the draft is the
//! operator's authorization, and this verb refuses to run without it — including for a dry run,
//! because an execution plan emitted from an unauthorized draft invites somebody to act on it.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::preview::{self, Draft, Resolved};
use crate::store::Result;

#[derive(Debug, Serialize)]
pub struct MoveReport {
    pub collection: String,
    pub origin: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub file_count: usize,
    pub bytes: u64,
    pub newest_mtime_seconds: i64,
    /// `would-move`, `moved`, `skipped`, `conflict` or `failed`.
    pub outcome: &'static str,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OrganizeReport {
    pub applied: bool,
    pub moves_authorized: bool,
    pub complete: bool,
    pub settled_for_seconds: i64,
    pub source_root: PathBuf,
    pub archive_root: PathBuf,
    pub moves: Vec<MoveReport>,
    pub moved: usize,
    pub skipped: usize,
    pub conflicts: usize,
    /// Mappings this journal already applied: the source is gone and the destination exists.
    pub already_moved: usize,
    /// Empty means every collection the draft places; otherwise the named subset a pilot ran on.
    pub only: Vec<String>,
    pub journal: Option<PathBuf>,
    pub issues: Vec<String>,
    pub limitations: Vec<&'static str>,
}

const LIMITATIONS: [&str; 5] = [
    "Only whole collections move, and only by rename within one volume; no file is copied, hashed, dated or deleted.",
    "A move is not a duplicate verdict. The ledger and the index are untouched, so library-internal duplicates are neither created nor removed by this verb.",
    "Correcting a move means swapping that journal row's two paths back; nothing else in the journal is implied.",
    "The destination path is reserved by checking it does not exist, which is not an atomic reservation against a concurrent writer.",
    "A mapping is reported as already applied only when this journal records the move and the destination is present; the filesystem alone is never read that way.",
];

impl OrganizeReport {
    fn fail(&mut self, issue: String) {
        self.complete = false;
        self.issues.push(issue);
    }
}

struct Inventory {
    file_count: usize,
    bytes: u64,
    newest_mtime_seconds: i64,
}

#[cfg(unix)]
fn mtime_seconds(meta: &fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
}

#[cfg(unix)]
fn device_of(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(unix))]
fn mtime_seconds(_meta: &fs::Metadata) -> i64 {
    i64::MIN
}

#[cfg(not(unix))]
fn device_of(_meta: &fs::Metadata) -> u64 {
    u64::MAX
}

/// Count regular files and find the newest write, refusing a symlink anywhere inside the tree. A
/// symlink is not a collection member we can reason about, and following one would let a move
/// escape the source root.
fn inventory(root: &Path) -> Result<Inventory> {
    let mut total = Inventory {
        file_count: 0,
        bytes: 0,
        newest_mtime_seconds: 0,
    };
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.file_type().is_symlink() {
            return Err(format!("symlink inside collection: {}", entry.path().display()).into());
        }
        if meta.is_dir() {
            let nested = inventory(&entry.path())?;
            total.file_count += nested.file_count;
            total.bytes += nested.bytes;
            total.newest_mtime_seconds =
                total.newest_mtime_seconds.max(nested.newest_mtime_seconds);
        } else if meta.is_file() {
            total.file_count += 1;
            total.bytes += meta.len();
            total.newest_mtime_seconds = total.newest_mtime_seconds.max(mtime_seconds(&meta));
        }
    }
    Ok(total)
}

/// A destination ancestor this verb is allowed to create: a declared category, or a four-digit
/// year. Anything else must already exist, so a mistyped destination fails instead of silently
/// growing a tree nobody declared.
fn creatable_ancestor(segment: &str, categories: &BTreeSet<&str>) -> bool {
    categories.contains(segment)
        || (segment.len() == 4 && segment.bytes().all(|b| b.is_ascii_digit()))
}

/// Resolve the destination's parent, creating only `creatable_ancestor` components. With
/// `create: false` it reports whether the parent exists without touching the filesystem.
fn resolve_parent(
    archive: &Path,
    destination: &str,
    categories: &BTreeSet<&str>,
    create: bool,
) -> Result<PathBuf> {
    let segments: Vec<&str> = destination.split('/').collect();
    let mut parent = archive.to_path_buf();
    for segment in segments.iter().take(segments.len().saturating_sub(1)) {
        parent.push(segment);
        match fs::symlink_metadata(&parent) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err(format!("not a real directory: {}", parent.display()).into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !creatable_ancestor(segment, categories) {
                    return Err(format!(
                        "missing ancestor that is not a declared category or year: {}",
                        parent.display()
                    )
                    .into());
                }
                if create {
                    fs::create_dir(&parent)?;
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(parent)
}

/// The deepest existing ancestor of `path`. The destination parent usually does not exist yet, and
/// the device comparison must not depend on creating it first.
fn deepest_existing(path: &Path) -> Result<PathBuf> {
    let mut current = path;
    loop {
        if fs::symlink_metadata(current).is_ok() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(format!("no existing ancestor for {}", path.display()).into()),
        }
    }
}

fn unix_now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs() as i64)
}

/// Append the move journal. It is append-only on purpose: a re-run adds rows rather than rewriting
/// history, and the reversal of a row is the same row with the paths exchanged.
/// The `collection -> destination` pairs an existing journal already applied.
///
/// Re-running a plan must not read as a collision. A mapping whose source is gone and whose
/// destination exists is only treated as applied when this journal says so — the filesystem alone
/// cannot tell "already moved" from "somebody put something there", and guessing in that direction
/// is how a plan stops describing the disk.
fn applied_moves(journal: &Path) -> Result<Vec<(String, String)>> {
    let Ok(file) = fs::File::open(journal) else {
        return Ok(Vec::new());
    };
    let mut applied = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 5 {
            applied.push((fields[1].to_owned(), fields[4].to_owned()));
        }
    }
    Ok(applied)
}

fn append_journal(path: &Path, report: &OrganizeReport, now: i64) -> Result<()> {
    let fresh = !path.exists();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    if fresh {
        writeln!(
            file,
            "applied_at\tcollection\torigin\tfrom\tto\tfile_count\tbytes\tnewest_mtime_seconds"
        )?;
    }
    for entry in report.moves.iter().filter(|m| m.outcome == "moved") {
        writeln!(
            file,
            "{now}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            entry.collection,
            entry.origin,
            entry.from.display(),
            entry.to.display(),
            entry.file_count,
            entry.bytes,
            entry.newest_mtime_seconds
        )?;
    }
    file.sync_all()?;
    Ok(())
}

/// The organizing entry point. `apply: false` is a dry run that performs every check but renames
/// nothing; `--apply` also requires a journal path, because a move that is not written down is a
/// move that cannot be undone.
pub fn organize(
    structure: &Path,
    apply: bool,
    settled_for: i64,
    journal: Option<&Path>,
    only: &[String],
) -> Result<OrganizeReport> {
    if apply && journal.is_none() {
        return Err("--apply requires --journal PATH so every move is recorded".into());
    }
    let draft = Draft::parse_authorized(&fs::read_to_string(structure)?)?;
    let (source, archive) = preview::validated_roots(&draft, true)?;
    for root in [&source, &archive] {
        let actual = crate::volume_uuid(root)?;
        if !actual.eq_ignore_ascii_case(&draft.expected_volume_uuid) {
            return Err(format!("volume UUID mismatch for {}", root.display()).into());
        }
    }
    run(&draft, &source, &archive, apply, settled_for, journal, only)
}

/// The plan-and-move core, separated from mount identity so a fixture can exercise it without a
/// registered volume. Every check that decides whether a file moves lives here.
fn run(
    draft: &Draft,
    source: &Path,
    archive: &Path,
    apply: bool,
    settled_for: i64,
    journal: Option<&Path>,
    only: &[String],
) -> Result<OrganizeReport> {
    if apply && journal.is_none() {
        return Err("--apply requires --journal PATH so every move is recorded".into());
    }
    let source = source.to_path_buf();
    let archive = archive.to_path_buf();
    let categories = preview::category_names(draft);
    let resolved = preview::resolved_destinations(draft)?;
    // A pilot names its collections. A name that matches nothing is refused rather than silently
    // doing less than the operator asked — a typo must not read as "nothing to do".
    if !only.is_empty() {
        for name in only {
            if !resolved.iter().any(|entry| &entry.collection == name) {
                return Err(format!("--only {name} matches no collection the draft places").into());
            }
        }
    }
    let resolved: Vec<Resolved> = resolved
        .into_iter()
        .filter(|entry| only.is_empty() || only.iter().any(|name| name == &entry.collection))
        .collect();
    let now = unix_now()?;
    let mut report = OrganizeReport {
        applied: apply,
        moves_authorized: true,
        complete: true,
        settled_for_seconds: settled_for,
        source_root: source.clone(),
        archive_root: archive.clone(),
        moves: Vec::new(),
        moved: 0,
        skipped: 0,
        conflicts: 0,
        already_moved: 0,
        only: only.to_vec(),
        journal: journal.map(Path::to_path_buf),
        issues: Vec::new(),
        limitations: LIMITATIONS.to_vec(),
    };
    // (row index, destination relative) for the entries that passed every check.
    // What this journal already applied, so re-running a plan is idempotent rather than a collision.
    let applied = match journal {
        Some(path) => applied_moves(path)?,
        None => Vec::new(),
    };
    // (row index, destination relative) for the entries that passed every check.
    let mut ready: Vec<(usize, String)> = Vec::new();
    for entry in resolved {
        let from = source.join(&entry.collection);
        let Resolved {
            collection,
            destination,
            origin,
        } = entry;
        let mut row = MoveReport {
            collection,
            origin: origin.to_owned(),
            from: from.clone(),
            to: archive.join(&destination),
            file_count: 0,
            bytes: 0,
            newest_mtime_seconds: 0,
            outcome: "conflict",
            reason: None,
        };
        let conflict = |row: &mut MoveReport, reason: String| {
            row.outcome = "conflict";
            row.reason = Some(reason);
        };
        if let Err(error) = preview::real_directory(&from) {
            // Already applied? Only the journal may say so, and only when the destination is there.
            let recorded = applied
                .iter()
                .any(|(name, to)| name == &row.collection && Path::new(to) == row.to);
            if recorded && fs::symlink_metadata(&row.to).is_ok() {
                row.outcome = "already-moved";
                row.reason = Some(
                    "source is gone and the destination exists, and this journal records the move"
                        .into(),
                );
                report.already_moved += 1;
            } else {
                conflict(&mut row, error.to_string());
                report.conflicts += 1;
            }
            report.moves.push(row);
            continue;
        }
        match inventory(&from) {
            Ok(counted) => {
                row.file_count = counted.file_count;
                row.bytes = counted.bytes;
                row.newest_mtime_seconds = counted.newest_mtime_seconds;
            }
            Err(error) => {
                conflict(&mut row, error.to_string());
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
        }
        if row.file_count == 0 {
            row.outcome = "skipped";
            row.reason = Some("collection holds no regular files".into());
            report.skipped += 1;
            report.moves.push(row);
            continue;
        }
        match fs::symlink_metadata(&row.to) {
            Ok(_) => {
                conflict(
                    &mut row,
                    "destination already exists; nothing is ever overwritten".into(),
                );
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                conflict(&mut row, error.to_string());
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
        }
        let parent = match resolve_parent(&archive, &destination, &categories, false) {
            Ok(parent) => parent,
            Err(error) => {
                conflict(&mut row, error.to_string());
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
        };
        // A rename cannot cross devices. If the two roots are on different volumes this verb must
        // refuse rather than silently degrade into a copy of a size nobody approved. The parent
        // usually does not exist yet, so compare against its deepest existing ancestor.
        let same_device = deepest_existing(&parent).and_then(|probe| {
            let from_device = device_of(&fs::symlink_metadata(&from)?);
            Ok(device_of(&fs::symlink_metadata(&probe)?) == from_device)
        });
        match same_device {
            Ok(true) => {}
            Ok(false) => {
                conflict(
                    &mut row,
                    "source and archive are on different devices; a rename would become a copy"
                        .into(),
                );
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
            Err(error) => {
                conflict(&mut row, error.to_string());
                report.conflicts += 1;
                report.moves.push(row);
                continue;
            }
        }
        let age = now - row.newest_mtime_seconds;
        if age < settled_for {
            row.outcome = "skipped";
            row.reason = Some(format!(
                "newest file is {age}s old and the quiet window is {settled_for}s"
            ));
            report.skipped += 1;
            report.moves.push(row);
            continue;
        }
        ready.push((report.moves.len(), destination));
        row.outcome = "would-move";
        report.moves.push(row);
    }
    if report.conflicts > 0 {
        // Fail closed: a conflict anywhere means the plan does not describe the filesystem, so no
        // part of it is executed. A dry run is not complete either, because its plan cannot be
        // executed as written and reporting it as complete would invite exactly that.
        if apply {
            for (index, _) in &ready {
                report.moves[*index].outcome = "refused";
                report.moves[*index].reason = Some(
                    "not applied: another mapping conflicts, so the plan is not trustworthy".into(),
                );
            }
            report.fail(format!(
                "{} conflicting mappings; nothing was moved",
                report.conflicts
            ));
        } else {
            report.fail(format!(
                "{} conflicting mappings; this plan cannot be executed as written",
                report.conflicts
            ));
        }
        return Ok(report);
    }
    if !apply {
        return Ok(report);
    }
    for (index, destination) in ready {
        if let Err(error) = resolve_parent(&archive, &destination, &categories, true) {
            report.moves[index].outcome = "failed";
            report.moves[index].reason = Some(error.to_string());
            report.fail("could not create the destination parent".into());
            continue;
        }
        let to = report.moves[index].to.clone();
        match fs::rename(&report.moves[index].from, &to) {
            Ok(()) => {
                report.moves[index].outcome = "moved";
                report.moved += 1;
            }
            Err(error) => {
                report.moves[index].outcome = "failed";
                report.moves[index].reason = Some(error.to_string());
                report.fail(format!(
                    "rename failed for {}",
                    report.moves[index].collection
                ));
            }
        }
    }
    if report.moved > 0 {
        let path = journal.ok_or("--apply requires --journal PATH so every move is recorded")?;
        append_journal(path, &report, now)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media-organize-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("source")).unwrap();
            fs::create_dir(root.join("archive")).unwrap();
            Self(root)
        }
        /// A collection holding `files` regular files, whose newest write is `age` seconds old.
        fn collection(&self, name: &str, files: usize, age: u64) -> PathBuf {
            let path = self.0.join("source").join(name);
            fs::create_dir_all(&path).unwrap();
            let stamp = SystemTime::now() - Duration::from_secs(age);
            for index in 0..files {
                let file = path.join(format!("file{index}.jpg"));
                fs::write(&file, b"fixture").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&file)
                    .unwrap()
                    .set_modified(stamp)
                    .unwrap();
            }
            path
        }
        fn roots(&self) -> (PathBuf, PathBuf) {
            (self.0.join("source"), self.0.join("archive"))
        }
        fn draft(&self, value: &Value) -> Draft {
            let mut value = value.clone();
            value["source_root"] = json!(self.0.join("source"));
            value["archive_root"] = json!(self.0.join("archive"));
            Draft::parse_authorized(&value.to_string()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn base(authorized: bool) -> Value {
        json!({
            "draft_version": 1,
            "archive_root": "/archive",
            "source_root": "/source",
            "expected_volume_uuid": "0000-0000",
            "organization": {
                "categories": [{"name": "Trips"}, {"name": "Events"}],
                "default_rule": {
                    "category": "Trips",
                    "destination": "{category}/{year}/{collection}"
                }
            },
            "mappings": [],
            "execution": {"file_moves_authorized": authorized}
        })
    }

    #[test]
    fn the_drafts_own_authorization_is_required_to_run_at_all() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        assert!(Draft::parse_authorized(&base(false).to_string()).is_err());
        assert!(Draft::parse_authorized(&base(true).to_string()).is_ok());
        // Applying without somewhere to write the journal is refused before anything is read.
        assert!(organize(Path::new("/nonexistent"), true, 0, None, &[])
            .unwrap_err()
            .to_string()
            .contains("--journal"));
    }

    #[test]
    fn dry_run_places_and_creates_nothing() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 2, 3600);
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let report = run(&draft, &source, &archive, false, 0, None, &[]).unwrap();
        assert!(report.complete);
        assert!(!report.applied);
        assert_eq!(report.moved, 0);
        assert_eq!(report.moves[0].outcome, "would-move");
        assert_eq!(report.moves[0].to, archive.join("Trips/2018/2018-Finland"));
        assert!(source.join("2018-Finland").exists());
        assert!(
            !archive.join("Trips").exists(),
            "dry run created a category"
        );
    }

    #[test]
    fn apply_renames_creates_declared_ancestors_and_journals_every_move() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 2, 3600);
        fixture.collection("2018-03-Iceland", 3, 3600);
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let journal = fixture.0.join("journal.tsv");
        let report = run(&draft, &source, &archive, true, 0, Some(&journal), &[]).unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.moved, 2);
        assert_eq!(report.conflicts, 0);
        assert!(archive.join("Trips/2018/2018-Finland/file0.jpg").exists());
        assert!(archive
            .join("Trips/2018/2018-03-Iceland/file2.jpg")
            .exists());
        assert!(!source.join("2018-Finland").exists());
        let text = fs::read_to_string(&journal).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3, "header plus two moves");
        assert!(lines[0].starts_with("applied_at\tcollection\torigin"));
        assert!(lines[1..].iter().all(|line| line.contains("2018/")));
    }

    #[test]
    fn a_conflict_anywhere_stops_every_move() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        fixture.collection("2018-03-Iceland", 1, 3600);
        // The destination is already taken by a different collection.
        fs::create_dir_all(fixture.0.join("archive/Trips/2018/2018-Finland")).unwrap();
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let journal = fixture.0.join("journal.tsv");
        let report = run(&draft, &source, &archive, true, 0, Some(&journal), &[]).unwrap();
        assert!(!report.complete);
        assert_eq!(report.moved, 0);
        assert_eq!(report.conflicts, 1);
        assert!(source.join("2018-Finland").exists());
        assert!(
            source.join("2018-03-Iceland").exists(),
            "an unrelated move went ahead"
        );
        let outcomes: BTreeSet<_> = report.moves.iter().map(|m| m.outcome).collect();
        assert_eq!(outcomes, BTreeSet::from(["conflict", "refused"]));
        assert!(!journal.exists(), "nothing moved but a journal was written");
    }

    #[test]
    fn a_collection_still_being_written_is_skipped_and_does_not_block_the_others() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        fixture.collection("2023-04-Canada", 1, 5);
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let journal = fixture.0.join("journal.tsv");
        let report = run(&draft, &source, &archive, true, 300, Some(&journal), &[]).unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(report.skipped, 1);
        let skipped: Vec<_> = report
            .moves
            .iter()
            .filter(|m| m.outcome == "skipped")
            .collect();
        assert_eq!(skipped[0].collection, "2023-04-Canada");
        assert!(skipped[0].reason.as_ref().unwrap().contains("quiet window"));
        assert!(source.join("2023-04-Canada").exists());
        assert!(archive.join("Trips/2018/2018-Finland").exists());
        let text = fs::read_to_string(&journal).unwrap();
        assert_eq!(text.lines().count(), 2, "header plus the one move");
    }

    #[test]
    fn the_rule_places_dated_names_and_skip_excludes_one() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        fixture.collection("2018-03-Iceland", 1, 3600);
        fixture.collection("Inbox-old", 1, 3600);
        fixture.collection("Unnamed", 1, 3600);
        let mut draft_value = base(true);
        draft_value["organization"]["default_rule"]["skip"] = json!(["Inbox-old"]);
        let draft = fixture.draft(&draft_value);
        let resolved = preview::resolved_destinations(&draft).unwrap();
        let placed: BTreeSet<_> = resolved
            .iter()
            .map(|r| (r.collection.as_str(), r.destination.as_str(), r.origin))
            .collect();
        assert_eq!(
            placed,
            BTreeSet::from([
                ("2018-03-Iceland", "Trips/2018/2018-03-Iceland", "rule"),
                ("2018-Finland", "Trips/2018/2018-Finland", "rule"),
            ]),
            "skip, an undated name and the rule's own scope"
        );
    }

    #[test]
    fn a_provisional_mapping_never_reaches_the_executing_verb() {
        let fixture = Fixture::new();
        fixture.collection("2018-06-Portugal", 1, 3600);
        fixture.collection("2022-07-Harz", 1, 3600);
        let mut draft_value = base(true);
        draft_value["mappings"] = json!([{
            "source_collection": "2018-06-Portugal",
            "candidate_destination_relative": "Trips/2018/2018-06-Portugal",
            "status": "provisional_event_membership_needs_review"
        }]);
        let draft = fixture.draft(&draft_value);
        let resolved = preview::resolved_destinations(&draft).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].collection, "2022-07-Harz");
        assert_eq!(resolved[0].origin, "rule");
    }

    #[test]
    fn a_conflicting_plan_is_not_reported_as_complete_even_in_a_dry_run() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        fs::create_dir_all(fixture.0.join("archive/Trips/2018/2018-Finland")).unwrap();
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let report = run(&draft, &source, &archive, false, 0, None, &[]).unwrap();
        assert!(!report.complete);
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.moves[0].outcome, "conflict");
        assert!(report.issues[0].contains("cannot be executed"));
        assert_eq!(
            report.moves[0].reason.as_deref(),
            Some("destination already exists; nothing is ever overwritten")
        );
    }

    /// A draft with explicit mappings, which — unlike rule-derived placements — persist after they
    /// have been applied, because the rule enumerates the source root and an applied collection is
    /// no longer in it.
    fn mapped(names: &[(&str, &str)]) -> Value {
        let mut value = base(true);
        value["mappings"] = json!(names
            .iter()
            .map(|(collection, destination)| json!({
                "source_collection": collection,
                "proposed_destination_relative": destination,
                "status": "destination_reviewed"
            }))
            .collect::<Vec<_>>());
        value
    }

    #[test]
    fn a_mapping_this_journal_already_applied_is_not_a_collision_on_the_next_run() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 2, 3600);
        fixture.collection("2018-03-Iceland", 2, 3600);
        let value = mapped(&[
            ("2018-Finland", "Trips/2018/2018-Finland"),
            ("2018-03-Iceland", "Trips/2018/2018-03-Iceland"),
        ]);
        let draft = fixture.draft(&value);
        let (source, archive) = fixture.roots();
        let journal = fixture.0.join("journal.tsv");
        // First run moves both.
        let first = run(&draft, &source, &archive, true, 0, Some(&journal), &[]).unwrap();
        assert_eq!(first.moved, 2);
        assert_eq!(first.already_moved, 0);
        // Second run must recognise them, not report a conflict and refuse everything.
        let second = run(&draft, &source, &archive, true, 0, Some(&journal), &[]).unwrap();
        assert!(second.complete, "{:?}", second.issues);
        assert_eq!(second.moved, 0);
        assert_eq!(second.conflicts, 0);
        assert_eq!(second.already_moved, 2);
        assert!(second.moves.iter().all(|m| m.outcome == "already-moved"));
        // The journal did not grow a second time.
        assert_eq!(fs::read_to_string(&journal).unwrap().lines().count(), 3);
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_missing_source_with_a_destination_but_no_journal_row_stays_a_conflict() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 2, 3600);
        let value = mapped(&[("2018-Finland", "Trips/2018/2018-Finland")]);
        let draft = fixture.draft(&value);
        let (source, archive) = fixture.roots();
        // Something is at the destination, but nothing says this verb put it there.
        fs::create_dir_all(archive.join("Trips/2018/2018-Finland")).unwrap();
        fs::remove_dir_all(source.join("2018-Finland")).unwrap();
        let report = run(&draft, &source, &archive, false, 0, None, &[]).unwrap();
        assert!(!report.complete);
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.already_moved, 0);
        assert_eq!(report.moves[0].outcome, "conflict");
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn only_names_a_subset_and_an_unknown_name_is_refused_rather_than_doing_less() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        fixture.collection("2018-07-Denmark", 1, 3600);
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let names = vec!["2018-Finland".to_string()];
        let report = run(&draft, &source, &archive, false, 0, None, &names).unwrap();
        assert_eq!(report.moves.len(), 1, "only one collection was in scope");
        assert_eq!(report.moves[0].collection, "2018-Finland");
        assert_eq!(report.only, names);
        let typo = vec!["2018-Finlnad".to_string()];
        let error = run(&draft, &source, &archive, false, 0, None, &typo)
            .unwrap_err()
            .to_string();
        assert!(error.contains("matches no collection"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_a_collection_is_a_conflict_not_a_followed_path() {
        let fixture = Fixture::new();
        let collection = fixture.collection("2018-Finland", 1, 3600);
        std::os::unix::fs::symlink("/etc/hosts", collection.join("escape")).unwrap();
        let draft = fixture.draft(&base(true));
        let (source, archive) = fixture.roots();
        let report = run(&draft, &source, &archive, false, 0, None, &[]).unwrap();
        assert_eq!(report.conflicts, 1);
        assert!(report.moves[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("symlink inside collection"));
    }

    #[test]
    fn an_ancestor_that_is_neither_a_category_nor_a_year_is_refused() {
        let fixture = Fixture::new();
        fixture.collection("2018-Finland", 1, 3600);
        let mut draft_value = base(true);
        draft_value["organization"]["default_rule"]["destination"] =
            json!("{category}/{year}/nested/{collection}");
        let draft = fixture.draft(&draft_value);
        let (source, archive) = fixture.roots();
        let report = run(&draft, &source, &archive, false, 0, None, &[]).unwrap();
        assert_eq!(report.conflicts, 1);
        assert!(report.moves[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("not a declared category or year"));
    }
}

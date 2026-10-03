//! Make one volume's tree match another's, reading as little as possible.
//!
//! `verify-mirror` answers whether a mirror is faithful. This is the half that *creates* it, and
//! the two live in one crate because a mirror that is built by one rule and judged by another is
//! two definitions of the same word.
//!
//! Three things make this cheap, and each was measured rather than assumed (2026-10-02):
//!
//! * **The ledger is the plan.** The manifest this replaces re-hashed 305 GiB on every run to
//!   learn facts the store already held. `index` records a path's digest with the size and mtime it
//!   saw, so a file whose size and mtime have not moved is not read again — for the source *or*
//!   the destination. That is what removes the 47-minute destination re-read that dominated the
//!   previous tool: 317 GiB at the 113 MB/s a USB SSD sustains.
//! * **Parallelism is not the answer, and measuring said so.** The same SSD read 112.7 MB/s on one
//!   thread and 67.9 MB/s on eight: it is device-capped, and concurrent readers contend. The fast
//!   volume gained only 1.3x. So this walks and hashes serially, on purpose, and the reason is
//!   recorded here rather than discovered again.
//! * **A file already on the destination volume can be *moved*, not copied.** Where a path is being
//!   retired — a pre-sort snapshot, a superseded copy — its bytes are already the bytes wanted, and
//!   `fs::rename` moves the inode in microseconds. That is strictly better than `clonefile(2)`,
//!   which the workspace's `unsafe_code = "deny"` forbids anyway and which would leave two paths
//!   sharing blocks where one is wanted.
//!
//! Nothing here deletes: a consumed path is *moved*, and the journal reverses it by swapping two
//! paths. A destination path the source no longer holds is left alone and reported, because
//! removing it is a separate declared step.
//!
//! One rule is absolute and is enforced rather than described: **a run without `--apply` writes
//! nothing at all** — no media, no moved path, and no ledger row. Recording a digest is `index`'s
//! job, and a verb that quietly indexed as a side effect would make "this was a dry run" and "the
//! ledger changed" true at once. So both volumes are indexed first, and then a plan costs one stat
//! per file instead of one read.

use std::collections::BTreeMap;
use std::fs::{self, FileTimes, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::{Ledger, Location, Result};
use crate::{copy_checked, files, hash, stamp};

/// A destination path whose bytes may be *moved* into place rather than copied.
///
/// Declared, never inferred: consuming a path is what makes it disappear, and a rule that guessed
/// which paths were expendable would be the same rule that deletes the wrong one.
fn under(rel: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_matches('/');
    !prefix.is_empty() && (rel == prefix || rel.starts_with(&format!("{prefix}/")))
}

fn consumable(rel: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| under(rel, p))
}

/// Drop the row that just satisfied `rel`, so a later wanted path holding the same digest cannot
/// consume the file out of a path this run has already verified.
///
/// A digest can be wanted at more than one path. Without this, the second path's search still sees
/// the first path's row in the map, moves it away, and the run finishes having removed a path it
/// reported as verified — the report says one thing and the destination says another. Found by
/// reading this loop, and pinned by a fixture rather than left to the next live run to discover.
fn unclaim(by_digest: &mut BTreeMap<String, Vec<Location>>, digest: &str, rel: &str) {
    if let Some(rows) = by_digest.get_mut(digest) {
        if let Some(at) = rows.iter().position(|r| r.relpath == rel) {
            rows.remove(at);
        }
    }
}

/// Whether a path is inside the run's scope. `--path` narrows it; `--exclude` defers it.
///
/// Exclusion is applied to the walk rather than to the result, because the case that needs it most
/// is a folder something else is still writing: hashing it at all is the mistake, so it must not be
/// read and then discarded.
fn selected(rel: &str, paths: &[String], exclude: &[String]) -> bool {
    if exclude.iter().any(|p| under(rel, p)) {
        return false;
    }
    paths.is_empty() || paths.iter().any(|p| under(rel, p))
}

pub struct MirrorOptions<'a> {
    pub from: &'a Path,
    pub from_uuid: &'a str,
    pub to: &'a Path,
    pub to_uuid: &'a str,
    pub paths: &'a [String],
    pub exclude: &'a [String],
    pub consume: &'a [String],
    pub journal: Option<&'a Path>,
    pub apply: bool,
    pub settled_for_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct MirrorReport {
    pub from: String,
    pub to: String,
    pub applied: bool,
    /// Paths the source holds.
    pub wanted: usize,
    /// Already on the destination and trusted without a read.
    pub already_verified: usize,
    /// Already on the destination, but the record did not cover it, so it was read to find out.
    pub rechecked: usize,
    /// Moved from a declared-consumable path on the destination volume.
    pub moved: usize,
    /// Bytes actually written.
    pub copied: usize,
    pub bytes_moved: u64,
    pub bytes_copied: u64,
    /// Source rows hashed because the ledger had no current record for them.
    pub source_hashed: usize,
    /// Destination paths the source does not hold. Never removed here.
    pub destination_only: usize,
    /// Bounded sample of destination-only paths.
    pub destination_only_paths: Vec<String>,
    /// Source subtrees with a file modified inside the configured quiet window.
    pub deferred: Vec<String>,
    pub failures: Vec<String>,
}

impl MirrorReport {
    pub fn check_failed(&self) -> bool {
        self.copied > 0
            || self.moved > 0
            || self.destination_only > 0
            || !self.deferred.is_empty()
            || !self.failures.is_empty()
    }
}

struct Wanted {
    digest: String,
    size: i64,
    mtime_ns: i64,
}

/// What the source holds, with a digest for each path — read only where the ledger cannot answer.
fn plan_source(
    ledger: &Ledger,
    opts: &MirrorOptions<'_>,
) -> Result<(BTreeMap<String, Wanted>, usize)> {
    let mut wanted = BTreeMap::new();
    let mut hashed = 0;
    for (rel, path) in files(opts.from)? {
        if !selected(&rel, opts.paths, opts.exclude) {
            continue;
        }
        if is_recent(&path, opts.settled_for_seconds)? {
            continue;
        }
        let (size, mtime_ns) = stamp(&path)?;
        let recorded = ledger.location(opts.from_uuid, &rel)?;
        let digest = match recorded {
            Some(row) if row.size == size && row.mtime_ns == mtime_ns => row.digest,
            _ => {
                let digest = hash(&path)?;
                if stamp(&path)? != (size, mtime_ns) {
                    return Err(format!("source changed while hashing: {rel}").into());
                }
                hashed += 1;
                if opts.apply {
                    ledger.record(&Location {
                        uuid: opts.from_uuid.into(),
                        relpath: rel.clone(),
                        digest: digest.clone(),
                        size,
                        mtime_ns,
                    })?;
                }
                digest
            }
        };
        wanted.insert(
            rel,
            Wanted {
                digest,
                size,
                mtime_ns,
            },
        );
    }
    Ok((wanted, hashed))
}

/// The destination's own files, grouped by digest, so a wanted digest can be satisfied by moving a
/// file that is already on the volume instead of reading one across the wire.
fn index_destination(ledger: &Ledger, to_uuid: &str) -> Result<BTreeMap<String, Vec<Location>>> {
    let mut by_digest: BTreeMap<String, Vec<Location>> = BTreeMap::new();
    for row in ledger.locations(to_uuid)? {
        by_digest.entry(row.digest.clone()).or_default().push(row);
    }
    Ok(by_digest)
}

fn is_recent(path: &Path, settled_for_seconds: u64) -> Result<bool> {
    if settled_for_seconds == 0 {
        return Ok(false);
    }
    let modified = fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .map_or(true, |age| age < Duration::from_secs(settled_for_seconds)))
}

fn is_current(root: &Path, row: &Location) -> bool {
    stamp(&root.join(&row.relpath)).is_ok_and(|s| s == (row.size, row.mtime_ns))
}

fn place_moved(from: &Path, to: &Path, row: &Location, journal: Option<&Path>) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(path) = journal {
        // Append-only and flushed per row: the journal is what makes a move reversible, so it is
        // worth less than nothing if it is still in a buffer when the run dies.
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let at = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("system clock is before the epoch: {e}"))?
            .as_secs();
        writeln!(
            file,
            "{at}\t{}\t{}\t{}\t{}\tconsumed",
            from.display(),
            to.display(),
            row.size,
            row.digest,
        )?;
        file.sync_data()?;
    }
    fs::rename(from, to)?;
    Ok(())
}

/// Copy one file, checking the digest inside the write stream, and preserve the source's mtime so
/// the next run can trust this row instead of reading the bytes again.
///
/// The copy itself is `copy_checked`, the same primitive `ingest` uses. What is added here is the
/// mtime and the promotion: without the mtime the next run's size-and-mtime check would never match
/// and every file would be re-read, which is the cost this verb exists to remove.
fn place_copied(source: &Path, to: &Path, wanted: &Wanted) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = PathBuf::from(format!("{}.part", to.display()));
    // A leftover `.part` means an interrupted run. Refusing names it; overwriting it would hide the
    // interruption, and the previous tool's leftover `.part` files were exactly this.
    if temp.exists() {
        return Err(format!(
            "{} exists from an interrupted run; remove it and retry",
            temp.display()
        )
        .into());
    }
    copy_checked(source, &temp, &wanted.digest)?;
    let output = OpenOptions::new().write(true).open(&temp)?;
    output.set_times(
        FileTimes::new()
            .set_modified(UNIX_EPOCH + Duration::from_nanos(u64::try_from(wanted.mtime_ns)?)),
    )?;
    drop(output);
    fs::rename(&temp, to)?;
    Ok(())
}

pub fn mirror(ledger: &Ledger, opts: &MirrorOptions<'_>) -> Result<MirrorReport> {
    // Both volumes are registered before any location row is written. `media_locations.uuid`
    // references `media_volumes(uuid)`, so an unregistered volume is a foreign-key failure rather
    // than a row — and a mirror is the first verb that writes to two volumes at once.
    if opts.apply {
        for (root, uuid) in [(opts.from, opts.from_uuid), (opts.to, opts.to_uuid)] {
            let label = root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("volume");
            ledger.register(uuid, label)?;
        }
    }
    // A destination holding files but no ledger is an unknown tree, not a mirror to update. Every
    // path in it would have to be read before anything could be decided — measured at ~47 minutes
    // for this library, because a USB SSD sustains 113 MB/s and no amount of parallelism changes
    // that. Refusing is the same rule `verify-mirror` already applies to two unindexed volumes, and
    // it is the difference between a stated precondition and a silent hour.
    if ledger.locations(opts.to_uuid)?.is_empty() {
        let existing = files(opts.to)?;
        if !existing.is_empty() {
            return Err(format!(
                "{} holds {} files but is not indexed; run `media index --root {} --uuid {}` first",
                opts.to.display(),
                existing.len(),
                opts.to.display(),
                opts.to_uuid
            )
            .into());
        }
    }
    let (wanted, source_hashed) = plan_source(ledger, opts)?;
    let mut by_digest = index_destination(ledger, opts.to_uuid)?;

    let mut report = MirrorReport {
        from: opts.from.display().to_string(),
        to: opts.to.display().to_string(),
        applied: opts.apply,
        wanted: wanted.len(),
        already_verified: 0,
        rechecked: 0,
        moved: 0,
        copied: 0,
        bytes_moved: 0,
        bytes_copied: 0,
        source_hashed,
        destination_only: 0,
        destination_only_paths: Vec::new(),
        deferred: Vec::new(),
        failures: Vec::new(),
    };

    for (rel, want) in &wanted {
        let to = opts.to.join(rel);
        let recorded = ledger.location(opts.to_uuid, rel)?;

        // Trust the record when the file has not moved. This is the whole reason a routine sync
        // costs minutes rather than the 47 the previous tool spent re-reading the destination.
        if let Some(row) = &recorded {
            if row.digest == want.digest && is_current(opts.to, row) {
                report.already_verified += 1;
                unclaim(&mut by_digest, &want.digest, rel);
                continue;
            }
        }

        // Present but not covered by the record: the only honest way to decide is to read it. A
        // wrong digest here would be recorded as truth, so it is not guessed. This also covers a
        // destination that was never indexed — an unknown file at the target path is read rather
        // than assumed equal, and the ledger is the only thing that ever spares that read.
        let mut satisfied = false;
        if to.exists() {
            let (size, mtime_ns) = stamp(&to)?;
            if size == want.size {
                report.rechecked += 1;
                if hash(&to)? == want.digest {
                    if opts.apply {
                        ledger.record(&Location {
                            uuid: opts.to_uuid.into(),
                            relpath: rel.clone(),
                            digest: want.digest.clone(),
                            size,
                            mtime_ns,
                        })?;
                    }
                    report.already_verified += 1;
                    unclaim(&mut by_digest, &want.digest, rel);
                    satisfied = true;
                }
            }
        }
        if satisfied {
            continue;
        }

        // Move a file the destination volume already holds, when the operator has declared that
        // path expendable. Taking the first current candidate and dropping it from the map means
        // one file can satisfy exactly one wanted path.
        let candidate = by_digest.get_mut(&want.digest).and_then(|rows| {
            let at = rows
                .iter()
                .position(|r| consumable(&r.relpath, opts.consume) && is_current(opts.to, r))?;
            Some(rows.remove(at))
        });
        if let Some(row) = candidate {
            if opts.apply {
                match place_moved(&opts.to.join(&row.relpath), &to, &row, opts.journal) {
                    Ok(()) => {
                        ledger.forget(opts.to_uuid, &row.relpath)?;
                        ledger.record(&Location {
                            uuid: opts.to_uuid.into(),
                            relpath: rel.clone(),
                            digest: want.digest.clone(),
                            size: row.size,
                            mtime_ns: row.mtime_ns,
                        })?;
                    }
                    Err(error) => {
                        report.failures.push(format!("{rel}: {error}"));
                        continue;
                    }
                }
            }
            report.moved += 1;
            report.bytes_moved += u64::try_from(want.size)?;
            continue;
        }

        if opts.apply {
            let source = opts.from.join(rel);
            match place_copied(&source, &to, want) {
                Ok(()) => {
                    ledger.record(&Location {
                        uuid: opts.to_uuid.into(),
                        relpath: rel.clone(),
                        digest: want.digest.clone(),
                        size: want.size,
                        mtime_ns: want.mtime_ns,
                    })?;
                }
                Err(error) => {
                    report.failures.push(format!("{rel}: {error}"));
                    continue;
                }
            }
        }
        report.copied += 1;
        report.bytes_copied += u64::try_from(want.size)?;
    }

    let mut deferred = BTreeMap::new();
    for (rel, path) in files(opts.from)? {
        if selected(&rel, opts.paths, opts.exclude) && is_recent(&path, opts.settled_for_seconds)? {
            let parent = Path::new(&rel)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| rel.clone());
            deferred.insert(parent, ());
        }
    }
    report.deferred = deferred.into_keys().take(20).collect();
    for (rel, _) in files(opts.to)? {
        if wanted.contains_key(&rel) || !selected(&rel, opts.paths, opts.exclude) {
            continue;
        }
        report.destination_only += 1;
        if report.destination_only_paths.len() < 20 {
            report.destination_only_paths.push(rel);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Ledger;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("mirror-test-{}-{id}", std::process::id()));
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
        fn ledger(&self) -> Ledger {
            Ledger::open(&self.0.join("test.db")).unwrap()
        }
        fn src(&self) -> PathBuf {
            self.0.join("src")
        }
        fn dst(&self) -> PathBuf {
            self.0.join("dst")
        }
        /// The fixture's own paths, leaked so the borrowed options can outlive the locals. A test
        /// that outlives its own process is not a leak worth avoiding, and it keeps the options
        /// shape identical to the one the binary builds.
        fn opts(&self, apply: bool, consume: &[String]) -> MirrorOptions<'static> {
            MirrorOptions {
                from: Box::leak(self.src().into_boxed_path()),
                from_uuid: "SRC",
                to: Box::leak(self.dst().into_boxed_path()),
                to_uuid: "DST",
                consume: Box::leak(consume.to_vec().into_boxed_slice()),
                paths: &[],
                exclude: &[],
                journal: None,
                apply,
                settled_for_seconds: 0,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_dry_run_writes_nothing_and_counts_everything() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        f.write("src", "b/y.jpg", "two");
        let ledger = f.ledger();
        let report = mirror(&ledger, &f.opts(false, &[])).unwrap();
        assert_eq!(report.wanted, 2);
        assert_eq!(report.copied, 2);
        assert_eq!(report.bytes_copied, 6);
        assert!(!f.dst().join("a/x.jpg").exists(), "dry run wrote a file");
        let (files, locations) = ledger.counts().unwrap();
        assert_eq!((files, locations), (0, 0), "dry run wrote to the ledger");
    }

    #[test]
    fn check_counts_unindexed_destination_only_paths() {
        let f = Fixture::new();
        f.write("src", "same/photo.jpg", "same");
        let ledger = f.ledger();
        mirror(&ledger, &f.opts(true, &[])).unwrap();
        f.write("dst", "old/export.jpg", "leftover");
        let report = mirror(&ledger, &f.opts(false, &[])).unwrap();
        assert_eq!(report.copied, 0);
        assert_eq!(report.destination_only, 1);
        assert_eq!(report.destination_only_paths, ["old/export.jpg"]);
        assert!(report.check_failed());
    }

    #[test]
    fn a_recently_written_collection_is_deferred_without_being_hashed_or_copied() {
        let f = Fixture::new();
        f.write("src", "Trips/2026/2026-05-Oberstdorf/photo.jpg", "recent");
        let ledger = f.ledger();
        let mut opts = f.opts(false, &[]);
        opts.settled_for_seconds = 300;
        let report = mirror(&ledger, &opts).unwrap();
        assert_eq!(report.copied, 0);
        assert_eq!(report.source_hashed, 0);
        assert_eq!(report.deferred, ["Trips/2026/2026-05-Oberstdorf"]);
        assert!(report.check_failed());
        assert!(!f
            .dst()
            .join("Trips/2026/2026-05-Oberstdorf/photo.jpg")
            .exists());
    }

    #[test]
    fn an_applied_run_copies_and_preserves_mtime() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        let ledger = f.ledger();
        mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert_eq!(fs::read_to_string(f.dst().join("a/x.jpg")).unwrap(), "one");
        let source = stamp(&f.src().join("a/x.jpg")).unwrap();
        assert_eq!(stamp(&f.dst().join("a/x.jpg")).unwrap(), source);
    }

    #[test]
    fn a_second_run_reads_nothing_and_copies_nothing() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        let ledger = f.ledger();
        mirror(&ledger, &f.opts(true, &[])).unwrap();
        let again = mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert!(!again.check_failed());
        assert_eq!(again.copied, 0);
        assert_eq!(again.already_verified, 1);
        assert_eq!(again.rechecked, 0, "the record should have been trusted");
        assert_eq!(
            again.source_hashed, 0,
            "the source should have been trusted"
        );
    }

    #[test]
    fn a_verified_path_is_not_consumed_for_another_path_holding_the_same_bytes() {
        let f = Fixture::new();
        // One payload at two source paths: `a/f.jpg`, which the destination already holds and will
        // verify, and `b/g.jpg`, which it does not. The consumable prefix covers the verified path.
        f.write("src", "a/f.jpg", "payload");
        f.write("src", "b/g.jpg", "payload");
        f.write("dst", "a/f.jpg", "payload");
        let ledger = f.ledger();
        let digest = hash(&f.dst().join("a/f.jpg")).unwrap();
        let (size, mtime_ns) = stamp(&f.dst().join("a/f.jpg")).unwrap();
        ledger.register("DST", "dst").unwrap();
        ledger
            .record(&Location {
                uuid: "DST".into(),
                relpath: "a/f.jpg".into(),
                digest,
                size,
                mtime_ns,
            })
            .unwrap();

        let consume = vec!["a".to_string()];
        let report = mirror(&ledger, &f.opts(true, &consume)).unwrap();

        assert_eq!(report.already_verified, 1, "a/f.jpg is already there");
        assert_eq!(report.moved, 0, "a verified path must not be consumed");
        assert_eq!(report.copied, 1, "b/g.jpg has to come from the source");
        assert!(
            f.dst().join("a/f.jpg").exists(),
            "the run reported a/f.jpg verified and then moved it away"
        );
        assert!(f.dst().join("b/g.jpg").exists());
    }

    #[test]
    fn a_consumable_path_is_moved_rather_than_copied() {
        let f = Fixture::new();
        f.write("src", "Trips/x.jpg", "payload");
        // The same bytes already on the destination volume, at a path being retired.
        f.write("dst", "Inbox/Trips/x.jpg", "payload");
        let ledger = f.ledger();
        let digest = hash(&f.dst().join("Inbox/Trips/x.jpg")).unwrap();
        let (size, mtime_ns) = stamp(&f.dst().join("Inbox/Trips/x.jpg")).unwrap();
        ledger.register("DST", "dst").unwrap();
        ledger
            .record(&Location {
                uuid: "DST".into(),
                relpath: "Inbox/Trips/x.jpg".into(),
                digest,
                size,
                mtime_ns,
            })
            .unwrap();

        let consume = vec!["Inbox".to_string()];
        let report = mirror(&ledger, &f.opts(true, &consume)).unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(report.copied, 0);
        assert_eq!(report.bytes_copied, 0, "a move must not write bytes");
        assert!(f.dst().join("Trips/x.jpg").exists());
        assert!(
            !f.dst().join("Inbox/Trips/x.jpg").exists(),
            "the source stayed"
        );
    }

    #[test]
    fn a_path_not_declared_consumable_is_copied_not_moved() {
        let f = Fixture::new();
        f.write("src", "Trips/x.jpg", "payload");
        f.write("dst", "Inbox/Trips/x.jpg", "payload");
        let ledger = f.ledger();
        // The destination is indexed, as it has to be: a destination holding content with no ledger
        // is refused outright, because deciding anything about it means reading all of it.
        ledger.register("DST", "dst").unwrap();
        let (size, mtime_ns) = stamp(&f.dst().join("Inbox/Trips/x.jpg")).unwrap();
        ledger
            .record(&Location {
                uuid: "DST".into(),
                relpath: "Inbox/Trips/x.jpg".into(),
                digest: hash(&f.dst().join("Inbox/Trips/x.jpg")).unwrap(),
                size,
                mtime_ns,
            })
            .unwrap();
        let report = mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert_eq!(report.moved, 0);
        assert_eq!(report.copied, 1);
        assert!(
            f.dst().join("Inbox/Trips/x.jpg").exists(),
            "an undeclared path must not be consumed"
        );
    }

    #[test]
    fn an_unindexed_destination_with_content_is_refused() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        f.write("dst", "whatever.jpg", "already here");
        let ledger = f.ledger();
        let error = mirror(&ledger, &f.opts(true, &[])).unwrap_err().to_string();
        assert!(
            error.contains("is not indexed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_empty_destination_is_not_refused() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        let ledger = f.ledger();
        let report = mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert_eq!(report.copied, 1);
    }

    #[test]
    fn a_changed_destination_is_re_read_and_corrected() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "correct");
        let ledger = f.ledger();
        mirror(&ledger, &f.opts(true, &[])).unwrap();
        // Same length, different bytes, and a new mtime: the record no longer covers it.
        f.write("dst", "a/x.jpg", "corrupt");
        let again = mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert_eq!(again.rechecked, 1);
        assert_eq!(again.copied, 1);
        assert_eq!(
            fs::read_to_string(f.dst().join("a/x.jpg")).unwrap(),
            "correct"
        );
    }

    #[test]
    fn a_destination_only_path_is_reported_and_never_removed() {
        let f = Fixture::new();
        f.write("src", "a/x.jpg", "one");
        f.write("dst", "old/gone.jpg", "stale");
        let ledger = f.ledger();
        let (size, mtime_ns) = stamp(&f.dst().join("old/gone.jpg")).unwrap();
        ledger.register("DST", "dst").unwrap();
        ledger
            .record(&Location {
                uuid: "DST".into(),
                relpath: "old/gone.jpg".into(),
                digest: hash(&f.dst().join("old/gone.jpg")).unwrap(),
                size,
                mtime_ns,
            })
            .unwrap();
        let report = mirror(&ledger, &f.opts(true, &[])).unwrap();
        assert_eq!(report.destination_only, 1);
        assert!(f.dst().join("old/gone.jpg").exists(), "the mirror deleted");
    }
}

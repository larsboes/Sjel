//! An exact-byte ingest gate. The shared store owns the database file; this crate owns media_ tables.

pub mod duplicates;
pub mod ingest;
pub mod mirror;
pub mod organize;
pub mod preview;
pub mod reclaim;
pub mod reconcile;
pub mod relabel;
pub mod store;
pub mod supersede;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Result as IoResult, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use sha2::{Digest, Sha256};
use store::{Ledger, Location, Result};

pub fn hash(path: &Path) -> IoResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_digest(&hasher.finalize()))
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut hex, "{byte:02x}").expect("String writes cannot fail");
    }
    hex
}

/// Copy one file to a temporary path, checking the digest **inside the write stream**.
///
/// The check is what makes a copy evidence rather than a hope: the bytes read and the bytes written
/// are hashed together, so a file that arrived at all arrived correct, and a source that changed
/// mid-copy is caught rather than recorded as truth. `ingest` writes into a library and `mirror`
/// writes across volumes; both need exactly this, and two implementations of it would be two things
/// that can disagree about what "copied" means.
///
/// The caller owns the temporary path and its promotion. This never creates the final destination,
/// so a copy that fails cannot leave behind a file that looks imported.
pub fn copy_checked(source: &Path, temp: &Path, expected: &str) -> Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(temp)?;
    let mut digest = Sha256::new();
    let mut buf = [0_u8; 1024 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        digest.update(&buf[..n]);
    }
    output.sync_all()?;
    if hex_digest(&digest.finalize()) != expected {
        return Err("source changed while copying".into());
    }
    Ok(())
}

/// macOS writes these beside the media without being asked: `.DS_Store` records a Finder window,
/// `._*` is an AppleDouble resource fork, and the dotted directories are volume metadata. None is
/// media, all of them change on their own, and indexing them makes `index` refuse forever on a file
/// nobody put in the library.
pub fn is_macos_metadata(name: &str) -> bool {
    name == ".DS_Store"
        || name.starts_with("._")
        || matches!(
            name,
            ".Spotlight-V100"
                | ".fseventsd"
                | ".Trashes"
                | ".DocumentRevisions-V100"
                | ".TemporaryItems"
        )
}

pub fn files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let path = item.path();
            if is_macos_metadata(&item.file_name().to_string_lossy()) {
                continue;
            }
            let ty = item.file_type()?;
            if ty.is_symlink() {
                return Err(format!("symlink in media tree: {}", path.display()).into());
            }
            if ty.is_dir() {
                pending.push(path);
            } else if ty.is_file() {
                let rel = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF-8 media path")?
                    .to_owned();
                found.push((rel, path));
            } else {
                return Err(format!("not a regular file: {}", path.display()).into());
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

pub fn stamp(path: &Path) -> Result<(i64, i64)> {
    let meta = fs::metadata(path)?;
    let size = i64::try_from(meta.len())?;
    let mtime_ns = i64::try_from(meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos())?;
    Ok((size, mtime_ns))
}

/// Resolve the mounted filesystem's UUID, not a caller-supplied device name.
/// A disappeared mount point may still be a directory on the host filesystem;
/// comparing the discovered UUID to the registered one catches that case.
pub fn volume_uuid(root: &Path) -> Result<String> {
    let mut mount = root.canonicalize()?;
    if !mount.is_dir() {
        return Err("library root is not a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        while let Some(parent) = mount.parent() {
            if fs::metadata(&mount)?.dev() != fs::metadata(parent)?.dev() {
                break;
            }
            if mount == parent {
                break;
            }
            mount = parent.to_path_buf();
        }
    }
    let output = if cfg!(target_os = "macos") {
        let plist = Command::new("diskutil")
            .args(["info", "-plist"])
            .arg(&mount)
            .output()?;
        if !plist.status.success() {
            return Err(format!("diskutil cannot identify {}", mount.display()).into());
        }
        let mut child = Command::new("plutil")
            .args(["-extract", "VolumeUUID", "raw", "-o", "-", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        use std::io::Write;
        child
            .stdin
            .take()
            .ok_or("plutil has no stdin")?
            .write_all(&plist.stdout)?;
        child.wait_with_output()?
    } else {
        Command::new("findmnt")
            .args(["-n", "-o", "UUID", "--target"])
            .arg(&mount)
            .output()?
    };
    let uuid = String::from_utf8(output.stdout)?
        .trim()
        .to_ascii_uppercase();
    if !output.status.success()
        || uuid.is_empty()
        || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return Err(format!("volume UUID unavailable for {}", mount.display()).into());
    }
    Ok(uuid)
}

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub hashed: usize,
    /// Locations dropped because the path is macOS metadata rather than media. Nonzero means an
    /// earlier index admitted files it should not have.
    pub pruned: usize,
}

pub fn index(ledger: &Ledger, root: &Path, uuid: &str, label: &str) -> Result<IndexReport> {
    let paths = files(root)?;
    ledger.register(uuid, label)?;
    let mut hashed = 0;
    // Drop metadata rows recorded before `files` learned to skip them, so the location count still
    // equals what the walk reports.
    let mut pruned = 0;
    for row in ledger.locations(uuid)? {
        if row.relpath.split('/').any(is_macos_metadata) {
            pruned += usize::from(ledger.forget(uuid, &row.relpath)?);
        }
    }
    for (rel, path) in paths {
        let (size, mtime_ns) = stamp(&path)?;
        let previous = ledger.location(uuid, &rel)?;
        if previous
            .as_ref()
            .is_some_and(|r| r.size == size && r.mtime_ns == mtime_ns)
        {
            continue;
        }
        let digest = hash(&path)?;
        if previous.as_ref().is_some_and(|r| r.digest != digest) {
            return Err(format!(
                "indexed path changed bytes: {rel}; inspect rather than replacing its digest"
            )
            .into());
        }
        if stamp(&path)? != (size, mtime_ns) {
            return Err(format!("file changed during index: {rel}").into());
        }
        ledger.record(&Location {
            uuid: uuid.into(),
            relpath: rel,
            digest,
            size,
            mtime_ns,
        })?;
        hashed += 1;
    }
    Ok(IndexReport { hashed, pruned })
}

#[derive(Debug, Serialize)]
pub struct Audit {
    pub disk_files: usize,
    pub indexed_locations: usize,
    pub sampled: usize,
    pub disagreements: Vec<String>,
}

pub fn audit(ledger: &Ledger, root: &Path, uuid: &str, sample: usize) -> Result<Audit> {
    let paths = files(root)?;
    let locations = ledger.locations(uuid)?;
    let by_path: BTreeMap<_, _> = locations.iter().map(|r| (r.relpath.as_str(), r)).collect();
    let disk_paths: BTreeSet<_> = paths.iter().map(|(rel, _)| rel.as_str()).collect();
    let mut discrepancies = Vec::new();
    for row in &locations {
        if !disk_paths.contains(row.relpath.as_str()) {
            discrepancies.push(format!("unindexed absence: {}", row.relpath));
        }
    }
    for (rel, _) in &paths {
        if !by_path.contains_key(rel.as_str()) {
            discrepancies.push(format!("not indexed: {rel}"));
        }
    }
    let step = paths.len().div_ceil(sample.max(1)).max(1);
    let mut sampled = 0;
    if sample > 0 {
        for (rel, path) in paths.iter().step_by(step).take(sample) {
            sampled += 1;
            if let Some(row) = by_path.get(rel.as_str()) {
                if hash(path)? != row.digest {
                    discrepancies.push(format!("digest differs: {rel}"));
                }
            }
        }
    }
    Ok(Audit {
        disk_files: paths.len(),
        indexed_locations: locations.len(),
        sampled,
        disagreements: discrepancies,
    })
}

#[derive(Debug, Serialize)]
pub struct PathComparison {
    pub left_only: usize,
    pub right_only: usize,
    pub differing: usize,
    /// A bounded sample, so the report names what it found rather than only counting it.
    pub samples: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MirrorReport {
    pub availability: String,
    pub checked: usize,
    pub discrepancies: Vec<String>,
    /// Set only by `--paths`. The default mode compares two *sets of bytes* and answers "is every
    /// digest on both volumes"; this compares two *trees* and answers "do they match". Those are
    /// different questions, and the difference is load-bearing: measured 2026-10-02, the default
    /// mode reports a healthy mirror while every byte sits at a different path on the other volume,
    /// which is exactly the state the mirror was in after the library was reorganised.
    pub by_path: Option<PathComparison>,
}

pub fn verify_mirror(
    ledger: &Ledger,
    left: (&Path, &str),
    right: (&Path, &str),
    by_path: bool,
) -> Result<MirrorReport> {
    let mut absent = Vec::new();
    for (root, expected) in [left, right] {
        if !root.exists() || volume_uuid(root)?.as_str() != expected {
            absent.push(expected.to_string());
        }
    }
    if !absent.is_empty() {
        return Ok(MirrorReport {
            availability: format!("absent: {}", absent.join(", ")),
            checked: 0,
            discrepancies: Vec::new(),
            by_path: None,
        });
    }
    if by_path {
        verify_by_path(ledger, left, right)
    } else {
        verify_indexed(ledger, left, right)
    }
}

pub struct SyncOptions<'a> {
    pub from: &'a Path,
    pub from_uuid: &'a str,
    pub to: &'a Path,
    pub to_uuid: &'a str,
    pub paths: &'a [String],
    pub exclude: &'a [String],
    pub consume: &'a [String],
    pub journal: Option<&'a Path>,
    pub settled_for_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub indexed_from: IndexReport,
    pub indexed_to: IndexReport,
    pub mirror: mirror::MirrorReport,
    pub verification: Option<MirrorReport>,
    pub verified: bool,
}

/// Run the ordered operator workflow. The caller validates both mounted volume UUIDs before calling:
/// index each side, apply the mirror, then verify the paths as trees. This function deliberately
/// does not reclaim destination-only paths.
pub fn sync(ledger: &Ledger, opts: &SyncOptions<'_>) -> Result<SyncReport> {
    let label = |root: &Path| {
        root.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("library")
            .to_owned()
    };
    let from_label = label(opts.from);
    let to_label = label(opts.to);
    let indexed_from = index(ledger, opts.from, opts.from_uuid, &from_label)?;
    let indexed_to = index(ledger, opts.to, opts.to_uuid, &to_label)?;
    let mirrored = mirror::mirror(
        ledger,
        &mirror::MirrorOptions {
            from: opts.from,
            from_uuid: opts.from_uuid,
            to: opts.to,
            to_uuid: opts.to_uuid,
            paths: opts.paths,
            exclude: opts.exclude,
            consume: opts.consume,
            journal: opts.journal,
            apply: true,
            settled_for_seconds: opts.settled_for_seconds,
        },
    )?;
    let verification = if mirrored.deferred.is_empty() {
        Some(verify_by_path(
            ledger,
            (opts.from, opts.from_uuid),
            (opts.to, opts.to_uuid),
        )?)
    } else {
        None
    };
    let verified = mirrored.failures.is_empty()
        && mirrored.deferred.is_empty()
        && mirrored.destination_only == 0
        && verification.as_ref().is_some_and(|report| {
            report.discrepancies.is_empty()
                && report.by_path.as_ref().is_some_and(|paths| {
                    paths.left_only == 0 && paths.right_only == 0 && paths.differing == 0
                })
        });
    Ok(SyncReport {
        indexed_from,
        indexed_to,
        mirror: mirrored,
        verification,
        verified,
    })
}

/// The two trees compared as trees. Both volumes must be indexed, for the same reason the digest
/// mode requires it: without an index there is nothing to compare that is not a full read.
fn verify_by_path(
    ledger: &Ledger,
    left: (&Path, &str),
    right: (&Path, &str),
) -> Result<MirrorReport> {
    let left_rows = ledger.locations(left.1)?;
    let right_rows = ledger.locations(right.1)?;
    if left_rows.is_empty() || right_rows.is_empty() {
        return Err("both mounted volumes must be indexed before mirror verification".into());
    }
    for (root, rows) in [(left.0, &left_rows), (right.0, &right_rows)] {
        ensure_index_current(root, rows)?;
    }
    let left_map: BTreeMap<&str, &str> = left_rows
        .iter()
        .map(|r| (r.relpath.as_str(), r.digest.as_str()))
        .collect();
    let right_map: BTreeMap<&str, &str> = right_rows
        .iter()
        .map(|r| (r.relpath.as_str(), r.digest.as_str()))
        .collect();

    let mut comparison = PathComparison {
        left_only: 0,
        right_only: 0,
        differing: 0,
        samples: Vec::new(),
    };
    let mut note = |line: String, counter: &mut usize| {
        *counter += 1;
        if comparison.samples.len() < 20 {
            comparison.samples.push(line);
        }
    };
    for (rel, digest) in &left_map {
        match right_map.get(rel) {
            None => note(format!("only on left: {rel}"), &mut comparison.left_only),
            Some(other) if other != digest => {
                note(format!("different bytes: {rel}"), &mut comparison.differing)
            }
            _ => {}
        }
    }
    for rel in right_map.keys() {
        if !left_map.contains_key(rel) {
            note(format!("only on right: {rel}"), &mut comparison.right_only);
        }
    }
    let checked = left_map.len().max(right_map.len());
    Ok(MirrorReport {
        availability: "mounted".into(),
        checked,
        discrepancies: Vec::new(),
        by_path: Some(comparison),
    })
}

/// Refuse comparisons when the filesystem contains a path the ledger cannot describe. Both mirror
/// modes rely on the index; reporting from a stale snapshot can claim completeness while new files
/// are already on disk.
pub(crate) fn ensure_index_current(root: &Path, rows: &[Location]) -> Result<()> {
    let recorded: BTreeSet<&str> = rows.iter().map(|r| r.relpath.as_str()).collect();
    let unindexed: Vec<String> = files(root)?
        .into_iter()
        .map(|(rel, _)| rel)
        .filter(|rel| !recorded.contains(rel.as_str()))
        .collect();
    if !unindexed.is_empty() {
        return Err(format!(
            "{} has {} paths on disk that are not indexed (first: {}); run `media index` before verifying the mirror",
            root.display(),
            unindexed.len(),
            unindexed[0]
        )
        .into());
    }
    Ok(())
}

fn verify_indexed(
    ledger: &Ledger,
    left: (&Path, &str),
    right: (&Path, &str),
) -> Result<MirrorReport> {
    let left_rows = ledger.locations(left.1)?;
    let right_rows = ledger.locations(right.1)?;
    if left_rows.is_empty() || right_rows.is_empty() {
        return Err("both mounted volumes must be indexed before mirror verification".into());
    }
    for (root, rows) in [(left.0, &left_rows), (right.0, &right_rows)] {
        ensure_index_current(root, rows)?;
    }
    let mut groups: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for r in left_rows {
        groups.entry(r.digest).or_default().0.push(r.relpath);
    }
    for r in right_rows {
        groups.entry(r.digest).or_default().1.push(r.relpath);
    }
    let mut discrepancies = Vec::new();
    for (digest, (a, b)) in &groups {
        let mut problems = Vec::new();
        if a.is_empty() || b.is_empty() {
            problems.push("only on one volume".to_string());
        }
        for (root, paths) in [(left.0, a), (right.0, b)] {
            for rel in paths {
                let path = root.join(rel);
                match hash(&path) {
                    Ok(actual) if actual == *digest => (),
                    Ok(actual) => problems.push(format!("{} hashes to {actual}", path.display())),
                    Err(e) => problems.push(format!("{}: {e}", path.display())),
                }
            }
        }
        if !problems.is_empty() {
            discrepancies.push(format!("{digest}: {}", problems.join("; ")));
        }
    }
    Ok(MirrorReport {
        availability: "mounted".into(),
        checked: groups.len(),
        discrepancies,
        by_path: None,
    })
}

#[cfg(test)]
mod db_tests {
    use super::*;
    #[test]
    fn macos_metadata_is_not_media_and_a_recorded_row_is_pruned_rather_than_left_absent() {
        let dir = std::env::temp_dir().join(format!("media-noise-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("tree/nested")).unwrap();
        fs::write(dir.join("tree/photo.jpg"), b"media").unwrap();
        fs::write(dir.join("tree/.DS_Store"), b"finder state").unwrap();
        fs::write(dir.join("tree/nested/._photo.jpg"), b"apple double").unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        let first = index(&db, &dir.join("tree"), "V", "v").unwrap();
        assert_eq!(
            (first.hashed, first.pruned),
            (1, 0),
            "only the photo is media"
        );
        // A row recorded before metadata was excluded must be dropped, not reported as an absence.
        db.record(&Location {
            uuid: "V".into(),
            relpath: ".DS_Store".into(),
            digest: "0".repeat(64),
            size: 12,
            mtime_ns: 0,
        })
        .unwrap();
        assert_eq!(db.counts().unwrap(), (2, 2));
        let second = index(&db, &dir.join("tree"), "V", "v").unwrap();
        assert_eq!((second.hashed, second.pruned), (0, 1));
        assert_eq!(
            db.counts().unwrap(),
            (1, 1),
            "the metadata row and its digest are gone"
        );
        assert!(audit(&db, &dir.join("tree"), "V", 0)
            .unwrap()
            .disagreements
            .is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_volumes_one_digest_and_repeat_index() {
        let dir = std::env::temp_dir().join(format!("media-fixture-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/photo.jpg"), b"bytes").unwrap();
        fs::write(dir.join("b/photo.jpg"), b"bytes").unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        assert_eq!(index(&db, &dir.join("a"), "A", "a").unwrap().hashed, 1);
        assert_eq!(index(&db, &dir.join("b"), "B", "b").unwrap().hashed, 1);
        assert_eq!(db.counts().unwrap(), (1, 2));
        assert_eq!(index(&db, &dir.join("a"), "A", "a").unwrap().hashed, 0);
        assert_eq!(db.counts().unwrap(), (1, 2));
        let audit = audit(&db, &dir.join("a"), "A", 1).unwrap();
        assert!(audit.disagreements.is_empty());
        fs::write(dir.join("a/photo.jpg"), b"changed bytes").unwrap();
        assert!(index(&db, &dir.join("a"), "A", "a").is_err());
        assert_eq!(
            db.location("A", "photo.jpg").unwrap().unwrap().digest,
            hash(&dir.join("b/photo.jpg")).unwrap()
        );
        fs::write(dir.join("a/photo.jpg"), b"bytes").unwrap();
        let mirror = verify_indexed(&db, (&dir.join("a"), "A"), (&dir.join("b"), "B")).unwrap();
        assert_eq!(mirror.checked, 1);
        assert!(mirror.discrepancies.is_empty());
        fs::write(dir.join("b/photo.jpg"), b"bad!!").unwrap();
        let mirror = verify_indexed(&db, (&dir.join("a"), "A"), (&dir.join("b"), "B")).unwrap();
        assert_eq!(mirror.discrepancies.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A ledger and two trees that describe exactly the rows given: `left/<relpath>` for volume A,
    /// `right/<relpath>` for volume B, with the digest computed from the bytes actually written. The
    /// database lives outside both trees, because a file inside them would be an unindexed path and
    /// the staleness guard would — correctly — refuse to compare anything.
    fn ledger_with(rows: &[(&str, &str, &str)]) -> (PathBuf, Ledger) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("media-paths-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("left")).unwrap();
        fs::create_dir_all(dir.join("right")).unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        for (uuid, relpath, body) in rows {
            let side = if *uuid == "A" { "left" } else { "right" };
            let path = dir.join(side).join(relpath);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body.as_bytes()).unwrap();
            let (size, mtime_ns) = stamp(&path).unwrap();
            db.register(uuid, "v").unwrap();
            db.record(&Location {
                uuid: (*uuid).into(),
                relpath: (*relpath).into(),
                digest: hash(&path).unwrap(),
                size,
                mtime_ns,
            })
            .unwrap();
        }
        (dir, db)
    }

    fn sides(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("left"), dir.join("right"))
    }

    fn sync_fixture() -> (PathBuf, Ledger) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("media-sync-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("from")).unwrap();
        fs::create_dir_all(dir.join("to")).unwrap();
        let ledger = Ledger::open(&dir.join("scratch.db")).unwrap();
        (dir, ledger)
    }

    fn sync_options<'a>(from: &'a Path, to: &'a Path, settled_for_seconds: u64) -> SyncOptions<'a> {
        SyncOptions {
            from,
            from_uuid: "FROM",
            to,
            to_uuid: "TO",
            paths: &[],
            exclude: &[],
            consume: &[],
            journal: None,
            settled_for_seconds,
        }
    }

    #[test]
    fn sync_indexes_applies_verifies_and_is_idempotent() {
        let (dir, ledger) = sync_fixture();
        let from = dir.join("from");
        let to = dir.join("to");
        fs::create_dir_all(from.join("Trips/2026")).unwrap();
        fs::write(from.join("Trips/2026/photo.jpg"), b"photo bytes").unwrap();
        let opts = sync_options(&from, &to, 0);

        let first = sync(&ledger, &opts).unwrap();
        assert_eq!(first.indexed_from.hashed, 1);
        assert_eq!(first.indexed_to.hashed, 0);
        assert_eq!(first.mirror.copied, 1);
        assert!(first.verified);
        assert_eq!(
            fs::read(to.join("Trips/2026/photo.jpg")).unwrap(),
            b"photo bytes"
        );
        assert_eq!(
            first
                .verification
                .as_ref()
                .unwrap()
                .by_path
                .as_ref()
                .unwrap()
                .left_only,
            0
        );

        let second = sync(&ledger, &opts).unwrap();
        assert_eq!(second.indexed_from.hashed, 0);
        assert_eq!(second.mirror.copied, 0);
        assert!(second.verified);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sync_reports_destination_only_without_removing_it() {
        let (dir, ledger) = sync_fixture();
        let from = dir.join("from");
        let to = dir.join("to");
        fs::write(from.join("wanted.jpg"), b"wanted").unwrap();
        fs::write(to.join("old.jpg"), b"destination-only").unwrap();
        let report = sync(&ledger, &sync_options(&from, &to, 0)).unwrap();
        assert_eq!(report.mirror.destination_only, 1);
        assert!(!report.verified);
        assert!(report.verification.is_some());
        assert_eq!(fs::read(to.join("old.jpg")).unwrap(), b"destination-only");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sync_defers_recent_source_subtrees_and_skips_final_verification() {
        let (dir, ledger) = sync_fixture();
        let from = dir.join("from");
        let to = dir.join("to");
        let collection = from.join("Trips/2026/2026-05-Trip");
        fs::create_dir_all(&collection).unwrap();
        fs::write(collection.join("photo.jpg"), b"recent export").unwrap();
        let report = sync(&ledger, &sync_options(&from, &to, 300)).unwrap();
        assert_eq!(report.mirror.deferred, ["Trips/2026/2026-05-Trip"]);
        assert!(!report.verified);
        assert!(report.verification.is_none());
        assert!(!to.join("Trips/2026/2026-05-Trip/photo.jpg").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_catches_a_tree_that_moved() {
        // The same bytes at different paths. The digest mode calls this a faithful mirror — every
        // digest is on both volumes — which is exactly why `--paths` exists: it is the state the
        // mirror was in after the library was reorganised, and the state a restore-from mirror must
        // not be in.
        let (dir, db) = ledger_with(&[
            ("A", "old/photo.jpg", "same bytes"),
            ("B", "new/photo.jpg", "same bytes"),
        ]);
        let (left, right) = sides(&dir);
        let report = verify_by_path(&db, (&left, "A"), (&right, "B")).unwrap();
        let comparison = report.by_path.unwrap();
        assert_eq!(comparison.left_only, 1);
        assert_eq!(comparison.right_only, 1);
        assert_eq!(comparison.differing, 0);
        assert_eq!(
            report.discrepancies.len(),
            0,
            "the digest mode's findings must not be invented here"
        );
        assert_eq!(
            comparison.samples.len(),
            2,
            "a count without a name is not a report"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_refuses_a_stale_index() {
        // A file on disk that no row describes. Comparing trees from a stale index answers a
        // question about the past, and two trees that differ would read as matching.
        let (dir, db) = ledger_with(&[("A", "same/photo.jpg", "x"), ("B", "same/photo.jpg", "x")]);
        fs::write(dir.join("left/same/unindexed.jpg"), b"y").unwrap();
        let (left, right) = sides(&dir);
        let error = verify_by_path(&db, (&left, "A"), (&right, "B"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not indexed"), "unexpected: {error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn digest_mode_refuses_a_stale_index() {
        let (dir, db) = ledger_with(&[("A", "same/photo.jpg", "x"), ("B", "same/photo.jpg", "x")]);
        fs::write(dir.join("left/same/unindexed.jpg"), b"new bytes").unwrap();
        let (left, right) = sides(&dir);
        let error = verify_indexed(&db, (&left, "A"), (&right, "B"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not indexed"), "unexpected: {error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_reports_matching_trees_as_clean() {
        let (dir, db) = ledger_with(&[("A", "same/photo.jpg", "x"), ("B", "same/photo.jpg", "x")]);
        let (left, right) = sides(&dir);
        let comparison = verify_by_path(&db, (&left, "A"), (&right, "B"))
            .unwrap()
            .by_path
            .unwrap();
        assert_eq!(
            (
                comparison.left_only,
                comparison.right_only,
                comparison.differing
            ),
            (0, 0, 0)
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_catches_one_path_holding_different_bytes() {
        let (dir, db) = ledger_with(&[
            ("A", "same/photo.jpg", "left bytes"),
            ("B", "same/photo.jpg", "right bytes"),
        ]);
        let (left, right) = sides(&dir);
        let comparison = verify_by_path(&db, (&left, "A"), (&right, "B"))
            .unwrap()
            .by_path
            .unwrap();
        assert_eq!(comparison.differing, 1);
        assert_eq!((comparison.left_only, comparison.right_only), (0, 0));
        fs::remove_dir_all(&dir).unwrap();
    }
}

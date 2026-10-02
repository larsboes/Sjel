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
use std::fs::{self, File};
use std::io::{Read, Result as IoResult};
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
    /// which is exactly the state INTENSO was in after the library was reorganised.
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
        assert_eq!((first.hashed, first.pruned), (1, 0), "only the photo is media");
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
        assert_eq!(db.counts().unwrap(), (1, 1), "the metadata row and its digest are gone");
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

    /// A ledger holding the rows named, with both volumes registered.
    fn ledger_with(rows: &[(&str, &str, &str)]) -> (PathBuf, Ledger) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("media-paths-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        for (uuid, relpath, digest) in rows {
            db.register(uuid, "v").unwrap();
            db.record(&Location {
                uuid: (*uuid).into(),
                relpath: (*relpath).into(),
                digest: (*digest).into(),
                size: 1,
                mtime_ns: 1,
            })
            .unwrap();
        }
        (dir, db)
    }

    #[test]
    fn path_mode_catches_a_tree_that_moved() {
        // The same bytes at different paths. The digest mode calls this a faithful mirror — every
        // digest is on both volumes — which is exactly why `--paths` exists: it is the state INTENSO
        // was in after the library was reorganised, and the state a restore-from mirror must not be
        // in.
        let (dir, db) = ledger_with(&[("A", "old/photo.jpg", "aa"), ("B", "new/photo.jpg", "aa")]);
        let report = verify_by_path(&db, (&dir, "A"), (&dir, "B")).unwrap();
        let comparison = report.by_path.unwrap();
        assert_eq!(comparison.left_only, 1);
        assert_eq!(comparison.right_only, 1);
        assert_eq!(comparison.differing, 0);
        assert_eq!(
            report.discrepancies.len(),
            0,
            "the digest mode's findings must not be invented here"
        );
        assert_eq!(comparison.samples.len(), 2, "a count without a name is not a report");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_reports_matching_trees_as_clean() {
        let (dir, db) = ledger_with(&[("A", "same/photo.jpg", "aa"), ("B", "same/photo.jpg", "aa")]);
        let comparison = verify_by_path(&db, (&dir, "A"), (&dir, "B"))
            .unwrap()
            .by_path
            .unwrap();
        assert_eq!((comparison.left_only, comparison.right_only, comparison.differing), (0, 0, 0));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_mode_catches_one_path_holding_different_bytes() {
        let (dir, db) = ledger_with(&[("A", "same/photo.jpg", "aa"), ("B", "same/photo.jpg", "bb")]);
        let comparison = verify_by_path(&db, (&dir, "A"), (&dir, "B"))
            .unwrap()
            .by_path
            .unwrap();
        assert_eq!(comparison.differing, 1);
        assert_eq!((comparison.left_only, comparison.right_only), (0, 0));
        fs::remove_dir_all(&dir).unwrap();
    }
}

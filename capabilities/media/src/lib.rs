//! An exact-byte ingest gate. The shared store owns the database file; this crate owns media_ tables.

pub mod ingest;
pub mod store;

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

pub fn files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let path = item.path();
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

pub fn index(ledger: &Ledger, root: &Path, uuid: &str, label: &str) -> Result<usize> {
    let paths = files(root)?;
    ledger.register(uuid, label)?;
    let mut hashed = 0;
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
    Ok(hashed)
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
pub struct MirrorReport {
    pub availability: String,
    pub checked: usize,
    pub discrepancies: Vec<String>,
}

pub fn verify_mirror(
    ledger: &Ledger,
    left: (&Path, &str),
    right: (&Path, &str),
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
        });
    }
    verify_indexed(ledger, left, right)
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
    })
}

#[cfg(test)]
mod db_tests {
    use super::*;
    #[test]
    fn two_volumes_one_digest_and_repeat_index() {
        let dir = std::env::temp_dir().join(format!("media-fixture-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/photo.jpg"), b"bytes").unwrap();
        fs::write(dir.join("b/photo.jpg"), b"bytes").unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        assert_eq!(index(&db, &dir.join("a"), "A", "a").unwrap(), 1);
        assert_eq!(index(&db, &dir.join("b"), "B", "b").unwrap(), 1);
        assert_eq!(db.counts().unwrap(), (1, 2));
        assert_eq!(index(&db, &dir.join("a"), "A", "a").unwrap(), 0);
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
}

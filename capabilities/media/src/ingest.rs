use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::store::{Ledger, Location, Outcome, Result};
use crate::{files, hash, hex_digest, stamp};

#[derive(Default, Debug, Serialize)]
pub struct IngestReport {
    pub considered: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub refused: usize,
    pub failed: usize,
    pub resumed: usize,
    pub verified: usize,
    pub bytes_imported: u64,
    pub pruned: bool,
}

// ExifTool emits RFC 4180 quoting in CSV mode. Parse records rather than lines:
// a filename can contain a comma, a quote, or a newline.
fn csv_records(input: &str) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut at_start = true;
    let mut iter = input.chars().peekable();
    while let Some(ch) = iter.next() {
        if quoted {
            if ch == '"' {
                if iter.peek() == Some(&'"') {
                    field.push('"');
                    iter.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else {
            match ch {
                '"' if at_start => {
                    quoted = true;
                    at_start = false;
                }
                ',' => {
                    row.push(std::mem::take(&mut field));
                    at_start = true;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut row));
                    at_start = true;
                }
                '\r' if iter.peek() == Some(&'\n') => (),
                _ => {
                    field.push(ch);
                    at_start = false;
                }
            }
        }
    }
    if quoted {
        return Err("unterminated exiftool CSV field".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        records.push(row);
    }
    Ok(records)
}

fn exif_dates(root: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let output = Command::new("exiftool")
        .args([
            "-csv",
            "-r",
            "-SourceFile",
            "-DateTimeOriginal",
            "-d",
            "%Y-%m-%dT%H:%M:%S",
        ])
        .arg(root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "exiftool failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let csv = String::from_utf8(output.stdout)?;
    let mut rows = csv_records(&csv)?.into_iter();
    let header = rows.next().ok_or("exiftool produced no CSV header")?;
    let src = header
        .iter()
        .position(|h| h == "SourceFile")
        .ok_or("exiftool CSV lacks SourceFile")?;
    let mut found = BTreeMap::new();
    let Some(date) = header.iter().position(|h| h == "DateTimeOriginal") else {
        return Ok(found);
    };
    for row in rows {
        let path = row.get(src).ok_or("incomplete exiftool row")?;
        let value = row.get(date).ok_or("incomplete exiftool row")?;
        if let Some(day) = value.get(..10) {
            if civil_date::unix_day_of_iso(day).is_some() {
                found.insert(PathBuf::from(path), day.to_string());
            }
        }
    }
    Ok(found)
}

fn dated_filename(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    for span in bytes.windows(8) {
        if !span.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let d = String::from_utf8_lossy(span);
        let date = format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]);
        if civil_date::unix_day_of_iso(&date).is_some() {
            return Some(date);
        }
    }
    None
}

fn destination_month(path: &Path, dates: &BTreeMap<PathBuf, String>) -> Result<String> {
    let date = dates
        .get(path)
        .cloned()
        .or_else(|| {
            path.file_name()
                .and_then(|s| s.to_str())
                .and_then(dated_filename)
        })
        .or_else(|| {
            fs::metadata(path)
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|t| civil_date::iso_of_unix_day((t.as_secs() / 86400) as i64))
        });
    Ok(date.map_or_else(|| "_undated".into(), |d| format!("by-date/{}", &d[..7])))
}

fn candidate(library: &Path, folder: &str, name: &str, ordinal: usize) -> (String, PathBuf) {
    let path = Path::new(name);
    let renamed = if ordinal == 1 {
        name.to_string()
    } else {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        let ext = path.extension().and_then(|s| s.to_str());
        match ext {
            Some(ext) => format!("{stem}~{ordinal}.{ext}"),
            None => format!("{stem}~{ordinal}"),
        }
    };
    let rel = format!("{folder}/{renamed}");
    let dest = library.join(&rel);
    (rel, dest)
}

fn staged_hashes(
    originals: &Path,
    paths: &[(String, PathBuf)],
    output: &Path,
) -> Result<Vec<(String, PathBuf, String, i64)>> {
    match fs::symlink_metadata(output) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err("staging-hashes.tsv must be a regular file, not a symlink".into())
        }
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let parent = output.parent().ok_or("hash report has no parent")?;
    let (temp, mut tsv) = (0..10000)
        .find_map(|n| {
            let path = parent.join(format!(".staging-hashes-{}-{n}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((path, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or("no collision-free temporary hash report")??;
    let result = (|| -> Result<Vec<(String, PathBuf, String, i64)>> {
        let mut rows = Vec::new();
        writeln!(tsv, "sha256\tsize\tmtime_ns\trelpath_json")?;
        for (rel, path) in paths {
            let (size, mtime) = stamp(path)?;
            let digest = hash(path)?;
            if stamp(path)? != (size, mtime) {
                return Err(format!("changed while hashing: {rel}").into());
            }
            writeln!(
                tsv,
                "{digest}\t{size}\t{mtime}\t{}",
                serde_json::to_string(rel)?
            )?;
            rows.push((rel.clone(), path.clone(), digest, size));
        }
        tsv.sync_all()?;
        if !originals.is_dir() {
            return Err("originals disappeared".into());
        }
        Ok(rows)
    })();
    drop(tsv);
    if result.is_err() {
        fs::remove_file(&temp)?;
        return result;
    }
    fs::rename(&temp, output)?;
    fs::File::open(parent)?.sync_all()?;
    result
}

fn copy_checked(source: &Path, temp: &Path, expected: &str) -> Result<()> {
    let mut input = fs::File::open(source)?;
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

pub struct IngestOptions<'a> {
    pub staging: &'a Path,
    pub library: &'a Path,
    pub uuid: &'a str,
    pub apply: bool,
    pub prune: bool,
    pub fail_before_verify: bool,
}

pub fn ingest(ledger: &Ledger, opts: &IngestOptions<'_>) -> Result<IngestReport> {
    if opts.prune && !opts.apply {
        return Err("--prune requires --apply".into());
    }
    let staging = opts.staging.canonicalize()?;
    let library = opts.library.canonicalize()?;
    if staging == library || staging.starts_with(&library) || library.starts_with(&staging) {
        return Err("staging and library must not contain one another".into());
    }
    let originals = staging.join("originals");
    if fs::symlink_metadata(&originals)?.file_type().is_symlink() {
        return Err("originals must be a directory, not a symlink".into());
    }
    let manifest_path = staging.join("export-manifest.json");
    if fs::symlink_metadata(&manifest_path)?
        .file_type()
        .is_symlink()
    {
        return Err("export manifest must not be a symlink".into());
    }
    let manifest = fs::read_to_string(manifest_path)?;
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest)?;
    let source = manifest_json
        .get("source")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("export manifest needs source")?;
    let mut refusals = BTreeMap::new();
    if let Some(entries) = manifest_json.get("refusals") {
        for entry in entries.as_array().ok_or("refusals must be an array")? {
            let path = entry
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("refusal needs path")?;
            let reason = entry
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .ok_or("refusal needs reason")?;
            let evidence = entry
                .get("evidence")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .ok_or("refusal needs evidence")?;
            refusals.insert(path.to_string(), format!("{reason}: {evidence}"));
        }
    }
    let paths = files(&originals)?;
    if refusals
        .keys()
        .any(|key| !paths.iter().any(|(rel, _)| rel == key))
    {
        return Err("manifest refusal names a file absent from originals".into());
    }
    let rows = staged_hashes(&originals, &paths, &staging.join("staging-hashes.tsv"))?;
    let dates = exif_dates(&originals)?;
    let id = if opts.apply {
        Some(ledger.start_ingest(source, &staging.to_string_lossy(), &manifest)?)
    } else {
        None
    };
    let mut seen = BTreeSet::new();
    let mut report = IngestReport::default();
    // Temporary writes are outside Library; only a verified copy gets linked into it.
    let incoming = library
        .parent()
        .ok_or("library has no parent")?
        .join(".media-incoming");
    for (rel, path, digest, size) in rows {
        report.considered += 1;
        let prior = ledger.prior_verified(&staging.to_string_lossy(), &rel, &digest)?;
        let duplicate = ledger.has_digest(&digest)? || seen.contains(&digest);
        if let Some(reason) = refusals.get(&rel) {
            report.refused += 1;
            if let Some(id) = id {
                ledger.item(
                    id,
                    &rel,
                    Some(&digest),
                    Some(size),
                    Outcome::new("refused", reason, None, false),
                )?;
            }
            continue;
        }
        if let Some(prior_rel) = prior {
            let dest = library.join(&prior_rel);
            if dest.is_file() && hash(&dest)? == digest {
                report.resumed += 1;
                if let Some(id) = id {
                    ledger.item(
                        id,
                        &rel,
                        Some(&digest),
                        Some(size),
                        Outcome::new(
                            "resumed",
                            "verified earlier from this staging path",
                            Some(&prior_rel),
                            true,
                        ),
                    )?;
                }
                continue;
            }
        }
        if let Some(pending) = ledger.prior_unverified(&staging.to_string_lossy(), &rel, &digest)? {
            let dest = library.join(&pending);
            if dest.is_file() && hash(&dest)? == digest {
                fs::File::open(dest.parent().ok_or("import has no parent")?)?.sync_all()?;
                let (size_now, mtime_ns) = stamp(&dest)?;
                if let Some(id) = id {
                    ledger.record(&Location {
                        uuid: opts.uuid.into(),
                        relpath: pending.clone(),
                        digest: digest.clone(),
                        size: size_now,
                        mtime_ns,
                    })?;
                    ledger.item(
                        id,
                        &rel,
                        Some(&digest),
                        Some(size),
                        Outcome::new(
                            "resumed",
                            "verified interrupted import",
                            Some(&pending),
                            true,
                        ),
                    )?;
                }
                report.resumed += 1;
                continue;
            }
            if dest.exists() && ledger.location(opts.uuid, &pending)?.is_none() {
                report.failed += 1;
                if let Some(id) = id {
                    ledger.item(
                        id,
                        &rel,
                        Some(&digest),
                        Some(size),
                        Outcome::new(
                            "failed",
                            "previous import exists but cannot be verified; inspect before retry",
                            Some(&pending),
                            false,
                        ),
                    )?;
                }
                continue;
            }
        }
        if duplicate {
            report.duplicates += 1;
            if let Some(id) = id {
                ledger.item(
                    id,
                    &rel,
                    Some(&digest),
                    Some(size),
                    Outcome::new(
                        "duplicate",
                        "digest exists in index or this batch",
                        None,
                        false,
                    ),
                )?;
            }
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str());
        let Some(name) = name else {
            report.refused += 1;
            if let Some(id) = id {
                ledger.item(
                    id,
                    &rel,
                    Some(&digest),
                    Some(size),
                    Outcome::new("refused", "non-UTF-8 file name", None, false),
                )?;
            }
            continue;
        };
        let folder = destination_month(&path, &dates)?;
        if !opts.apply {
            seen.insert(digest);
            report.imported += 1;
            report.bytes_imported += size as u64;
            continue;
        }
        let id = id.ok_or("ingest ID missing")?;
        ledger.item(
            id,
            &rel,
            Some(&digest),
            Some(size),
            Outcome::new("failed", "copy not yet verified", None, false),
        )?;
        let work = (|| -> Result<(String, PathBuf)> {
            fs::create_dir_all(&incoming)?;
            let temp = incoming.join(format!("{id}-{}", report.considered));
            let result = (|| -> Result<(String, PathBuf)> {
                copy_checked(&path, &temp, &digest)?;
                for ordinal in 1..=10000 {
                    let (dest_rel, dest) = candidate(&library, &folder, name, ordinal);
                    fs::create_dir_all(dest.parent().ok_or("import has no parent")?)?;
                    ledger.reserve_path(id, &rel, &dest_rel)?;
                    match fs::hard_link(&temp, &dest) {
                        Ok(()) => {
                            fs::File::open(dest.parent().ok_or("import has no parent")?)?
                                .sync_all()?;
                            return Ok((dest_rel, dest));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
                Err("no collision-free file name after 10000 attempts".into())
            })();
            if temp.exists() {
                fs::remove_file(&temp)?;
            }
            result
        })();
        match work {
            Ok((dest_rel, dest)) => {
                // The reservation stays unverified until the final path has been read back.
                if opts.fail_before_verify {
                    ledger.fail_item(id, &rel, "verification interrupted")?;
                    report.failed += 1;
                    continue;
                }
                if hash(&dest)? != digest || fs::metadata(&dest)?.len() != size as u64 {
                    ledger.fail_item(id, &rel, "imported bytes failed verification")?;
                    report.failed += 1;
                    continue;
                }
                let (size_now, mtime_ns) = stamp(&dest)?;
                ledger.record(&Location {
                    uuid: opts.uuid.into(),
                    relpath: dest_rel,
                    digest: digest.clone(),
                    size: size_now,
                    mtime_ns,
                })?;
                ledger.verified(id, &rel)?;
                seen.insert(digest.clone());
                report.imported += 1;
                report.verified += 1;
                report.bytes_imported += size as u64;
            }
            Err(e) => {
                report.failed += 1;
                ledger.fail_item(id, &rel, &e.to_string())?;
            }
        }
    }
    if let Some(id) = id {
        ledger.finish(id)?;
    }
    // Duplicate lookup proves a matching digest was indexed, not that a copy still
    // exists on a mounted volume. Never destroy the only remaining staged copy.
    if opts.prune
        && report.failed == 0
        && report.refused == 0
        && report.duplicates == 0
        && report.imported == report.verified
    {
        fs::remove_dir_all(&originals)?;
        report.pruned = true;
    }
    Ok(report)
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::index;

    #[test]
    fn csv_quotes_and_newlines() {
        assert_eq!(
            csv_records("SourceFile,DateTimeOriginal\n\"x,\"\"y\n.jpg\",2020-01-01\n").unwrap()[1]
                [0],
            "x,\"y\n.jpg"
        );
    }

    #[test]
    fn new_duplicate_collision_and_verification_gate() {
        let root = std::env::temp_dir().join(format!("media-ingest-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(library.join("by-date/2020-01")).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(library.join("by-date/2020-01/20200102.jpg"), b"old!").unwrap();
        fs::write(staging.join("originals/20200102.jpg"), b"new!").unwrap();
        fs::write(staging.join("originals/duplicate.jpg"), b"old!").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        let db = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&db, &library, "scratch", "scratch").unwrap();
        let mut opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: false,
            fail_before_verify: true,
        };
        let failed = ingest(&db, &opts).unwrap();
        assert_eq!(failed.failed, 1);
        assert_eq!(failed.verified, 0);
        {
            let conn = rusqlite::Connection::open(root.join("db.sqlite")).unwrap();
            let (disposition, verified): (String, i64) = conn.query_row(
                "SELECT disposition,verified FROM media_ingest_items WHERE relpath='20200102.jpg' ORDER BY ingest_id DESC LIMIT 1",
                [], |row| Ok((row.get(0)?, row.get(1)?))
            ).unwrap();
            assert_eq!((disposition.as_str(), verified), ("failed", 0));
        }
        assert!(staging.join("originals/20200102.jpg").exists());
        opts.fail_before_verify = false;
        let again = ingest(&db, &opts).unwrap();
        assert_eq!(again.resumed, 1);
        assert_eq!(again.duplicates, 1);
        assert_eq!(
            hash(&library.join("by-date/2020-01/20200102.jpg")).unwrap(),
            hash(&staging.join("originals/duplicate.jpg")).unwrap()
        );
        assert!(library.join("by-date/2020-01/20200102~2.jpg").exists());
        let repeat = ingest(&db, &opts).unwrap();
        assert_eq!(repeat.resumed, 1);
        assert_eq!(repeat.imported, 0);
        fs::remove_dir_all(&root).unwrap();
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::store::{Ledger, Location, Outcome, Result};
use crate::{copy_checked, files_with_refusals, hash, stamp, Refusal};

/// A staged row per readable path, plus the paths that could not be read.
type StagedHashes = (Vec<(String, PathBuf, String, i64)>, Vec<Refusal>);

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
    pub incoming_unresolved: Vec<String>,
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
    let output = match Command::new("exiftool")
        .args([
            "-csv",
            "-r",
            "-SourceFile",
            "-DateTimeOriginal",
            "-d",
            "%Y-%m-%dT%H:%M:%S",
        ])
        .arg(root)
        .output()
    {
        Ok(output) => output,
        // exiftool is the first rung of the ladder, not a precondition for ingest. A machine
        // without it dates by filename and mtime exactly as a file with no EXIF would, so an
        // empty map is the honest answer and the ladder already handles it.
        //
        // It is also a host tool, and CI (ubuntu-latest) does not install it: requiring it here
        // made a test that calls itself hermetic depend on the machine it ran on. It passed
        // locally and failed in CI, which is the whole tell.
        //
        // Recorded rather than silent, because the fallback changes the date of every file it
        // touches. Correct the date on ingest, not in a migration afterwards.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "media: exiftool is not on PATH; dating this batch by filename and mtime. \
                 Install exiftool for EXIF dates."
            );
            return Ok(BTreeMap::new());
        }
        Err(e) => return Err(e.into()),
    };
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
) -> Result<StagedHashes> {
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
    let result = (|| -> Result<StagedHashes> {
        let mut rows = Vec::new();
        let mut refused = Vec::new();
        writeln!(tsv, "sha256\tsize\tmtime_ns\trelpath_json")?;
        for (rel, path) in paths {
            // A file this run cannot read is refused, not fatal: one unreadable path
            // must not deny every other path its disposition.
            let (size, mtime) = match stamp(path) {
                Ok(v) => v,
                Err(e) => {
                    refused.push(Refusal {
                        label: rel.clone(),
                        reason: format!("unreadable: {e}"),
                    });
                    continue;
                }
            };
            let digest = match hash(path) {
                Ok(d) => d,
                Err(e) => {
                    refused.push(Refusal {
                        label: rel.clone(),
                        reason: format!("unreadable: {e}"),
                    });
                    continue;
                }
            };
            // A file that changes mid-hash is a race, not a refusal; stop the batch.
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
        Ok((rows, refused))
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

// Never traverse a substituted directory or accept a symlink as an imported file.
fn checked_destination(library: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = library.to_path_buf();
    for part in Path::new(relative).components() {
        let std::path::Component::Normal(part) = part else {
            return Err("import destination is not a bounded relative path".into());
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("symlink in import destination".into())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

fn verified_bytes(path: &Path, digest: &str, size: i64) -> Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || meta.len() != size as u64 {
        return Ok(false);
    }
    let before = stamp(path)?;
    Ok(hash(path)? == digest && stamp(path)? == before)
}

fn remove_owned_copy(directory: &Path) -> Result<()> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err("incoming copy directory was substituted".into());
    }
    let entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    if entries.iter().any(|e| e.file_name() != "copy") {
        return Err("unexpected contents in owned incoming directory".into());
    }
    if let Some(entry) = entries.first() {
        if !entry.file_type()?.is_file() {
            return Err("incoming copy is not a regular file".into());
        }
        fs::remove_file(entry.path())?;
    }
    fs::remove_dir(directory)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Created,
    Owned,
    Copied,
    Reserved,
    Linked,
    Recorded,
    Verified,
    BeforePrune,
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
    ingest_at_boundaries(ledger, opts, &mut |_| Ok(()))
}

fn ingest_at_boundaries(
    ledger: &Ledger,
    opts: &IngestOptions<'_>,
    boundary: &mut impl FnMut(Boundary) -> Result<()>,
) -> Result<IngestReport> {
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
    let (paths, enumeration_refusals) = files_with_refusals(&originals)?;
    if refusals
        .keys()
        .any(|key| !paths.iter().any(|(rel, _)| rel == key))
    {
        return Err("manifest refusal names a file absent from originals".into());
    }
    let (rows, unreadable_refusals) =
        staged_hashes(&originals, &paths, &staging.join("staging-hashes.tsv"))?;
    let dates = exif_dates(&originals)?;
    let incoming = library
        .parent()
        .ok_or("library has no parent")?
        .join(".media-incoming");
    let mut report = IngestReport::default();
    // The OS releases this lock even when the process dies. Recovery must not race a live copy.
    let _lock = if opts.apply {
        fs::create_dir_all(&incoming)?;
        if !fs::symlink_metadata(&incoming)?.is_dir() {
            return Err("incoming must be a directory, not a symlink".into());
        }
        let lock_path = incoming.join(".lock");
        if let Ok(meta) = fs::symlink_metadata(&lock_path) {
            if !meta.file_type().is_file() {
                return Err("incoming lock must be a regular file".into());
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock()
            .map_err(|_| "another media ingest is running")?;
        for (name, rel, digest) in ledger.incoming_copies(
            &staging.to_string_lossy(),
            opts.uuid,
            &library.to_string_lossy(),
        )? {
            if Path::new(&name).components().count() != 1 || !name.starts_with("copy-") {
                continue;
            }
            let directory = incoming.join(name);
            if fs::symlink_metadata(&directory).is_ok()
                && rows.iter().any(|(r, _, d, _)| *r == rel && *d == digest)
            {
                // Originals were hashed above; do not remove anything if their bytes no longer agree.
                let source = rows
                    .iter()
                    .find(|(r, _, d, _)| *r == rel && *d == digest)
                    .ok_or("source missing")?;
                if verified_bytes(&source.1, &digest, source.3)? {
                    // An unfamiliar child or substitution is preserved and reported below.
                    if remove_owned_copy(&directory).is_ok() {
                        fs::File::open(&incoming)?.sync_all()?;
                    }
                }
            }
        }
        Some(lock)
    } else {
        None
    };
    if incoming.exists() {
        if !fs::symlink_metadata(&incoming)?.is_dir() {
            return Err("incoming must be a directory".into());
        }
        for entry in fs::read_dir(&incoming)? {
            let entry = entry?;
            if entry.file_name() != ".lock" {
                report
                    .incoming_unresolved
                    .push(entry.path().to_string_lossy().into_owned());
            }
        }
        report.incoming_unresolved.sort();
    }
    let id = if opts.apply {
        Some(ledger.start_ingest(source, &staging.to_string_lossy(), &manifest)?)
    } else {
        None
    };
    let mut seen = BTreeSet::new();
    // A path the walk could not name or read is recorded, not dropped: a run that
    // silently loses one file is the failure this gate exists to prevent. Counted as
    // considered because it was looked at, and refused because it got no row.
    for refusal in enumeration_refusals
        .iter()
        .chain(unreadable_refusals.iter())
    {
        report.considered += 1;
        report.refused += 1;
        if let Some(id) = id {
            ledger.item(
                id,
                &refusal.label,
                None,
                None,
                Outcome::new("refused", &refusal.reason, None, false),
            )?;
        }
    }
    let prune_rows = rows
        .iter()
        .map(|(rel, _, digest, size)| (rel.clone(), (digest.clone(), *size)))
        .collect::<BTreeMap<_, _>>();
    for (rel, path, digest, size) in rows {
        report.considered += 1;
        let prior = ledger.prior_verified(
            &staging.to_string_lossy(),
            opts.uuid,
            &library.to_string_lossy(),
            &rel,
            &digest,
        )?;
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
            let dest = checked_destination(&library, &prior_rel)?;
            if verified_bytes(&dest, &digest, size)? {
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
        if let Some(pending) = ledger.prior_unverified(
            &staging.to_string_lossy(),
            opts.uuid,
            &library.to_string_lossy(),
            &rel,
            &digest,
        )? {
            let dest = checked_destination(&library, &pending)?;
            if verified_bytes(&dest, &digest, size)? {
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
            if fs::symlink_metadata(&dest).is_ok() {
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
        let directory = (0..10000)
            .find_map(|n| {
                let directory = incoming.join(format!("copy-{id}-{}-{n}", report.considered));
                match fs::create_dir(&directory) {
                    Ok(()) => Some(Ok(directory)),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .ok_or("no free incoming directory")??;
        boundary(Boundary::Created)?;
        let incoming_name = directory
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("incoming name invalid")?;
        ledger.own_incoming(
            id,
            &rel,
            opts.uuid,
            &library.to_string_lossy(),
            incoming_name,
        )?;
        fs::File::open(&incoming)?.sync_all()?;
        boundary(Boundary::Owned)?;
        let temp = directory.join("copy");
        let copied = copy_checked(&path, &temp, &digest);
        if let Err(e) = copied {
            report.failed += 1;
            ledger.fail_item(id, &rel, &e.to_string())?;
            // Leave partial bytes with their durable owner for a checked retry.
            continue;
        }
        fs::File::open(&directory)?.sync_all()?;
        boundary(Boundary::Copied)?;
        let mut work: Result<(String, PathBuf)> =
            Err("no collision-free file name after 10000 attempts".into());
        for ordinal in 1..=10000 {
            let (dest_rel, _) = candidate(&library, &folder, name, ordinal);
            let dest = checked_destination(&library, &dest_rel)?;
            fs::create_dir_all(dest.parent().ok_or("import has no parent")?)?;
            ledger.reserve_path(id, &rel, &dest_rel)?;
            boundary(Boundary::Reserved)?;
            match fs::hard_link(&temp, &dest) {
                Ok(()) => {
                    fs::File::open(dest.parent().ok_or("import has no parent")?)?.sync_all()?;
                    boundary(Boundary::Linked)?;
                    work = Ok((dest_rel, dest));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    work = Err(e.into());
                    break;
                }
            }
        }
        remove_owned_copy(&directory)?;
        fs::File::open(&incoming)?.sync_all()?;
        match work {
            Ok((dest_rel, dest)) => {
                // The reservation stays unverified until the final path has been read back.
                if opts.fail_before_verify {
                    ledger.fail_item(id, &rel, "verification interrupted")?;
                    report.failed += 1;
                    continue;
                }
                if !verified_bytes(&dest, &digest, size)? {
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
                boundary(Boundary::Recorded)?;
                ledger.verified(id, &rel)?;
                boundary(Boundary::Verified)?;
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
        && report.incoming_unresolved.is_empty()
        && report.failed == 0
        && report.refused == 0
        && report.duplicates == 0
        && report.imported == report.verified
    {
        boundary(Boundary::BeforePrune)?;
        if !fs::symlink_metadata(&originals)?.is_dir() {
            return Err("staging changed; prune refused".into());
        }
        let (current, refused) = files_with_refusals(&originals)?;
        if !refused.is_empty() || current.len() != prune_rows.len() {
            return Err("staging file set changed; prune refused".into());
        }
        for (rel, path) in current {
            let Some((digest, size)) = prune_rows.get(&rel) else {
                return Err("staging file set changed; prune refused".into());
            };
            if !verified_bytes(&path, digest, *size)? {
                return Err("staged bytes changed; prune refused".into());
            }
        }
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
    fn every_boundary_recovers_without_losing_an_earlier_item() {
        for stop in [
            Boundary::Owned,
            Boundary::Copied,
            Boundary::Reserved,
            Boundary::Linked,
            Boundary::Recorded,
            Boundary::Verified,
        ] {
            let root = std::env::temp_dir()
                .join(format!("media-boundary-{}-{stop:?}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let library = root.join("library");
            let staging = root.join("staging");
            fs::create_dir_all(&library).unwrap();
            fs::create_dir_all(staging.join("originals")).unwrap();
            fs::write(staging.join("originals/20200101.jpg"), b"first").unwrap();
            fs::write(staging.join("originals/20200102.jpg"), b"second").unwrap();
            fs::write(
                staging.join("export-manifest.json"),
                r#"{"source":"boundary fixture"}"#,
            )
            .unwrap();
            let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
            index(&ledger, &library, "scratch", "scratch").unwrap();
            let opts = IngestOptions {
                staging: &staging,
                library: &library,
                uuid: "scratch",
                apply: true,
                prune: false,
                fail_before_verify: false,
            };
            let mut first_verified = false;
            let interrupted = ingest_at_boundaries(&ledger, &opts, &mut |at| {
                if first_verified && at == stop {
                    return Err("synthetic process interruption".into());
                }
                if at == Boundary::Verified {
                    first_verified = true;
                }
                Ok(())
            });
            assert!(interrupted.is_err(), "{stop:?}");
            let first = library.join("by-date/2020-01/20200101.jpg");
            assert_eq!(fs::read(&first).unwrap(), b"first");
            let recovered = ingest(&ledger, &opts).unwrap();
            assert_eq!(recovered.failed, 0, "{stop:?}");
            assert!(recovered.incoming_unresolved.is_empty(), "{stop:?}");
            assert_eq!(recovered.imported + recovered.resumed, 2, "{stop:?}");
            assert_eq!(
                fs::read(library.join("by-date/2020-01/20200102.jpg")).unwrap(),
                b"second"
            );
            assert_eq!(ledger.locations("scratch").unwrap().len(), 2);
            assert_eq!(crate::files(&library).unwrap().len(), 2);
            assert_eq!(
                fs::read(staging.join("originals/20200102.jpg")).unwrap(),
                b"second"
            );
            let repeat = ingest(&ledger, &opts).unwrap();
            assert_eq!(repeat.resumed, 2, "{stop:?}: idempotent retry");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn prune_rechecks_staged_bytes_and_the_complete_file_set() {
        for add_file in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "media-prune-race-{}-{add_file}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let library = root.join("library");
            let staging = root.join("staging");
            fs::create_dir_all(&library).unwrap();
            fs::create_dir_all(staging.join("originals")).unwrap();
            fs::write(staging.join("originals/20200102.jpg"), b"original").unwrap();
            fs::write(
                staging.join("export-manifest.json"),
                r#"{"source":"fixture"}"#,
            )
            .unwrap();
            let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
            index(&ledger, &library, "scratch", "scratch").unwrap();
            let opts = IngestOptions {
                staging: &staging,
                library: &library,
                uuid: "scratch",
                apply: true,
                prune: true,
                fail_before_verify: false,
            };
            let result = ingest_at_boundaries(&ledger, &opts, &mut |at| {
                if at == Boundary::BeforePrune {
                    let name = if add_file {
                        "added.jpg"
                    } else {
                        "20200102.jpg"
                    };
                    fs::write(staging.join("originals").join(name), b"new bytes")?;
                }
                Ok(())
            });
            assert!(result.unwrap_err().to_string().contains("prune refused"));
            assert!(staging.join("originals").is_dir());
            assert_eq!(
                fs::read(staging.join("originals").join(if add_file {
                    "added.jpg"
                } else {
                    "20200102.jpg"
                }))
                .unwrap(),
                b"new bytes"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn unjournaled_directory_is_reported_and_blocks_prune() {
        let root = std::env::temp_dir().join(format!("media-unowned-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(staging.join("originals/20200102.jpg"), b"source").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&ledger, &library, "scratch", "scratch").unwrap();
        let opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: true,
            fail_before_verify: false,
        };
        assert!(ingest_at_boundaries(&ledger, &opts, &mut |at| {
            if at == Boundary::Created {
                Err("crash before ownership journal".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        let unowned = root.join(".media-incoming/copy-1-1-0");
        let report = ingest(&ledger, &opts).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            report.incoming_unresolved,
            vec![unowned
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()]
        );
        assert!(!report.pruned);
        assert!(unowned.is_dir());
        assert_eq!(
            fs::read(staging.join("originals/20200102.jpg")).unwrap(),
            b"source"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_killed_process_releases_the_lock_and_recovers_its_link() {
        const CHILD_ROOT: &str = "SJEL_MEDIA_KILL_FIXTURE";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let library = root.join("library");
            let staging = root.join("staging");
            let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
            let opts = IngestOptions {
                staging: &staging,
                library: &library,
                uuid: "scratch",
                apply: true,
                prune: false,
                fail_before_verify: false,
            };
            let _ = ingest_at_boundaries(&ledger, &opts, &mut |at| {
                if at == Boundary::Linked {
                    fs::write(root.join("linked"), b"ready").unwrap();
                    loop {
                        std::thread::park();
                    }
                }
                Ok(())
            });
            panic!("child did not reach link boundary");
        }
        let root = std::env::temp_dir().join(format!("media-killed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(staging.join("originals/20200102.jpg"), b"source").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&ledger, &library, "scratch", "scratch").unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ingest::db_tests::a_killed_process_releases_the_lock_and_recovers_its_link",
            ])
            .env(CHILD_ROOT, &root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("linked").exists() && std::time::Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let linked = root.join("linked").exists();
        child.kill().ok();
        child.wait().unwrap();
        assert!(linked, "child must reach the synced final-link boundary");
        let opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: false,
            fail_before_verify: false,
        };
        let report = ingest(&ledger, &opts).unwrap();
        assert_eq!(report.resumed, 1);
        assert_eq!(report.failed, 0);
        assert!(report.incoming_unresolved.is_empty());
        assert_eq!(
            fs::read(library.join("by-date/2020-01/20200102.jpg")).unwrap(),
            b"source"
        );
        assert_eq!(ledger.locations("scratch").unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_preserves_substitutions_and_uses_the_target_volume() {
        let root = std::env::temp_dir().join(format!("media-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(staging.join("originals/20200102.jpg"), b"source").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&ledger, &library, "scratch", "scratch").unwrap();
        let mut opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: false,
            fail_before_verify: false,
        };
        assert!(ingest_at_boundaries(&ledger, &opts, &mut |at| {
            if at == Boundary::Owned {
                Err("synthetic interruption".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        let owned = root.join(".media-incoming/copy-1-1-0");
        fs::write(owned.join("copy"), b"partial").unwrap();
        opts.uuid = "another-volume";
        ledger.register(opts.uuid, "scratch").unwrap();
        let report = ingest(&ledger, &opts).unwrap();
        assert_eq!(report.incoming_unresolved.len(), 1);
        assert_eq!(fs::read(owned.join("copy")).unwrap(), b"partial");
        // Unfamiliar children prevent cleanup even when the source/target owns the directory.
        fs::write(owned.join("unknown"), b"keep").unwrap();
        opts.uuid = "scratch";
        let report = ingest(&ledger, &opts).unwrap();
        assert_eq!(report.incoming_unresolved.len(), 1);
        assert_eq!(fs::read(owned.join("unknown")).unwrap(), b"keep");
        fs::remove_file(owned.join("unknown")).unwrap();
        let report = ingest(&ledger, &opts).unwrap();
        assert!(report.incoming_unresolved.is_empty());
        assert!(
            !owned.exists(),
            "owned partial copy is removed after checking source bytes"
        );
        let lock = fs::File::open(root.join(".media-incoming/.lock")).unwrap();
        lock.try_lock().unwrap();
        assert!(
            ingest(&ledger, &opts).is_err(),
            "a live ingest excludes recovery"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_incoming_and_invalid_final_bytes_are_preserved() {
        let root = std::env::temp_dir().join(format!("media-unknown-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(staging.join("originals/20200102.jpg"), b"original").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        let ledger = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&ledger, &library, "scratch", "scratch").unwrap();
        let opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: true,
            fail_before_verify: false,
        };
        assert!(ingest_at_boundaries(&ledger, &opts, &mut |at| {
            if at == Boundary::Linked {
                Err("synthetic interruption".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        let dest = library.join("by-date/2020-01/20200102.jpg");
        fs::write(&dest, b"tampered").unwrap();
        let unknown = root.join(".media-incoming/unknown");
        fs::write(&unknown, b"unowned bytes").unwrap();
        let report = ingest(&ledger, &opts).unwrap();
        assert_eq!(report.failed, 1);
        assert!(!report.pruned);
        assert_eq!(fs::read(&dest).unwrap(), b"tampered");
        assert_eq!(fs::read(unknown).unwrap(), b"unowned bytes");
        assert_eq!(report.incoming_unresolved.len(), 1);
        assert!(ledger.locations("scratch").unwrap().is_empty());
        assert_eq!(
            fs::read(staging.join("originals/20200102.jpg")).unwrap(),
            b"original"
        );
        fs::remove_file(&dest).unwrap();
        std::os::unix::fs::symlink(staging.join("originals/20200102.jpg"), &dest).unwrap();
        assert!(
            ingest(&ledger, &opts).is_err(),
            "a matching symlink is never adopted"
        );
        assert_eq!(
            fs::read(staging.join("originals/20200102.jpg")).unwrap(),
            b"original"
        );
        fs::remove_dir_all(root).unwrap();
    }

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

    /// The F1 falsifier: one path the walk cannot name must not deny every other path
    /// its disposition. A symlink is refused and recorded; the good file still imports.
    #[test]
    fn a_refused_path_does_not_deny_the_rest_its_disposition() {
        let root =
            std::env::temp_dir().join(format!("media-ingest-refusal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = root.join("staging");
        fs::create_dir_all(library.join("by-date/2020-01")).unwrap();
        fs::create_dir_all(staging.join("originals")).unwrap();
        fs::write(staging.join("originals/good.jpg"), b"good!").unwrap();
        fs::write(
            staging.join("export-manifest.json"),
            r#"{"source":"fixture"}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(
            staging.join("originals/good.jpg"),
            staging.join("originals/link.jpg"),
        )
        .unwrap();
        let db = Ledger::open(&root.join("db.sqlite")).unwrap();
        index(&db, &library, "scratch", "scratch").unwrap();
        let opts = IngestOptions {
            staging: &staging,
            library: &library,
            uuid: "scratch",
            apply: true,
            prune: false,
            fail_before_verify: false,
        };
        let report = ingest(&db, &opts).unwrap();
        assert_eq!(report.imported, 1, "the good file still imports");
        assert_eq!(report.refused, 1, "the symlink is refused, not fatal");
        assert_eq!(report.considered, 2, "both paths were looked at");
        let conn = rusqlite::Connection::open(root.join("db.sqlite")).unwrap();
        let refused: i64 = conn
            .query_row(
                "SELECT count(*) FROM media_ingest_items WHERE disposition='refused'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refused, 1, "the refused path has a durable row");
        fs::remove_dir_all(&root).unwrap();
    }
}

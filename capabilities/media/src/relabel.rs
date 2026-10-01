//! File-level organisation: move a directory's files into a library folder, renaming each to begin
//! with its capture date.
//!
//! This is the file-granularity sibling of `organize`, which moves whole collections. It exists
//! because some material arrives as a flat pile with no collection name — `Inbox-old` is 198 drone
//! clips and phone videos in one directory — and the only honest structure for it is a date read
//! from the file itself.
//!
//! The differences from `organize` are deliberate, not accidental:
//!
//! * **A refusal is per file, not fatal.** One unreadable clip must not block the other 197, so a
//!   file that cannot be placed is reported and skipped. A *structural* failure — the source is
//!   missing, the destination is not a real directory, a symlink is in the tree — still refuses the
//!   whole run, because then the plan does not describe the filesystem.
//! * **A name collision is resolved, not refused.** The library's existing disambiguator is `~2`,
//!   and a rename that would overwrite is never taken.
//! * **No date means no invented date.** A file whose metadata holds no usable capture date keeps
//!   its name unchanged and is counted as undated. It is not filed under a guessed month.
//!
//! Nothing is copied and nothing is deleted: every move is a same-volume rename, journalled, and
//! reversed by swapping a row's two paths.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::Result;

#[derive(Debug, Serialize)]
pub struct FileMove {
    pub from: PathBuf,
    pub to: PathBuf,
    pub size: u64,
    /// The embedded capture day, when one was readable.
    pub day: Option<String>,
    /// The metadata field the day came from, so the choice can be checked.
    pub day_field: Option<String>,
    pub outcome: &'static str,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelabelReport {
    pub applied: bool,
    pub complete: bool,
    pub from: PathBuf,
    pub to: PathBuf,
    pub considered: usize,
    pub moved: usize,
    pub bytes: u64,
    /// Files whose name gained a day prefix.
    pub dated: usize,
    /// Files with no usable capture date; their names are untouched.
    pub undated: usize,
    /// Files that kept a `~N` because the plain name was taken.
    pub disambiguated: usize,
    pub refused: usize,
    pub journal: PathBuf,
    pub moves: Vec<FileMove>,
    pub issues: Vec<String>,
    pub limitations: Vec<&'static str>,
}

const LIMITATIONS: [&str; 5] = [
    "Every move is a rename within one volume; no file is copied, hashed or deleted.",
    "A file with no usable embedded capture date keeps its name and is counted as undated rather than filed under a guessed date.",
    "The day comes from the same ExifTool field precedence the preview uses, so the two verbs cannot disagree about what a capture date is.",
    "A refusal is per file: one unplaceable file is reported and skipped, and the rest proceed. A missing source, an unusable destination or a symlink in the tree refuses the whole run.",
    "A move is not a duplicate verdict. Run `media duplicates` afterwards.",
];

/// A path under `root` that must be a regular file, refusing traversal and a symlink anywhere.
fn member(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains(['\\', '\0'])
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
    {
        return Err(format!("unsafe path in the plan: {relative}").into());
    }
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path).map_err(|error| format!("{relative}: {error}"))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!("not a regular file: {relative}").into());
    }
    Ok(path)
}

/// A path under `root` that must not exist yet, with every existing ancestor a real directory.
fn member_free(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains(['\\', '\0'])
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
    {
        return Err(format!("unsafe destination in the plan: {relative}").into());
    }
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(format!("destination already exists: {relative}").into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = path.parent().ok_or("destination has no parent")?;
    crate::preview::real_directory(parent)?;
    Ok(path)
}

/// A resolved move: source, destination, size, the capture day and its field, and the ordinal the
/// library's disambiguator settled on.
type Planned = (PathBuf, PathBuf, u64, Option<(String, String)>, usize);

/// Every regular file under `root`, relative path and all, refusing a symlink anywhere.
fn walk(root: &Path) -> Result<Vec<(String, PathBuf, u64)>> {
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                return Err(format!("symlink in the source tree: {}", path.display()).into());
            }
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_file() {
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF-8 source path")?
                    .to_owned();
                found.push((relative, path, meta.len()));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// The name a file should carry, given its capture day. A name that already begins with the day is
/// left alone, so re-running the same verb cannot stack prefixes.
fn target_name(original: &str, day: Option<&str>) -> String {
    match day {
        Some(day) if !original.starts_with(&format!("{day}_")) => format!("{day}_{original}"),
        _ => original.to_owned(),
    }
}

/// The library's disambiguator: `name.ext`, then `name~2.ext`, `name~3.ext`.
fn disambiguate(directory: &Path, name: &str, ordinal: usize) -> PathBuf {
    if ordinal <= 1 {
        return directory.join(name);
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let renamed = match path.extension().and_then(|s| s.to_str()) {
        Some(extension) => format!("{stem}~{ordinal}.{extension}"),
        None => format!("{stem}~{ordinal}"),
    };
    directory.join(renamed)
}

pub fn relabel(
    from: &Path,
    to: &Path,
    journal: &Path,
    apply: bool,
    settled_for: i64,
    plan: Option<&Path>,
) -> Result<RelabelReport> {
    let from = from.canonicalize()?;
    let to = to.canonicalize()?;
    if from == to || to.starts_with(&from) || from.starts_with(&to) {
        return Err("source and destination must be distinct and not nested".into());
    }
    crate::preview::real_directory(&from)?;
    crate::preview::real_directory(&to)?;

    let files = match plan {
        // Plan mode: the caller has already decided every destination, so nothing is walked and no
        // date is read. The plan is an approved artefact, like a removal list, and a plan that does
        // not describe the disk refuses the whole run rather than moving the rows that happen to fit.
        Some(_) => Vec::new(),
        None => walk(&from)?,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs() as i64;
    let mut report = RelabelReport {
        applied: apply,
        complete: true,
        from: from.clone(),
        to: to.clone(),
        considered: files.len(),
        moved: 0,
        bytes: 0,
        dated: 0,
        undated: 0,
        disambiguated: 0,
        refused: 0,
        journal: journal.to_path_buf(),
        moves: Vec::new(),
        issues: Vec::new(),
        limitations: LIMITATIONS.to_vec(),
    };

    // The newest write decides whether the pile is settled; a tree still being written must not be
    // reorganised, which is the lesson of 2026-09-30.
    let newest = files
        .iter()
        .filter_map(|(_, path, _)| fs::metadata(path).ok())
        .filter_map(|meta| meta.modified().ok())
        .filter_map(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_secs() as i64)
        .max()
        .unwrap_or(0);
    let quiet = now - newest;
    if quiet < settled_for {
        report.complete = false;
        report.issues.push(format!(
            "the source was written {quiet}s ago and the quiet window is {settled_for}s; nothing was moved"
        ));
        return Ok(report);
    }

    let relative: Vec<String> = files.iter().map(|(rel, _, _)| rel.clone()).collect();
    let days = if plan.is_some() {
        BTreeMap::new()
    } else {
        crate::preview::capture_days(&from, &relative)?
    };

    let mut plans: Vec<Planned> = Vec::new();
    // Every destination resolved so far, so a name taken twice within this same run is seen.
    let mut taken: BTreeMap<PathBuf, ()> = BTreeMap::new();
    if let Some(path) = plan {
        for (index, line) in std::io::BufReader::new(fs::File::open(path)?)
            .lines()
            .enumerate()
        {
            let line = line?;
            if index == 0 {
                if !line.starts_with("from\tto") {
                    return Err("a relabel plan must start with its `from`/`to` header row".into());
                }
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 2 {
                return Err(format!("plan row {} needs exactly two fields", index + 1).into());
            }
            let (source_rel, target_rel) = (fields[0], fields[1]);
            let source = member(&from, source_rel)?;
            let target = member_free(&to, target_rel)?;
            if taken.contains_key(&target) {
                return Err(format!("two plan rows share the destination {target_rel}").into());
            }
            taken.insert(target.clone(), ());
            let size = fs::metadata(&source)?.len();
            plans.push((source, target, size, None, 1));
        }
        if plans.is_empty() {
            return Err("the relabel plan has no rows".into());
        }
    }
    for (rel, path, size) in files {
        let original = Path::new(&rel)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&rel)
            .to_owned();
        let found = days.get(&rel).cloned();
        let name = target_name(&original, found.as_ref().map(|(day, _)| day.as_str()));
        if found.is_some() {
            report.dated += 1;
        } else {
            report.undated += 1;
        }
        let mut ordinal = 1;
        let mut target = disambiguate(&to, &name, ordinal);
        while target.exists() || taken.contains_key(&target) {
            ordinal += 1;
            if ordinal > 9999 {
                report.refused += 1;
                report
                    .issues
                    .push(format!("no free name for {rel} after 9999 attempts"));
                break;
            }
            target = disambiguate(&to, &name, ordinal);
        }
        if ordinal > 1 {
            report.disambiguated += 1;
        }
        if report.refused > 0 {
            continue;
        }
        taken.insert(target.clone(), ());
        plans.push((path, target, size, found, ordinal));
    }

    for (source, target, size, found, _) in &plans {
        let (day, field) = match found {
            Some((day, field)) => (Some(day.clone()), Some(field.clone())),
            None => (None, None),
        };
        let mut row = FileMove {
            from: source.clone(),
            to: target.clone(),
            size: *size,
            day,
            day_field: field,
            outcome: if apply { "moved" } else { "would-move" },
            reason: None,
        };
        if apply {
            if let Err(error) = fs::rename(source, target) {
                row.outcome = "failed";
                row.reason = Some(error.to_string());
                report.complete = false;
                report
                    .issues
                    .push(format!("rename failed for {}: {error}", source.display()));
            } else {
                report.moved += 1;
                report.bytes += size;
            }
        }
        report.moves.push(row);
    }
    if !apply {
        return Ok(report);
    }

    let stamp = now.to_string();
    let fresh = !journal.exists();
    if let Some(parent) = journal.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut log = OpenOptions::new().append(true).create(true).open(journal)?;
    if fresh {
        writeln!(
            log,
            "moved_at\tfrom\tto\tsize\tday\tday_field"
        )?;
    }
    for row in report.moves.iter().filter(|row| row.outcome == "moved") {
        writeln!(
            log,
            "{stamp}\t{}\t{}\t{}\t{}\t{}",
            row.from.display(),
            row.to.display(),
            row.size,
            row.day.as_deref().unwrap_or(""),
            row.day_field.as_deref().unwrap_or("")
        )?;
    }
    log.sync_all()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media-relabel-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("from")).unwrap();
            fs::create_dir_all(root.join("to")).unwrap();
            Self(root)
        }
        fn source(&self) -> PathBuf {
            self.0.join("from")
        }
        fn target(&self) -> PathBuf {
            self.0.join("to")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_day_prefix_is_added_once_and_a_second_run_does_not_stack_it() {
        assert_eq!(target_name("DJI_0062.MP4", Some("2019-06-12")), "2019-06-12_DJI_0062.MP4");
        assert_eq!(
            target_name("2019-06-12_DJI_0062.MP4", Some("2019-06-12")),
            "2019-06-12_DJI_0062.MP4",
            "already prefixed"
        );
        assert_eq!(target_name("DJI_0062.MP4", None), "DJI_0062.MP4", "no invented date");
    }

    #[test]
    fn the_disambiguator_matches_the_library_convention() {
        let dir = Path::new("/x");
        assert_eq!(disambiguate(dir, "a.MP4", 1), dir.join("a.MP4"));
        assert_eq!(disambiguate(dir, "a.MP4", 2), dir.join("a~2.MP4"));
        assert_eq!(disambiguate(dir, "noext", 3), dir.join("noext~3"));
    }

    #[test]
    fn a_file_already_at_the_destination_name_is_disambiguated_rather_than_overwritten() {
        let fixture = Fixture::new();
        let source = fixture.source().join("clip.MP4");
        fs::write(&source, b"incoming").unwrap();
        let occupied = fixture.target().join("clip.MP4");
        fs::write(&occupied, b"already here").unwrap();
        // No metadata, so the name is unchanged and collides.
        let report = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            true,
            0,
            None,
        )
        .unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(report.disambiguated, 1);
        assert_eq!(fs::read(&occupied).unwrap(), b"already here", "never overwritten");
        assert_eq!(fs::read(fixture.target().join("clip~2.MP4")).unwrap(), b"incoming");
    }

    #[test]
    fn a_dry_run_moves_nothing_and_names_every_destination() {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("a.MP4"), b"one").unwrap();
        fs::write(fixture.source().join("b.MOV"), b"two").unwrap();
        let report = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            false,
            0,
            None,
        )
        .unwrap();
        assert_eq!(report.considered, 2);
        assert_eq!(report.moved, 0);
        assert!(report.moves.iter().all(|row| row.outcome == "would-move"));
        assert!(fixture.source().join("a.MP4").exists());
        assert_eq!(fs::read_dir(fixture.target()).unwrap().count(), 0);
        assert!(!fixture.0.join("j.tsv").exists());
    }

    #[test]
    fn a_source_written_inside_the_quiet_window_refuses_the_run() {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("fresh.MP4"), b"just written").unwrap();
        let report = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            true,
            3600,
            None,
        )
        .unwrap();
        assert!(!report.complete);
        assert_eq!(report.moved, 0);
        assert!(fixture.source().join("fresh.MP4").exists());
        assert!(report.issues[0].contains("quiet window"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_source_tree_refuses_the_whole_run() {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("real.MP4"), b"bytes").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", fixture.source().join("escape")).unwrap();
        let error = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            true,
            0,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("symlink"), "{error}");
    }

    #[test]
    fn a_plan_names_every_destination_and_is_journalled() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.target().join("Trips/2026/T")).unwrap();
        fs::create_dir_all(fixture.target().join("To Sort")).unwrap();
        fs::write(fixture.source().join("a.insv"), b"clip").unwrap();
        fs::write(fixture.source().join("b.insv"), b"other").unwrap();
        let plan = fixture.0.join("plan.tsv");
        fs::write(
            &plan,
            "from\tto\na.insv\tTrips/2026/T/a.insv\nb.insv\tTo Sort/b.insv\n",
        )
        .unwrap();
        let journal = fixture.0.join("j.tsv");
        let report = relabel(
            &fixture.source(),
            &fixture.target(),
            &journal,
            true,
            0,
            Some(&plan),
        )
        .unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.moved, 2);
        assert!(fixture.target().join("Trips/2026/T/a.insv").exists());
        assert!(fixture.target().join("To Sort/b.insv").exists());
        assert!(!fixture.source().join("a.insv").exists());
        let text = fs::read_to_string(&journal).unwrap();
        assert!(text.contains("To Sort/b.insv"));
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_plan_row_that_does_not_describe_the_disk_refuses_the_whole_run() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.target().join("T")).unwrap();
        fs::write(fixture.source().join("present.insv"), b"here").unwrap();
        let plan = fixture.0.join("plan.tsv");
        fs::write(
            &plan,
            "from\tto\npresent.insv\tT/present.insv\nmissing.insv\tT/missing.insv\n",
        )
        .unwrap();
        let error = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            true,
            0,
            Some(&plan),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("missing.insv"), "{error}");
        assert!(
            fixture.source().join("present.insv").exists(),
            "the row that fitted was not moved either"
        );
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_plan_never_overwrites_an_existing_destination() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.target().join("T")).unwrap();
        fs::write(fixture.source().join("a.insv"), b"incoming").unwrap();
        fs::write(fixture.target().join("T/a.insv"), b"already here").unwrap();
        let plan = fixture.0.join("plan.tsv");
        fs::write(&plan, "from\tto\na.insv\tT/a.insv\n").unwrap();
        let error = relabel(
            &fixture.source(),
            &fixture.target(),
            &fixture.0.join("j.tsv"),
            true,
            0,
            Some(&plan),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            fs::read(fixture.target().join("T/a.insv")).unwrap(),
            b"already here"
        );
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn nested_source_directories_are_flattened_and_journalled() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.source().join("nested")).unwrap();
        fs::write(fixture.source().join("nested/deep.MP4"), b"bytes").unwrap();
        let journal = fixture.0.join("j.tsv");
        let report = relabel(&fixture.source(), &fixture.target(), &journal, true, 0, None).unwrap();
        assert_eq!(report.moved, 1);
        assert!(fixture.target().join("deep.MP4").exists());
        let text = fs::read_to_string(&journal).unwrap();
        assert!(text.contains("nested/deep.MP4"), "the journal records where it came from");
    }
}

//! Guarded removal of superseded duplicates.
//!
//! The list this acts on is an approval artefact, produced by `media duplicates`: one row per
//! superseded path, naming the surviving copy that makes the removal safe. Two rules make it
//! different from `rm`:
//!
//! * **Every row is re-verified immediately before anything moves.** The surviving copy is re-read
//!   and must still hash to the row's digest, and the superseded copy must too. An index is a
//!   record of a past read; a deletion may not rest on one. If any row fails, nothing moves at all.
//! * **Nothing is deleted.** A superseded file is renamed into a quarantine directory on the same
//!   volume, so the removal is a rename, the journal reverses it, and reclaiming the space is a
//!   separate declared step taken after the mirror is verified.
//!
//! A digest's last location is never removed: the surviving copy is required to exist before the
//! superseded one is touched, so a group cannot lose its only copy.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::Result;

#[derive(Debug, Serialize)]
pub struct SupersedeReport {
    pub applied: bool,
    pub complete: bool,
    pub root: PathBuf,
    pub quarantine: PathBuf,
    pub rows: usize,
    pub verified: usize,
    pub quarantined: usize,
    /// Rows an earlier run of this same journal already moved. Re-running an approved list is safe:
    /// the file is not moved twice, and its index row is dropped again.
    pub already_quarantined: usize,
    pub bytes_quarantined: u64,
    pub refused: usize,
    /// Kept copies renamed to drop a macOS duplicate suffix, adopting the superseded copy's name.
    pub names_normalised: usize,
    /// Groups where the clean name was already taken by a different file, so the suffix stayed.
    pub name_collisions: usize,
    pub journal: PathBuf,
    /// The relpaths moved out, so the caller can drop their index rows. The index must not be left
    /// describing a file the volume no longer holds, and a disappearance is never pruned silently.
    pub quarantined_paths: Vec<String>,
    /// Surviving copies whose name changed. The index row for the old name must go too, and the
    /// next `index` records the new one — a rename is a path change like any other.
    pub renamed_paths: Vec<String>,
    pub issues: Vec<String>,
    pub limitations: Vec<&'static str>,
}

const LIMITATIONS: [&str; 5] = [
    "Every row is re-hashed before anything moves; a stale index is therefore not a reason for a deletion.",
    "A superseded file is renamed into quarantine on the same volume, not deleted, so the journal reverses it.",
    "Reclaiming the space means emptying the quarantine, which is a separate declared step.",
    "The surviving copy is required to exist and match its digest, so a digest never loses its last location.",
    "Re-running an approved list is idempotent: a row this journal already quarantined is reconciled, not moved again.",
];

/// The `from` column of an existing journal: the paths an earlier run already moved out.
/// One line of an existing journal: the superseded path, and the surviving path's previous name
/// when it was normalised. Re-running a list needs both — a rename is a path change too, and an
/// index row for a name that no longer exists is a stale row.
struct JournalRow {
    from: String,
    renamed_from: String,
}

fn journal_rows(journal: &Path) -> Result<Vec<JournalRow>> {
    let Ok(file) = fs::File::open(journal) else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 5 {
            rows.push(JournalRow {
                from: fields[3].to_owned(),
                renamed_from: fields.get(6).map_or(String::new(), |f| f.to_string()),
            });
        }
    }
    Ok(rows)
}

struct Row {
    digest: String,
    superseded: String,
    kept: String,
}

fn read_list(path: &Path) -> Result<Vec<Row>> {
    let file = fs::File::open(path)?;
    let mut rows = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 {
            if !line.starts_with("digest\tsize\tsuperseded\tkept") {
                return Err("removal list must start with its header row".into());
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(
                format!("removal list row {} has {} fields", index + 1, fields.len()).into(),
            );
        }
        rows.push(Row {
            digest: fields[0].to_owned(),
            superseded: fields[2].to_owned(),
            kept: fields[3].to_owned(),
        });
    }
    if rows.is_empty() {
        return Err("removal list has no rows".into());
    }
    Ok(rows)
}

/// A path under the root, refusing traversal, an absolute escape, and a symlink at any component.
fn member(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains(['\\', '\0'])
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
    {
        return Err(format!("unsafe path in removal list: {relative}").into());
    }
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!("not a regular file: {relative}").into());
    }
    Ok(path)
}

/// The clean name a suffixed copy should adopt, when the superseded copy holds it.
///
/// macOS appends ` 2`, ` 3`, ` (1)` when it will not overwrite, and an export can add both
/// (`DJI_0609 (1) 2.MP4`). Guessing from a pattern alone would be dangerous — `video 21.01.18, 14 34 37.mov`
/// ends in a number and duplicates nothing. So the suffix is only stripped when doing so reproduces
/// **exactly** the superseded copy's own name, which the removal list already proves is the same
/// bytes. The name is then the only difference, and the clean one can be adopted.
fn cleaned_name(stays: &Path, gone: &Path) -> Option<PathBuf> {
    let stays_name = stays.file_name()?.to_str()?;
    let gone_name = gone.file_name()?.to_str()?;
    let mut candidate = stays_name.to_owned();
    for _ in 0..8 {
        let next = strip_one_suffix(&candidate)?;
        if next == gone_name {
            return Some(stays.with_file_name(&next));
        }
        candidate = next;
    }
    None
}

/// Remove one trailing ` (N)` or ` N` from a file name, keeping the extension.
pub(crate) fn strip_one_suffix(name: &str) -> Option<String> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem.to_owned(), Some(extension.to_owned()))
        }
        _ => (name.to_owned(), None),
    };
    let shortened = if let Some(open) = stem.rfind(" (") {
        match stem[open + 2..].strip_suffix(')') {
            Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                Some(stem[..open].to_owned())
            }
            _ => None,
        }
    } else {
        None
    }
    .or_else(|| {
        let space = stem.rfind(' ')?;
        let digits = &stem[space + 1..];
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| stem[..space].to_owned())
    })?;
    Some(match extension {
        Some(extension) => format!("{shortened}.{extension}"),
        None => shortened,
    })
}

pub fn supersede(
    root: &Path,
    list: &Path,
    quarantine: &Path,
    journal: &Path,
    apply: bool,
) -> Result<SupersedeReport> {
    let root = root.canonicalize()?;
    let rows = read_list(list)?;
    let applied_journal = journal_rows(journal)?;
    let mut report = SupersedeReport {
        applied: apply,
        complete: true,
        root: root.clone(),
        quarantine: quarantine.to_path_buf(),
        rows: rows.len(),
        verified: 0,
        quarantined: 0,
        already_quarantined: 0,
        bytes_quarantined: 0,
        refused: 0,
        names_normalised: 0,
        name_collisions: 0,
        journal: journal.to_path_buf(),
        quarantined_paths: Vec::new(),
        renamed_paths: Vec::new(),
        issues: Vec::new(),
        limitations: LIMITATIONS.to_vec(),
    };

    // Phase one: verify every row. Nothing moves while any row is unproven.
    // (row, superseded path, surviving path, size, name to adopt, clean name already taken)
    let mut plans: Vec<(Row, PathBuf, PathBuf, u64, Option<PathBuf>, bool)> = Vec::new();
    for row in rows {
        let declared = root.join(&row.superseded);
        if let Some(done) = applied_journal
            .iter()
            .find(|entry| entry.from == declared.to_string_lossy())
            .filter(|_| !declared.exists())
        {
            // An earlier run of this same journal already moved it. Reconcile its index rows — both
            // the quarantined path and the name the surviving copy used to have — and move on, so
            // re-running an approved list is safe rather than a refusal.
            report.already_quarantined += 1;
            report.quarantined_paths.push(row.superseded.clone());
            if !done.renamed_from.is_empty() {
                if let Ok(relative) = Path::new(&done.renamed_from).strip_prefix(&root) {
                    if let Some(relative) = relative.to_str() {
                        report.renamed_paths.push(relative.to_owned());
                    }
                }
            }
            continue;
        }
        let gone = match member(&root, &row.superseded) {
            Ok(path) => path,
            Err(error) => {
                report.refused += 1;
                report.complete = false;
                report.issues.push(format!("{}: {error}", row.superseded));
                continue;
            }
        };
        let stays = match member(&root, &row.kept) {
            Ok(path) => path,
            Err(error) => {
                report.refused += 1;
                report.complete = false;
                report
                    .issues
                    .push(format!("{}: surviving copy unusable: {error}", row.kept));
                continue;
            }
        };
        for (label, path) in [("superseded", &gone), ("kept", &stays)] {
            let digest = match crate::hash(path) {
                Ok(digest) => digest,
                Err(error) => {
                    report.refused += 1;
                    report.complete = false;
                    report.issues.push(format!("{label} unreadable: {error}"));
                    continue;
                }
            };
            if digest != row.digest {
                report.refused += 1;
                report.complete = false;
                report.issues.push(format!(
                    "{label} no longer matches its recorded digest: {}",
                    path.strip_prefix(&root).unwrap_or(path).display()
                ));
            }
        }
        if report.refused > 0 {
            // A failure anywhere means the list does not describe the disk. Stop verifying rows and
            // refuse the whole run rather than moving the ones that happened to pass.
            continue;
        }
        report.verified += 1;
        let size = fs::metadata(&gone)?.len();
        // Decide the name here, in the phase that runs for a dry run too, so both modes report the
        // same numbers and the operator approves what will actually happen.
        let mut adopt = None;
        let mut collision = false;
        if let Some(clean) = cleaned_name(&stays, &gone) {
            if fs::symlink_metadata(&clean).is_err() {
                adopt = Some(clean);
                report.names_normalised += 1;
            } else {
                collision = true;
                report.name_collisions += 1;
                report.issues.push(format!(
                    "kept the suffixed name: {} is already present in that folder",
                    clean.display()
                ));
            }
        }
        plans.push((row, gone, stays, size, adopt, collision));
    }
    if report.refused > 0 {
        report
            .issues
            .push("nothing was moved: a row could not be proven against the disk".into());
        return Ok(report);
    }
    if !apply {
        report
            .issues
            .push("dry run: every row was re-verified; nothing was quarantined".into());
        return Ok(report);
    }

    // Phase two: rename each superseded file into quarantine. Same volume, so this is a rename.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs()
        .to_string();
    let batch = quarantine.join(&stamp);
    let fresh = !journal.exists();
    if let Some(parent) = journal.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut log = OpenOptions::new().append(true).create(true).open(journal)?;
    if fresh {
        writeln!(
            log,
            "quarantined_at\tdigest\tsize\tfrom\tto\tkept\tkept_renamed_from"
        )?;
    }
    for (row, gone, stays, size, adopt, collision) in plans {
        // Adopt the cleaner name before the copy holding it is quarantined. The decision was made
        // while verifying; here it is only carried out.
        let mut stays = stays;
        let mut renamed_from = String::new();
        if collision {
            report
                .issues
                .push(format!("kept the suffixed name for {}", stays.display()));
        }
        if let Some(clean) = adopt {
            match fs::rename(&stays, &clean) {
                Ok(()) => {
                    renamed_from = stays.display().to_string();
                    if let Some(old) = stays.strip_prefix(&root).ok().and_then(|p| p.to_str()) {
                        report.renamed_paths.push(old.to_owned());
                    }
                    stays = clean;
                }
                Err(error) => report.issues.push(format!(
                    "name normalisation failed for {}: {error}",
                    stays.display()
                )),
            }
        }
        let relative = gone.strip_prefix(&root)?.to_path_buf();
        let target = batch.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(error) = fs::rename(&gone, &target) {
            report.complete = false;
            report
                .issues
                .push(format!("rename failed for {}: {error}", relative.display()));
            continue;
        }
        report.quarantined += 1;
        report.bytes_quarantined += size;
        report.quarantined_paths.push(row.superseded.clone());
        writeln!(
            log,
            "{stamp}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.digest,
            size,
            gone.display(),
            target.display(),
            stays.display(),
            renamed_from
        )?;
    }
    log.sync_all()?;
    // The parent of a removed file may now be empty; leave it. An empty by-date bucket still names
    // the month and costs nothing, and removing directories is not this verb's business.
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
                "media-supersede-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("library/by-date/2018-03")).unwrap();
            fs::create_dir_all(root.join("library/Trips/2018/2018-03-Iceland")).unwrap();
            Self(root)
        }
        fn library(&self) -> PathBuf {
            self.0.join("library")
        }
        fn pair(&self, name: &str, bytes: &[u8]) -> String {
            let gone = self.library().join("by-date/2018-03").join(name);
            let stays = self.library().join("Trips/2018/2018-03-Iceland").join(name);
            fs::write(&gone, bytes).unwrap();
            fs::write(&stays, bytes).unwrap();
            crate::hash(&stays).unwrap()
        }
        fn list(&self, rows: &[(&str, &str, &str)]) -> PathBuf {
            let path = self.0.join("list.tsv");
            let mut text = String::from("digest\tsize\tsuperseded\tkept\n");
            for (digest, gone, stays) in rows {
                text.push_str(&format!("{digest}\t1\t{gone}\t{stays}\n"));
            }
            fs::write(&path, text).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_suffixed_kept_copy_adopts_the_clean_name_before_its_twin_is_quarantined() {
        let fixture = Fixture::new();
        let gone = fixture.library().join("by-date/2018-03/DJI_0653.MP4");
        let stays = fixture
            .library()
            .join("Trips/2018/2018-03-Iceland/DJI_0653 2.MP4");
        fs::write(&gone, b"same bytes").unwrap();
        fs::write(&stays, b"same bytes").unwrap();
        let digest = crate::hash(&stays).unwrap();
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/DJI_0653.MP4",
            "Trips/2018/2018-03-Iceland/DJI_0653 2.MP4",
        )]);
        let journal = fixture.0.join("j.tsv");
        let report = supersede(
            &fixture.library(),
            &list,
            &fixture.0.join("q"),
            &journal,
            true,
        )
        .unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.names_normalised, 1);
        assert_eq!(report.name_collisions, 0);
        assert_eq!(report.quarantined, 1);
        let clean = fixture
            .library()
            .join("Trips/2018/2018-03-Iceland/DJI_0653.MP4");
        assert!(clean.exists(), "the clean name was adopted");
        assert_eq!(fs::read(&clean).unwrap(), b"same bytes");
        assert!(!stays.exists(), "the suffixed name is gone");
        assert!(!gone.exists(), "the twin is quarantined");
        assert!(fs::read_to_string(&journal)
            .unwrap()
            .contains("DJI_0653 2.MP4"));
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_taken_clean_name_leaves_the_suffix_alone_and_is_reported() {
        let fixture = Fixture::new();
        let gone = fixture.library().join("by-date/2018-03/DJI_0653.MP4");
        let stays = fixture
            .library()
            .join("Trips/2018/2018-03-Iceland/DJI_0653 2.MP4");
        // A *different* file already holds the clean name in that folder.
        let occupied = fixture
            .library()
            .join("Trips/2018/2018-03-Iceland/DJI_0653.MP4");
        fs::write(&gone, b"same bytes").unwrap();
        fs::write(&stays, b"same bytes").unwrap();
        fs::write(&occupied, b"different bytes").unwrap();
        let digest = crate::hash(&stays).unwrap();
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/DJI_0653.MP4",
            "Trips/2018/2018-03-Iceland/DJI_0653 2.MP4",
        )]);
        let report = supersede(
            &fixture.library(),
            &list,
            &fixture.0.join("q"),
            &fixture.0.join("j.tsv"),
            true,
        )
        .unwrap();
        assert_eq!(report.names_normalised, 0);
        assert_eq!(report.name_collisions, 1);
        assert_eq!(
            fs::read(&occupied).unwrap(),
            b"different bytes",
            "never overwritten"
        );
        assert!(
            stays.exists(),
            "the suffixed name is kept rather than forced"
        );
        assert_eq!(report.quarantined, 1, "the twin still goes");
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_name_that_merely_ends_in_a_number_is_never_rewritten() {
        // The same name on both sides means there is no suffix to adopt, even though stripping this
        // name repeatedly would produce shorter ones.
        let fixture = Fixture::new();
        let name = "video 21.01.18, 14 34 37.mov";
        let gone = fixture.library().join("by-date/2018-03").join(name);
        let stays = fixture
            .library()
            .join("Trips/2018/2018-03-Iceland")
            .join(name);
        fs::write(&gone, b"bytes").unwrap();
        fs::write(&stays, b"bytes").unwrap();
        let digest = crate::hash(&stays).unwrap();
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/video 21.01.18, 14 34 37.mov",
            "Trips/2018/2018-03-Iceland/video 21.01.18, 14 34 37.mov",
        )]);
        let report = supersede(
            &fixture.library(),
            &list,
            &fixture.0.join("q"),
            &fixture.0.join("j.tsv"),
            true,
        )
        .unwrap();
        assert_eq!(report.names_normalised, 0);
        assert_eq!(report.name_collisions, 0);
        assert!(stays.exists(), "the name is untouched");
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn the_suffix_stripper_handles_both_macos_forms_and_their_combination() {
        assert_eq!(
            strip_one_suffix("DJI_1 2.MP4").as_deref(),
            Some("DJI_1.MP4")
        );
        assert_eq!(
            strip_one_suffix("photo (1).JPG").as_deref(),
            Some("photo.JPG")
        );
        assert_eq!(
            strip_one_suffix("DJI_0609 (1) 2.MP4").as_deref(),
            Some("DJI_0609 (1).MP4")
        );
        assert_eq!(strip_one_suffix("no-suffix.MP4"), None);
        assert_eq!(
            strip_one_suffix("2.MP4"),
            None,
            "a numeric stem is not an index"
        );
    }

    #[test]
    fn a_verified_row_is_quarantined_not_deleted_and_the_journal_reverses_it() {
        let fixture = Fixture::new();
        let digest = fixture.pair("IMG_7811.MOV", b"same bytes");
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/IMG_7811.MOV",
            "Trips/2018/2018-03-Iceland/IMG_7811.MOV",
        )]);
        let quarantine = fixture.0.join("quarantine");
        let journal = fixture.0.join("journal.tsv");
        let report = supersede(&fixture.library(), &list, &quarantine, &journal, true).unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.quarantined, 1);
        assert_eq!(report.bytes_quarantined, 10);
        assert!(!fixture
            .library()
            .join("by-date/2018-03/IMG_7811.MOV")
            .exists());
        assert!(fixture
            .library()
            .join("Trips/2018/2018-03-Iceland/IMG_7811.MOV")
            .exists());
        // The bytes still exist, in quarantine, so the space is not yet reclaimed.
        let quarantined: Vec<_> = walk(&quarantine);
        assert_eq!(quarantined.len(), 1);
        assert_eq!(fs::read(&quarantined[0]).unwrap(), b"same bytes");
        let text = fs::read_to_string(&journal).unwrap();
        assert!(text.contains("by-date/2018-03/IMG_7811.MOV"));
        let _ = fs::remove_dir_all(&fixture.0);
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    out.extend(walk(&path));
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[test]
    fn a_missing_surviving_copy_refuses_the_whole_run() {
        let fixture = Fixture::new();
        let digest = fixture.pair("a.MOV", b"one");
        let other = fixture.pair("b.MOV", b"two");
        fs::remove_file(fixture.library().join("Trips/2018/2018-03-Iceland/b.MOV")).unwrap();
        let list = fixture.list(&[
            (
                &digest,
                "by-date/2018-03/a.MOV",
                "Trips/2018/2018-03-Iceland/a.MOV",
            ),
            (
                &other,
                "by-date/2018-03/b.MOV",
                "Trips/2018/2018-03-Iceland/b.MOV",
            ),
        ]);
        let report = supersede(
            &fixture.library(),
            &list,
            &fixture.0.join("quarantine"),
            &fixture.0.join("journal.tsv"),
            true,
        )
        .unwrap();
        assert!(!report.complete);
        assert_eq!(
            report.quarantined, 0,
            "the row that passed was not moved either"
        );
        assert!(fixture.library().join("by-date/2018-03/a.MOV").exists());
        assert_eq!(report.refused, 1);
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_changed_superseded_copy_refuses_rather_than_quarantining_the_wrong_file() {
        let fixture = Fixture::new();
        let digest = fixture.pair("a.MOV", b"original");
        // The superseded copy is rewritten after the list was produced.
        fs::write(fixture.library().join("by-date/2018-03/a.MOV"), b"tampered").unwrap();
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/a.MOV",
            "Trips/2018/2018-03-Iceland/a.MOV",
        )]);
        let report = supersede(
            &fixture.library(),
            &list,
            &fixture.0.join("quarantine"),
            &fixture.0.join("journal.tsv"),
            true,
        )
        .unwrap();
        assert!(!report.complete);
        assert_eq!(report.quarantined, 0);
        assert!(report
            .issues
            .iter()
            .any(|i| i.contains("no longer matches")));
        assert_eq!(
            fs::read(fixture.library().join("by-date/2018-03/a.MOV")).unwrap(),
            b"tampered"
        );
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_dry_run_verifies_every_row_and_moves_nothing() {
        let fixture = Fixture::new();
        let digest = fixture.pair("a.MOV", b"bytes");
        let list = fixture.list(&[(
            &digest,
            "by-date/2018-03/a.MOV",
            "Trips/2018/2018-03-Iceland/a.MOV",
        )]);
        let quarantine = fixture.0.join("quarantine");
        let report = supersede(
            &fixture.library(),
            &list,
            &quarantine,
            &fixture.0.join("j.tsv"),
            false,
        )
        .unwrap();
        assert_eq!(report.verified, 1);
        assert_eq!(report.quarantined, 0);
        assert!(fixture.library().join("by-date/2018-03/a.MOV").exists());
        assert!(!quarantine.exists());
        let _ = fs::remove_dir_all(&fixture.0);
    }

    #[test]
    fn a_traversal_or_absolute_path_in_the_list_is_refused() {
        let fixture = Fixture::new();
        let digest = fixture.pair("a.MOV", b"bytes");
        for bad in ["../../etc/passwd", "/etc/passwd", "by-date/../..//x"] {
            let list = fixture.list(&[(&digest, bad, "Trips/2018/2018-03-Iceland/a.MOV")]);
            let report = supersede(
                &fixture.library(),
                &list,
                &fixture.0.join("q"),
                &fixture.0.join("j.tsv"),
                true,
            )
            .unwrap();
            assert!(!report.complete, "{bad}");
            assert_eq!(report.quarantined, 0, "{bad}");
        }
        let _ = fs::remove_dir_all(&fixture.0);
    }
}

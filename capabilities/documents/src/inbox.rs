//! Non-recursive local intake. Planning reads directory metadata only.

use std::{
    fs::{self, File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::{library::MAX_BYTES, LibraryError};

pub struct Plan {
    pub files: Vec<InboxFile>,
    pub excluded: usize,
    pub unready: usize,
}

pub struct InboxFile {
    pub filename: String,
    path: PathBuf,
    metadata: Metadata,
}

impl InboxFile {
    /// Read a bounded original only if it still matches the planned regular file.
    ///
    /// # Errors
    /// The file changed, disappeared, exceeded the size limit, or could not be read.
    pub fn read(&self) -> Result<Vec<u8>, LibraryError> {
        let current = fs::symlink_metadata(&self.path)?;
        if !unchanged(&self.metadata, &current) {
            return Err(LibraryError::Invalid(
                "inbox file changed; rescan before intake".into(),
            ));
        }
        let file = File::open(&self.path)?;
        let opened = file.metadata()?;
        if !unchanged(&self.metadata, &opened) {
            return Err(LibraryError::Invalid(
                "inbox file was replaced; rescan before intake".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        (&file).take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if !unchanged(&opened, &file.metadata()?) || bytes.len() as u64 != opened.len() {
            return Err(LibraryError::Invalid(
                "inbox file changed during reading; retry later".into(),
            ));
        }
        Ok(bytes)
    }
}

fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    after.is_file()
        && before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}

/// Plan the latest `limit` supported documents by modification time, then exclude
/// sensitive names and files still being written. Exclusions are never backfilled.
///
/// # Errors
/// The directory is a symlink, the limit is zero, or directory metadata is unreadable.
pub fn scan(directory: &Path, limit: usize) -> Result<Plan, LibraryError> {
    if limit == 0 || !fs::symlink_metadata(directory)?.is_dir() {
        return Err(LibraryError::Invalid(
            "inbox needs a real directory and a positive limit".into(),
        ));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.is_file() {
            continue;
        }
        let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if supported(&filename) {
            files.push(InboxFile {
                filename,
                path: entry.path(),
                metadata,
            });
        }
    }
    files.sort_by(|a, b| {
        (b.metadata.mtime(), b.metadata.mtime_nsec())
            .cmp(&(a.metadata.mtime(), a.metadata.mtime_nsec()))
            .then_with(|| a.filename.cmp(&b.filename))
    });
    files.truncate(limit);
    let mut plan = Plan {
        files: Vec::new(),
        excluded: 0,
        unready: 0,
    };
    let now = SystemTime::now();
    for file in files {
        if sensitive_name(&file.filename) {
            plan.excluded += 1;
        } else if file.metadata.len() == 0
            || file.metadata.len() > MAX_BYTES as u64
            || file.metadata.modified()?.duration_since(now).is_ok()
            || now
                .duration_since(file.metadata.modified()?)
                .unwrap_or_default()
                < Duration::from_secs(30)
        {
            plan.unready += 1;
        } else {
            plan.files.push(file);
        }
    }
    Ok(plan)
}

fn supported(filename: &str) -> bool {
    filename.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "pdf"
                | "txt"
                | "md"
                | "doc"
                | "docx"
                | "xls"
                | "xlsx"
                | "ppt"
                | "pptx"
                | "odt"
                | "ods"
                | "odp"
                | "rtf"
                | "csv"
                | "epub"
                | "pages"
                | "numbers"
                | "keynote"
        )
    })
}

/// Whether a filename must be excluded before local extraction or upload intake.
pub fn sensitive_name(filename: &str) -> bool {
    let name = filename.to_lowercase();
    name.starts_with('.')
        || name.chars().any(char::is_control)
        || [
            "secret",
            "password",
            "passwd",
            "credential",
            "token",
            "private",
            "confidential",
            "vertraulich",
            "geheim",
            "passport",
            "ausweis",
            "recovery",
            "seed phrase",
            "seed_phrase",
            "medical",
            "patient",
            "payroll",
            "salary",
            "steuer",
            "tax",
            "bank",
            "konto",
        ]
        .iter()
        .any(|word| name.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sjel-inbox-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn old_file(root: &Path, name: &str, age: u64) {
        let path = root.join(name);
        fs::write(&path, b"synthetic document").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(age)))
            .unwrap();
    }

    #[test]
    fn latest_limit_precedes_sensitive_exclusions_without_backfill() {
        let root = scratch();
        old_file(&root, "older.txt", 300);
        old_file(&root, "invoice.pdf", 200);
        old_file(&root, "SECRETS.txt", 100);
        let plan = scan(&root, 2).unwrap();
        assert_eq!(plan.excluded, 1);
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].filename, "invoice.pdf");
        assert_eq!(plan.files[0].read().unwrap(), b"synthetic document");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_links_subdirectories_unsupported_and_unsettled_files() {
        let root = scratch();
        old_file(&root, "kept.MD", 100);
        old_file(&root, "installer.dmg", 100);
        fs::write(root.join("new.txt"), b"upload in progress").unwrap();
        fs::create_dir(root.join("nested.pdf")).unwrap();
        std::os::unix::fs::symlink(root.join("kept.MD"), root.join("link.txt")).unwrap();
        let plan = scan(&root, 10).unwrap();
        assert_eq!(plan.unready, 1);
        assert_eq!(plan.files.len(), 1);
        assert!(scan(&root, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_a_planned_file_replaced_by_a_symlink_or_changed() {
        let root = scratch();
        old_file(&root, "input.txt", 100);
        let plan = scan(&root, 10).unwrap();
        fs::write(root.join("input.txt"), b"different").unwrap();
        assert!(plan.files[0].read().is_err());
        fs::remove_file(root.join("input.txt")).unwrap();
        std::os::unix::fs::symlink("missing", root.join("input.txt")).unwrap();
        assert!(plan.files[0].read().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filenames_are_screened_without_reading_contents() {
        for name in [
            ".env.txt",
            "credentials.pdf",
            "private-notes.md",
            "Vertraulich.docx",
            "bank-statement.pdf",
            "seed_phrase.txt",
        ] {
            assert!(sensitive_name(name), "{name}");
        }
        assert!(!sensitive_name("invoice.pdf"));
        assert!(!sensitive_name("manual.md"));
    }
}

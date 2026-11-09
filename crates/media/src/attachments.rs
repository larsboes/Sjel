//! Local immutable originals, separate from mounted-volume locations.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::store::{Ledger, Result};

/// Maximum raw upload size in bytes.
pub const MAX_BYTES: usize = 128 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Reject known Apple placeholder payloads without claiming broad media decoding.
///
/// # Errors
/// The bytes are an unsupported binary-property-list placeholder rather than an original.
pub fn validate_content(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(b"bplist00") {
        return Err(
            "unsupported Apple binary-plist placeholder; original media bytes are required".into(),
        );
    }
    Ok(())
}

/// Attachment identity and all observed source filenames.
#[derive(Debug, Serialize)]
pub struct Attachment {
    pub sha256: String,
    pub byte_len: u64,
    pub filenames: Vec<String>,
}

/// Media-owned private storage. Filesystem writers must not mutate this root externally.
pub struct Library {
    ledger: Arc<Ledger>,
    root: PathBuf,
    intake: Mutex<()>,
}

/// Reject path components, control characters, and ambiguous filenames.
///
/// # Errors
/// The name is not a single visible UTF-8 filename of at most 255 bytes.
pub fn validate_filename(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
        || name.trim() != name
    {
        return Err("filename must be a single visible filename, not a path".into());
    }
    Ok(())
}

/// Validate the canonical lowercase SHA-256 identifier.
///
/// # Errors
/// The identifier is not 64 lowercase hexadecimal characters.
pub fn validate_digest(digest: &str) -> Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("sha256 must be 64 lowercase hex characters".into());
    }
    Ok(())
}

fn no_symlink_components(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("attachment path contains a symlink".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

impl Library {
    /// Open a private attachment root using the media ledger's migrated tables.
    ///
    /// # Errors
    /// The root is relative, contains symlinks, or cannot be created privately.
    pub fn open(ledger: Arc<Ledger>, root: PathBuf) -> Result<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("attachment root must be an absolute path without traversal".into());
        }
        no_symlink_components(&root)?;
        fs::create_dir_all(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            ledger,
            root,
            intake: Mutex::new(()),
        })
    }

    fn verify(&self, digest: &str, size: u64) -> Result<()> {
        let path = self.root.join(digest);
        no_symlink_components(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.len() != size || crate::hash(&path)? != digest {
            return Err("attachment original failed size or SHA-256 verification".into());
        }
        Ok(())
    }

    /// Look up metadata only after verifying the stored original.
    ///
    /// # Errors
    /// The digest, database, or original fails validation; corrupt originals are never reported present.
    pub fn holding(&self, digest: &str) -> Result<Option<Attachment>> {
        validate_digest(digest)?;
        let conn = self.ledger.pool.get()?;
        let size: Option<i64> = conn
            .query_row(
                "SELECT byte_len FROM media_attachments WHERE sha256=?1",
                [digest],
                |r| r.get(0),
            )
            .optional()?;
        let Some(byte_len) = size else {
            return Ok(None);
        };
        let byte_len = u64::try_from(byte_len)?;
        self.verify(digest, byte_len)?;
        let mut statement = conn.prepare(
            "SELECT filename FROM media_attachment_sources WHERE sha256=?1 ORDER BY filename",
        )?;
        let filenames = statement
            .query_map([digest], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(Attachment {
            sha256: digest.into(),
            byte_len,
            filenames,
        }))
    }

    /// Read one stable regular source without following symlinks, then store its original.
    ///
    /// # Errors
    /// The path is relative, symlinked, oversized, unstable, unsupported, or cannot be stored.
    pub fn upload_file(&self, path: &Path) -> Result<(Attachment, bool)> {
        use std::os::unix::fs::MetadataExt;
        if !path.is_absolute() {
            return Err("source must be an absolute file path".into());
        }
        no_symlink_components(path)?;
        let before = fs::symlink_metadata(path)?;
        if !before.is_file() || before.len() > MAX_BYTES as u64 {
            return Err("source must be a regular file of at most 128 MiB".into());
        }
        let file = fs::File::open(path)?;
        let opened = file.metadata()?;
        let identity = |m: &fs::Metadata| (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec());
        if identity(&before) != identity(&opened) {
            return Err("source changed while opening".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        let after = fs::symlink_metadata(path)?;
        if !after.is_file()
            || identity(&after) != identity(&opened)
            || bytes.len() as u64 != opened.len()
        {
            return Err("source size or mtime changed during intake".into());
        }
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("source filename must be UTF-8")?;
        let digest = crate::hex_digest(&Sha256::digest(&bytes));
        self.upload(filename, &digest, &bytes)
    }

    /// Verify and atomically publish bytes without replacing any existing pathname, then record provenance.
    ///
    /// # Errors
    /// Invalid input, corrupt existing bytes, symlinks, or an I/O or metadata failure. A retry verifies any orphan original.
    pub fn upload(&self, filename: &str, digest: &str, bytes: &[u8]) -> Result<(Attachment, bool)> {
        validate_filename(filename)?;
        validate_digest(digest)?;
        validate_content(bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err("attachment exceeds 128 MiB".into());
        }
        if crate::hex_digest(&Sha256::digest(bytes)) != digest {
            return Err("upload SHA-256 mismatch".into());
        }
        let _guard = self
            .intake
            .lock()
            .map_err(|_| "attachment intake lock poisoned")?;
        no_symlink_components(&self.root)?;
        let destination = self.root.join(digest);
        let created = match fs::symlink_metadata(&destination) {
            Ok(_) => {
                self.verify(digest, bytes.len() as u64)?;
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let temporary = self.root.join(format!(
                    ".upload-{}-{}",
                    std::process::id(),
                    NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
                ));
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                let publish = (|| -> Result<bool> {
                    file.write_all(bytes)?;
                    file.sync_all()?;
                    if crate::hash(&temporary)? != digest {
                        return Err("temporary original failed SHA-256 verification".into());
                    }
                    file.set_permissions(fs::Permissions::from_mode(0o400))?;
                    match fs::hard_link(&temporary, &destination) {
                        Ok(()) => Ok(true),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            Ok(false)
                        }
                        Err(error) => Err(error.into()),
                    }
                })();
                fs::remove_file(&temporary)?;
                let created = publish?;
                self.verify(digest, bytes.len() as u64)?;
                created
            }
            Err(error) => return Err(error.into()),
        };
        fs::File::open(&self.root)?.sync_all()?;
        let mut conn = self.ledger.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute("INSERT INTO media_attachments(sha256,byte_len) VALUES(?1,?2) ON CONFLICT(sha256) DO NOTHING", params![digest, bytes.len() as i64])?;
        let recorded: i64 = tx.query_row(
            "SELECT byte_len FROM media_attachments WHERE sha256=?1",
            [digest],
            |r| r.get(0),
        )?;
        if recorded != bytes.len() as i64 {
            return Err("attachment metadata size mismatch".into());
        }
        tx.execute("INSERT INTO media_attachment_sources(sha256,filename) VALUES(?1,?2) ON CONFLICT(sha256,filename) DO NOTHING", params![digest, filename])?;
        tx.commit()?;
        let attachment = self
            .holding(digest)?
            .ok_or("attachment metadata missing after upload")?;
        Ok((attachment, created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (PathBuf, Arc<Ledger>, Library) {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("media-attachments-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let ledger = Arc::new(Ledger::open(&root.join("ledger.db")).unwrap());
        let library = Library::open(ledger.clone(), root.join("originals")).unwrap();
        (root, ledger, library)
    }

    #[test]
    fn dedup_preserves_names_and_retry_recovers_an_orphan() {
        let (root, ledger, library) = scratch("dedup");
        let digest = crate::hex_digest(&Sha256::digest(b"image"));
        assert!(library.upload("photo.jpg", &digest, b"image").unwrap().1);
        let (held, created) = library.upload("copy.jpg", &digest, b"image").unwrap();
        assert!(!created);
        assert_eq!(held.filenames, ["copy.jpg", "photo.jpg"]);
        assert!(!library.upload("copy.jpg", &digest, b"image").unwrap().1);
        ledger
            .pool
            .get()
            .unwrap()
            .execute("DELETE FROM media_attachment_sources", [])
            .unwrap();
        ledger
            .pool
            .get()
            .unwrap()
            .execute("DELETE FROM media_attachments", [])
            .unwrap();
        assert!(library.holding(&digest).unwrap().is_none());
        assert!(
            !library
                .upload("recovered.jpg", &digest, b"image")
                .unwrap()
                .1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_file_intake_rejects_binary_plist_placeholders_and_symlinks() {
        let (root, _, library) = scratch("single-file");
        let mut placeholder = vec![0; 135];
        placeholder[..8].copy_from_slice(b"bplist00");
        let path = root.join("placeholder.heic");
        fs::write(&path, &placeholder).unwrap();
        let digest = crate::hex_digest(&Sha256::digest(&placeholder));
        let error = library.upload_file(&path).unwrap_err().to_string();
        assert!(error.contains("placeholder"));
        assert!(library.holding(&digest).unwrap().is_none());
        assert!(!library.root.join(&digest).exists());
        assert_eq!(fs::read(&path).unwrap(), placeholder);
        let photo = root.join("screenshot.png");
        fs::write(&photo, b"\x89PNG\r\n\x1a\nimage").unwrap();
        assert!(library.upload_file(&photo).unwrap().1);
        let alias = root.join("alias.png");
        std::os::unix::fs::symlink(&photo, &alias).unwrap();
        assert!(library.upload_file(&alias).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_original_is_not_present_and_is_never_overwritten() {
        let (root, _, library) = scratch("corrupt");
        let digest = crate::hex_digest(&Sha256::digest(b"image"));
        library.upload("photo.jpg", &digest, b"image").unwrap();
        let path = library.root.join(&digest);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"wrong").unwrap();
        assert!(library.holding(&digest).is_err());
        assert!(library.upload("photo.jpg", &digest, b"image").is_err());
        assert_eq!(fs::read(path).unwrap(), b"wrong");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_names_digests_and_symlinks_are_refused() {
        let (root, ledger, library) = scratch("refuse");
        let digest = crate::hex_digest(&Sha256::digest(b"image"));
        for name in [
            "", ".hidden", "..", "../x.jpg", "/x.jpg", "a\\b.jpg", "a\n.jpg", "x:stream",
        ] {
            assert!(library.upload(name, &digest, b"image").is_err(), "{name:?}");
        }
        assert!(library.upload("x.jpg", &digest, b"changed").is_err());
        assert!(library.holding("../x").is_err());
        let target = root.join("untouched");
        fs::write(&target, b"image").unwrap();
        std::os::unix::fs::symlink(&target, library.root.join(&digest)).unwrap();
        assert!(library.upload("x.jpg", &digest, b"image").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"image");
        std::os::unix::fs::symlink(&library.root, root.join("linked")).unwrap();
        assert!(Library::open(ledger, root.join("linked/subdir")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

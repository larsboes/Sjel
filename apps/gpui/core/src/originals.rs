//! Document originals on the phone (`docs/plans/gpui.md`, "What may live on the phone", rule 2).
//!
//! An opened original lives in a cache folder the system may clear. "Keep offline" copies it,
//! with its detail, into a kept folder, so it opens while the Mac is unreachable. Every original
//! is checked against the library's SHA-256 before it is written.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::device::Source;
use crate::shell::{DocumentDetail, DocumentSummary};

/// The two folders originals live in.
#[derive(Clone, Debug)]
pub struct Originals {
    cache: PathBuf,
    kept: PathBuf,
}

impl Originals {
    pub fn new(cache: PathBuf, kept: PathBuf) -> Self {
        Self { cache, kept }
    }

    fn kept_original(&self, id: &str) -> PathBuf {
        self.kept.join(format!("{id}.original"))
    }

    fn kept_detail(&self, id: &str) -> PathBuf {
        self.kept.join(format!("{id}.json"))
    }

    /// Whether `id` is kept on this phone.
    pub fn is_kept(&self, id: &str) -> bool {
        self.kept_original(id).is_file() && self.kept_detail(id).is_file()
    }

    /// The details of every kept document, newest import first.
    pub fn kept(&self) -> Vec<DocumentDetail> {
        let Ok(entries) = fs::read_dir(&self.kept) else {
            return Vec::new();
        };
        let mut kept: Vec<DocumentDetail> = entries
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|detail: &DocumentDetail| self.is_kept(&detail.document.id))
            .collect();
        kept.sort_by(|a, b| b.document.imported_at.cmp(&a.document.imported_at));
        kept
    }

    /// The kept detail of `id`, when it is kept.
    pub fn kept_detail_of(&self, id: &str) -> Option<DocumentDetail> {
        let bytes = fs::read(self.kept_detail(id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The path of `document`'s original: the kept one, or one downloaded through `source`.
    ///
    /// # Errors
    /// No source for a document that is not kept, the download failure, a library that sent no
    /// checksum, an original whose SHA-256 differs from the library's, or a write failure.
    pub async fn open(
        &self,
        source: Option<&Source>,
        document: &DocumentSummary,
    ) -> Result<PathBuf, String> {
        if self.is_kept(&document.id) {
            return Ok(self.kept_original(&document.id));
        }
        let source = source.ok_or_else(|| "The Mac is not reachable.".to_string())?;
        let bytes = source.document_original(&document.id).await?;
        let path = self.cache.join(format!("{}.original", document.id));
        write_checked(&path, &bytes, &document.sha256)?;
        Ok(path)
    }

    /// Keeps `detail`'s document on this phone: its original from the cache, and the detail.
    ///
    /// # Errors
    /// An original that was never opened, or a write failure.
    pub fn keep(&self, detail: &DocumentDetail) -> Result<PathBuf, String> {
        let id = &detail.document.id;
        let cached = self.cache.join(format!("{id}.original"));
        let bytes =
            fs::read(&cached).map_err(|_| "Open the original before keeping it.".to_string())?;
        write_checked(&self.kept_original(id), &bytes, &detail.document.sha256)?;
        let json = serde_json::to_vec(detail).map_err(|e| e.to_string())?;
        fs::write(self.kept_detail(id), json).map_err(|e| format!("Could not keep it: {e}"))?;
        Ok(self.kept_original(id))
    }

    /// Removes `id` from this phone. A document that was not kept is left as it is.
    pub fn forget(&self, id: &str) {
        let _ = fs::remove_file(self.kept_original(id));
        let _ = fs::remove_file(self.kept_detail(id));
    }
}

/// Writes `bytes` to `path` only when their SHA-256 is `expected` (lowercase hex).
fn write_checked(path: &Path, bytes: &[u8], expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Err("The library sent no checksum for this document.".into());
    }
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("The original does not match the library's checksum.".into());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Could not store it: {e}"))?;
    }
    fs::write(path, bytes).map_err(|e| format!("Could not store it: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(name: &str) -> Originals {
        let root =
            std::env::temp_dir().join(format!("sjel-originals-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Originals::new(root.join("cache"), root.join("kept"))
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn only_bytes_with_the_library_checksum_are_written() {
        let store = folders("check");
        let path = store.cache.join("a.original");
        assert!(write_checked(&path, b"%PDF-1", "").is_err());
        assert!(write_checked(&path, b"%PDF-1", &sha(b"other")).is_err());
        assert!(!path.exists());
        assert!(write_checked(&path, b"%PDF-1", &sha(b"%PDF-1").to_uppercase()).is_ok());
        assert_eq!(fs::read(&path).unwrap(), b"%PDF-1");
    }

    #[test]
    fn a_kept_document_lists_and_opens_from_this_phone_until_it_is_forgotten() {
        let store = folders("keep");
        let id = "b".repeat(64);
        let detail = DocumentDetail {
            document: DocumentSummary {
                id: id.clone(),
                sha256: sha(b"%PDF-2"),
                ..Default::default()
            },
            text: Some("text".into()),
            proposals: Vec::new(),
            pages: Vec::new(),
            children: Vec::new(),
            split_suggestions: None,
        };
        assert!(store.keep(&detail).is_err(), "nothing is cached yet");
        write_checked(
            &store.cache.join(format!("{id}.original")),
            b"%PDF-2",
            &detail.document.sha256,
        )
        .unwrap();
        store.keep(&detail).unwrap();
        assert!(store.is_kept(&id));
        assert_eq!(store.kept().len(), 1);
        assert_eq!(
            store.kept_detail_of(&id).unwrap().text.as_deref(),
            Some("text")
        );
        store.forget(&id);
        assert!(!store.is_kept(&id));
        assert!(store.kept().is_empty());
    }
}

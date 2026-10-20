use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PREFIX: &str = "documents";
pub(crate) const MAX_BYTES: usize = 128 * 1024 * 1024;

type Fallible<T> = Result<T, LibraryError>;

#[derive(Debug)]
pub enum LibraryError {
    Invalid(String),
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Store(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Io(error) => write!(f, "document file operation: {error}"),
            Self::Sql(error) => write!(f, "document metadata: {error}"),
            Self::Store(error) => write!(f, "document store: {error}"),
        }
    }
}

impl std::error::Error for LibraryError {}
impl From<std::io::Error> for LibraryError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for LibraryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    #[serde(default)]
    pub sha256: String,
    pub filename: String,
    pub media_type: String,
    pub byte_len: i64,
    pub imported_at: String,
    pub status: String,
    pub category: Option<String>,
    pub project: Option<String>,
    pub extracted_title: Option<String>,
    pub extracted_text: Option<String>,
    pub extractor: Option<String>,
    pub suggested_category: Option<String>,
    pub suggestion_reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    New(Document),
    Unchanged(Document),
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub category: String,
    pub project: Option<String>,
}

pub struct Library {
    pool: sjel_store::Pool,
    root: PathBuf,
}

impl Library {
    /// Open the prefixed shared-store tables and local canonical file root.
    ///
    /// # Errors
    /// The database cannot be migrated, or the root cannot be created.
    pub fn open(database: &Path, root: PathBuf) -> Fallible<Self> {
        let blobs = root.join("blobs");
        let records = root.join("records");
        fs::create_dir_all(&blobs)?;
        fs::create_dir_all(&records)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&blobs, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&records, fs::Permissions::from_mode(0o700))?;
        let pool = sjel_store::open_pool(database, PREFIX, migrate)
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        Ok(Self { pool, root })
    }

    /// Intake bytes idempotently by digest. Existing originals are never overwritten.
    ///
    /// # Errors
    /// The filename is unsafe, the input is too large, extraction or persistence fails.
    pub fn intake(&self, filename: &str, bytes: &[u8]) -> Fallible<Stored> {
        let filename = safe_filename(filename)?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(LibraryError::Invalid(format!(
                "document must be 1..={MAX_BYTES} bytes"
            )));
        }
        let sha256 = hex(&Sha256::digest(bytes));
        let media_type = media_type(&filename).to_string();
        if let Some(existing) = self.by_hash_and_name(&sha256, &filename)? {
            self.read_blob(&sha256)?;
            self.write_record(&existing)?;
            return Ok(Stored::Unchanged(existing));
        }
        let id = record_id(&sha256, &filename);
        let extracted = extract(&media_type, bytes);
        let (suggested_category, suggestion_reason) = suggest_category(
            &filename,
            extracted.as_ref().map(|result| result.text.as_str()),
        );
        self.write_blob(&sha256, bytes)?;
        let document = Document {
            id: id.clone(),
            sha256: sha256.clone(),
            filename,
            media_type,
            byte_len: bytes.len() as i64,
            imported_at: chrono::Utc::now().to_rfc3339(),
            status: "inbox".into(),
            category: None,
            project: None,
            extracted_title: extracted.as_ref().and_then(|e| e.title.clone()),
            extracted_text: extracted.as_ref().map(|e| e.text.clone()),
            extractor: extracted.as_ref().map(|e| e.producer.to_string()),
            suggested_category,
            suggestion_reason,
        };
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO documents_items
             (id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'inbox', NULL, NULL, ?7, ?8, ?9)",
            params![document.id, document.sha256, document.filename, document.media_type, document.byte_len,
                document.imported_at, document.extracted_title, document.extracted_text, document.extractor],
        )?;
        let saved = self.get(&id)?.ok_or_else(|| {
            LibraryError::Invalid("intake did not create a document record".into())
        })?;
        self.write_record(&saved)?;
        if inserted == 1 {
            Ok(Stored::New(saved))
        } else {
            Ok(Stored::Unchanged(saved))
        }
    }

    /// List all documents, optionally filtered by the explicit review state.
    ///
    /// # Errors
    /// The metadata query fails.
    pub fn list(&self, status: Option<&str>) -> Fallible<Vec<Document>> {
        if let Some(status) = status {
            check_status(status)?;
        }
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let sql = if status.is_some() {
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor FROM documents_items WHERE status = ?1 ORDER BY imported_at DESC, filename"
        } else {
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor FROM documents_items ORDER BY imported_at DESC, filename"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(status) = status {
            stmt.query_map([status], read_document)?
        } else {
            stmt.query_map([], read_document)?
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Read one metadata record.
    ///
    /// # Errors
    /// The id is malformed or the metadata query fails.
    pub fn get(&self, id: &str) -> Fallible<Option<Document>> {
        check_id(id)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        Ok(conn.query_row(
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor FROM documents_items WHERE id = ?1",
            [id], read_document,
        ).optional()?)
    }

    fn by_hash_and_name(&self, sha256: &str, filename: &str) -> Fallible<Option<Document>> {
        check_id(sha256)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        Ok(conn.query_row(
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor FROM documents_items WHERE sha256 = ?1 AND filename = ?2",
            params![sha256, filename], read_document,
        ).optional()?)
    }

    /// Read and verify the original bytes for one document id.
    ///
    /// # Errors
    /// The id is malformed, the file is missing, or its digest differs from the record.
    pub fn bytes(&self, id: &str) -> Fallible<Vec<u8>> {
        check_id(id)?;
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        self.read_blob(&document.sha256)
    }

    fn read_blob(&self, sha256: &str) -> Fallible<Vec<u8>> {
        check_id(sha256)?;
        let bytes = fs::read(self.blob_path(sha256))?;
        if hex(&Sha256::digest(&bytes)) != sha256 {
            return Err(LibraryError::Invalid(
                "stored document failed SHA-256 verification".into(),
            ));
        }
        Ok(bytes)
    }

    /// Set a reviewed category and optional project label; this does not modify the original.
    ///
    /// # Errors
    /// The id/category/project is invalid, or the document does not exist.
    pub fn review(&self, id: &str, review: &Review) -> Fallible<Document> {
        check_id(id)?;
        let category = safe_category(&review.category)?;
        let project = review.project.as_deref().map(safe_project).transpose()?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let changed = conn.execute(
            "UPDATE documents_items SET status = 'filed', category = ?2, project = ?3 WHERE id = ?1",
            params![id, category, project],
        )?;
        if changed == 0 {
            return Err(LibraryError::Invalid("no document with that digest".into()));
        }
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("reviewed record disappeared".into()))?;
        self.write_record(&document)?;
        Ok(document)
    }

    /// Mark a document archived without deleting its bytes or metadata.
    ///
    /// # Errors
    /// The id is malformed or no matching document exists.
    pub fn archive(&self, id: &str) -> Fallible<Document> {
        check_id(id)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        if conn.execute(
            "UPDATE documents_items SET status = 'archived' WHERE id = ?1",
            [id],
        )? == 0
        {
            return Err(LibraryError::Invalid("no document with that digest".into()));
        }
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("archived record disappeared".into()))?;
        self.write_record(&document)?;
        Ok(document)
    }

    /// Restore missing metadata rows from the self-contained sidecar archive.
    /// Existing rows must match; this never overwrites a different record.
    ///
    /// # Errors
    /// A sidecar is malformed, its original is missing/corrupt, or an existing row conflicts.
    pub fn restore_index(&self) -> Fallible<(usize, usize)> {
        let mut restored = 0;
        let mut unchanged = 0;
        let records = self.root.join("records");
        for entry in fs::read_dir(&records)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let document = parse_sidecar(&fs::read(&path)?)
                .map_err(|e| LibraryError::Invalid(format!("invalid document sidecar: {e}")))?;
            check_id(&document.id)?;
            check_id(&document.sha256)?;
            safe_filename(&document.filename)?;
            check_status(&document.status)?;
            if let Some(category) = document.category.as_deref() {
                safe_category(category)?;
            }
            if let Some(project) = document.project.as_deref() {
                safe_project(project)?;
            }
            if path.file_stem().and_then(|name| name.to_str()) != Some(document.id.as_str()) {
                return Err(LibraryError::Invalid(
                    "sidecar filename does not match its digest".into(),
                ));
            }
            if self.read_blob(&document.sha256)?.len() as i64 != document.byte_len {
                return Err(LibraryError::Invalid(format!(
                    "sidecar length does not match original {}",
                    document.id
                )));
            }
            if let Some(existing) = self.get(&document.id)? {
                if !same_record(&existing, &document) {
                    return Err(LibraryError::Invalid(format!(
                        "sidecar conflicts with metadata for {}",
                        document.id
                    )));
                }
                unchanged += 1;
            } else {
                self.insert_record(&document)?;
                restored += 1;
            }
        }
        Ok((restored, unchanged))
    }

    /// Check every recorded original and its recovery sidecar.
    ///
    /// # Errors
    /// The metadata query or filesystem walk fails.
    pub fn verify(&self) -> Fallible<Vec<String>> {
        let docs = self.list(None)?;
        let mut issues = Vec::new();
        let known: std::collections::HashSet<String> =
            docs.iter().map(|doc| doc.id.clone()).collect();
        for doc in docs {
            match fs::read(self.blob_path(&doc.sha256)) {
                Ok(bytes) if hex(&Sha256::digest(&bytes)) == doc.sha256 => {}
                _ => issues.push(format!("{}:original", doc.id)),
            }
            match fs::read(self.record_path(&doc.id)) {
                Ok(bytes) if parse_sidecar(&bytes).is_ok_and(|copy| same_record(&copy, &doc)) => {}
                _ => issues.push(format!("{}:metadata", doc.id)),
            }
        }
        for entry in fs::read_dir(self.root.join("records"))? {
            let entry = entry?;
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                let id = entry
                    .path()
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !known.contains(&id) {
                    issues.push(format!("{id}:orphan-sidecar"));
                }
            }
        }
        Ok(issues)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join("records").join(format!("{id}.json"))
    }

    fn write_record(&self, document: &Document) -> Fallible<()> {
        let path = self.record_path(&document.id);
        let parent = path
            .parent()
            .ok_or_else(|| LibraryError::Invalid("metadata path has no parent".into()))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| LibraryError::Invalid(e.to_string()))?
            .as_nanos();
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            document.id,
            std::process::id(),
            stamp
        ));
        let bytes =
            serde_json::to_vec(document).map_err(|e| LibraryError::Invalid(e.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn insert_record(&self, document: &Document) -> Fallible<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        conn.execute(
            "INSERT INTO documents_items (id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![document.id, document.sha256, document.filename, document.media_type, document.byte_len, document.imported_at,
                document.status, document.category, document.project, document.extracted_title, document.extracted_text, document.extractor],
        )?;
        Ok(())
    }

    fn blob_path(&self, id: &str) -> PathBuf {
        self.root.join("blobs").join(&id[..2]).join(id)
    }

    fn write_blob(&self, id: &str, bytes: &[u8]) -> Fallible<()> {
        let target = self.blob_path(id);
        let parent = target
            .parent()
            .ok_or_else(|| LibraryError::Invalid("blob path has no parent".into()))?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = fs::read(&target)?;
                if existing == bytes {
                    return Ok(());
                }
                return Err(LibraryError::Invalid(
                    "a conflicting blob already exists at its digest path".into(),
                ));
            }
            Err(error) => return Err(error.into()),
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn migrate(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents_items (
            id TEXT PRIMARY KEY CHECK (length(id) = 64),
            sha256 TEXT NOT NULL,
            filename TEXT NOT NULL,
            media_type TEXT NOT NULL,
            byte_len INTEGER NOT NULL CHECK (byte_len > 0),
            imported_at TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('inbox', 'filed', 'archived')),
            category TEXT,
            project TEXT,
            extracted_title TEXT,
            extracted_text TEXT,
            extractor TEXT
        );
        CREATE INDEX IF NOT EXISTS documents_items_status_time ON documents_items (status, imported_at DESC);"
    )?;
    let columns = conn
        .prepare("PRAGMA table_info(documents_items)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "sha256") {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN sha256 TEXT NOT NULL DEFAULT ''; UPDATE documents_items SET sha256 = id WHERE sha256 = ''; ")?;
    }
    conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS documents_items_digest_filename ON documents_items (sha256, filename);")?;
    Ok(())
}

fn record_id(sha256: &str, filename: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"sjel-document-record\0");
    hash.update(sha256.as_bytes());
    hash.update(b"\0");
    hash.update(filename.as_bytes());
    hex(&hash.finalize())
}

fn parse_sidecar(bytes: &[u8]) -> Result<Document, serde_json::Error> {
    let mut document: Document = serde_json::from_slice(bytes)?;
    if document.sha256.is_empty() {
        document.sha256 = document.id.clone();
    }
    Ok(document)
}

fn same_record(left: &Document, right: &Document) -> bool {
    left.id == right.id
        && left.sha256 == right.sha256
        && left.filename == right.filename
        && left.media_type == right.media_type
        && left.byte_len == right.byte_len
        && left.imported_at == right.imported_at
        && left.status == right.status
        && left.category == right.category
        && left.project == right.project
        && left.extracted_title == right.extracted_title
        && left.extracted_text == right.extracted_text
        && left.extractor == right.extractor
}

fn read_document(row: &Row<'_>) -> rusqlite::Result<Document> {
    let filename: String = row.get(2)?;
    let extracted_text: Option<String> = row.get(10)?;
    let (suggested_category, suggestion_reason) =
        suggest_category(&filename, extracted_text.as_deref());
    Ok(Document {
        id: row.get(0)?,
        sha256: row.get(1)?,
        filename,
        media_type: row.get(3)?,
        byte_len: row.get(4)?,
        imported_at: row.get(5)?,
        status: row.get(6)?,
        category: row.get(7)?,
        project: row.get(8)?,
        extracted_title: row.get(9)?,
        extracted_text,
        extractor: row.get(11)?,
        suggested_category,
        suggestion_reason,
    })
}

fn extract(media_type: &str, bytes: &[u8]) -> Option<sjel_extraction::Extraction> {
    use sjel_extraction::{Document as Input, InputClass};
    match media_type {
        "application/pdf" => sjel_extraction::for_class(InputClass::Pdf)?
            .extract(&Input::pdf(bytes))
            .ok(),
        "text/plain" => sjel_extraction::for_class(InputClass::PlainText)?
            .extract(&Input::text(bytes))
            .ok(),
        _ => None,
    }
}

fn suggest_category(filename: &str, text: Option<&str>) -> (Option<String>, Option<String>) {
    let evidence = format!("{filename} {}", text.unwrap_or_default()).to_lowercase();
    let category = if ["invoice", "rechnung", "receipt", "rechnungs"]
        .iter()
        .any(|token| evidence.contains(token))
    {
        Some("Finance")
    } else if ["summit", "conference", "congress", "konferenz"]
        .iter()
        .any(|token| evidence.contains(token))
    {
        Some("Events")
    } else if ["manual", "handbuch", "documentation", "guide"]
        .iter()
        .any(|token| evidence.contains(token))
    {
        Some("Reference")
    } else {
        None
    };
    match category {
        Some(category) => (
            Some(category.to_string()),
            Some(
                "filename or extracted text matched a local keyword rule; review before filing"
                    .into(),
            ),
        ),
        None => (None, None),
    }
}

fn media_type(filename: &str) -> &'static str {
    match filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("txt") | Some("md") => "text/plain",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

fn safe_filename(name: &str) -> Fallible<String> {
    if name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
    {
        return Err(LibraryError::Invalid(
            "filename must be a single safe path component (max 255 bytes)".into(),
        ));
    }
    Ok(name.to_string())
}

fn safe_category(category: &str) -> Fallible<&str> {
    if category.is_empty()
        || category.len() > 120
        || category == "."
        || category == ".."
        || category.contains('/')
        || category.contains('\\')
        || category.chars().any(char::is_control)
    {
        return Err(LibraryError::Invalid(
            "category must be a single safe label (max 120 bytes)".into(),
        ));
    }
    Ok(category)
}

fn safe_project(project: &str) -> Fallible<&str> {
    let project = project.trim();
    if project.is_empty() || project.len() > 120 || project.chars().any(char::is_control) {
        return Err(LibraryError::Invalid(
            "project label must be 1..=120 printable characters".into(),
        ));
    }
    Ok(project)
}

fn check_status(status: &str) -> Fallible<()> {
    if matches!(status, "inbox" | "filed" | "archived") {
        Ok(())
    } else {
        Err(LibraryError::Invalid(
            "status must be inbox, filed or archived".into(),
        ))
    }
}

fn check_id(id: &str) -> Fallible<()> {
    if id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(LibraryError::Invalid(
            "document id must be a SHA-256 digest".into(),
        ))
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> (Library, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sjel-documents-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        (
            Library::open(&path.join("test.db"), path.join("library")).unwrap(),
            path,
        )
    }

    #[test]
    fn intake_is_content_addressed_idempotent_and_verified() {
        let (library, root) = scratch();
        let first = library.intake("sample.txt", b"private text").unwrap();
        let Stored::New(document) = first else {
            panic!("first intake must be new")
        };
        assert_eq!(document.status, "inbox");
        assert_eq!(library.bytes(&document.id).unwrap(), b"private text");
        assert!(matches!(
            library.intake("sample.txt", b"private text").unwrap(),
            Stored::Unchanged(_)
        ));
        assert!(library.verify().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_bytes_keep_distinct_source_records_but_share_one_blob() {
        let (library, root) = scratch();
        let Stored::New(first) = library.intake("invoice-a.pdf", b"same bytes").unwrap() else {
            panic!("new document expected")
        };
        let Stored::New(second) = library.intake("invoice-b.pdf", b"same bytes").unwrap() else {
            panic!("distinct source filename must get a record")
        };
        assert_ne!(first.id, second.id);
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(library.list(None).unwrap().len(), 2);
        assert_eq!(
            library.bytes(&first.id).unwrap(),
            library.bytes(&second.id).unwrap()
        );
        assert!(library.verify().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn review_requires_a_safe_category_and_never_changes_original_bytes() {
        let (library, root) = scratch();
        let Stored::New(document) = library.intake("report.pdf", b"not actually a pdf").unwrap()
        else {
            panic!("new")
        };
        assert!(library
            .review(
                &document.id,
                &Review {
                    category: "../Finance".into(),
                    project: None
                }
            )
            .is_err());
        let filed = library
            .review(
                &document.id,
                &Review {
                    category: "Finance".into(),
                    project: Some("GenAI Summit 2026".into()),
                },
            )
            .unwrap();
        assert_eq!(filed.status, "filed");
        assert_eq!(filed.project.as_deref(), Some("GenAI Summit 2026"));
        assert_eq!(library.bytes(&document.id).unwrap(), b"not actually a pdf");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn suggestions_are_advisory_and_conflicts_stay_unfiled() {
        let (library, root) = scratch();
        let Stored::New(invoice) = library.intake("invoice.pdf", b"not a pdf").unwrap() else {
            panic!("new document expected")
        };
        assert_eq!(invoice.suggested_category.as_deref(), Some("Finance"));
        assert_eq!(invoice.status, "inbox");

        let Stored::New(ambiguous) = library
            .intake("summit-invoice.pdf", b"not a pdf either")
            .unwrap()
        else {
            panic!("new document expected")
        };
        assert_eq!(ambiguous.suggested_category.as_deref(), Some("Finance"));
        assert_eq!(ambiguous.status, "inbox");

        let Stored::New(event) = library.intake("summit-agenda.txt", b"agenda").unwrap() else {
            panic!("new document expected")
        };
        assert_eq!(event.suggested_category.as_deref(), Some("Events"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sidecars_rebuild_missing_rows_and_verify_as_a_pair() {
        let (library, root) = scratch();
        let Stored::New(document) = library.intake("invoice.txt", b"invoice number 42").unwrap()
        else {
            panic!("new document expected")
        };
        assert!(library.verify().unwrap().is_empty());
        let conn = library.pool.get().unwrap();
        conn.execute("DELETE FROM documents_items WHERE id = ?1", [&document.id])
            .unwrap();
        assert_eq!(library.restore_index().unwrap(), (1, 0));
        assert_eq!(
            library.get(&document.id).unwrap().unwrap().filename,
            "invoice.txt"
        );
        let issues = library.verify().unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_path_components_and_bad_digests() {
        let (library, root) = scratch();
        assert!(library.intake("../private.pdf", b"x").is_err());
        assert!(library.get("../bad").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

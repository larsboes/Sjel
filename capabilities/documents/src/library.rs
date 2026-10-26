use std::{
    collections::HashSet,
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::review::DuplicateDecision;
use sha2::{Digest, Sha256};

const PREFIX: &str = "documents";
const DOCUMENT_ID_BATCH: usize = 500;
const SEARCH_INDEX_VERSION: i64 = 1;
pub(crate) const MAX_BYTES: usize = 128 * 1024 * 1024;

type Fallible<T> = Result<T, LibraryError>;

#[derive(Debug)]
pub enum LibraryError {
    Invalid(String),
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Store(String),
    Model(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Io(error) => write!(f, "document file operation: {error}"),
            Self::Sql(error) => write!(f, "document metadata: {error}"),
            Self::Store(error) => write!(f, "document store: {error}"),
            Self::Model(error) => write!(f, "local document model: {error}"),
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
    #[serde(default)]
    pub pages: Vec<sjel_extraction::ExtractedPage>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub page_range: Option<crate::split::PageRange>,
    pub suggested_category: Option<String>,
    pub suggestion_reason: Option<String>,
}

/// A page of documents ranked by how many query terms match locally stored text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSearchPage {
    pub documents: Vec<Document>,
    pub matched_pages: Vec<Vec<u32>>,
    pub total: usize,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationSidecar {
    version: u32,
    document_id: String,
    proposals: Vec<crate::classification::StoredProposal>,
}

pub struct Library {
    pool: sjel_store::Pool,
    root: PathBuf,
    classification: crate::classification::ClassificationConfig,
    rule_proposal_lock: Mutex<()>,
}

impl Library {
    /// Open the prefixed shared-store tables and local canonical file root.
    ///
    /// # Errors
    /// The database cannot be migrated, or the root cannot be created.
    pub fn open(database: &Path, root: PathBuf) -> Fallible<Self> {
        Self::open_with_config(
            database,
            root,
            crate::classification::ClassificationConfig::default(),
        )
    }

    pub(crate) fn open_with_config(
        database: &Path,
        root: PathBuf,
        classification: crate::classification::ClassificationConfig,
    ) -> Fallible<Self> {
        let blobs = root.join("blobs");
        let records = root.join("records");
        let classifications = records.join("classification");
        fs::create_dir_all(&blobs)?;
        fs::create_dir_all(&records)?;
        fs::create_dir_all(&classifications)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&blobs, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&records, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&classifications, fs::Permissions::from_mode(0o700))?;
        let pool = sjel_store::open_pool(database, PREFIX, migrate)
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let library = Self {
            pool,
            root,
            classification,
            rule_proposal_lock: Mutex::new(()),
        };
        Ok(library)
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
        let extracted_pages = extracted
            .as_ref()
            .and_then(|value| serde_json::to_string(&value.pages).ok())
            .unwrap_or_else(|| "[]".into());
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
            pages: extracted
                .as_ref()
                .map_or_else(Vec::new, |e| e.pages.clone()),
            parent_id: None,
            page_range: None,
            suggested_category: None,
            suggestion_reason: None,
        };
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO documents_items
             (id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'inbox', NULL, NULL, ?7, ?8, ?9, ?10, NULL, NULL, NULL)",
            params![document.id, document.sha256, document.filename, document.media_type, document.byte_len,
                document.imported_at, document.extracted_title, document.extracted_text, document.extractor, extracted_pages],
        )?;
        let saved = self.get(&id)?.ok_or_else(|| {
            LibraryError::Invalid("intake did not create a document record".into())
        })?;
        if inserted == 1 {
            self.ensure_rule_proposals(&saved.id)?;
        }
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
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE status = ?1 ORDER BY imported_at DESC, filename"
        } else {
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items ORDER BY imported_at DESC, filename"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(status) = status {
            stmt.query_map([status], read_document)?
        } else {
            stmt.query_map([], read_document)?
        };
        let documents = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(documents
            .into_iter()
            .map(|document| self.with_configured_suggestion(document))
            .collect())
    }

    /// Read one recent page of document metadata without loading the full library.
    pub fn list_page(
        &self,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Fallible<(Vec<Document>, usize)> {
        if let Some(status) = status {
            check_status(status)?;
        }
        let limit = i64::try_from(limit)
            .map_err(|_| LibraryError::Invalid("document page limit is too large".into()))?;
        let offset = i64::try_from(offset)
            .map_err(|_| LibraryError::Invalid("document page offset is too large".into()))?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let (count_sql, page_sql) = if status.is_some() {
            (
                "SELECT COUNT(*) FROM documents_items WHERE status = ?1",
                "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE status = ?1 ORDER BY imported_at DESC, filename LIMIT ?2 OFFSET ?3",
            )
        } else {
            (
                "SELECT COUNT(*) FROM documents_items",
                "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items ORDER BY imported_at DESC, filename LIMIT ?1 OFFSET ?2",
            )
        };
        let total: i64 = if let Some(status) = status {
            conn.query_row(count_sql, [status], |row| row.get(0))?
        } else {
            conn.query_row(count_sql, [], |row| row.get(0))?
        };
        let total = usize::try_from(total)
            .map_err(|_| LibraryError::Invalid("document count is out of range".into()))?;
        let mut statement = conn.prepare(page_sql)?;
        let rows = if let Some(status) = status {
            statement.query_map(params![status, limit, offset], read_document)?
        } else {
            statement.query_map(params![limit, offset], read_document)?
        };
        let documents = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|document| self.with_configured_suggestion(document))
            .collect();
        Ok((documents, total))
    }

    /// Search indexed document text locally with token prefixes, loading full records only for this page.
    ///
    /// # Errors
    /// The query is empty, pagination exceeds the supported range, or the store cannot be read.
    pub fn search(
        &self,
        query: &str,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Fallible<DocumentSearchPage> {
        if let Some(status) = status {
            check_status(status)?;
        }
        if !(1..=500).contains(&limit) || offset > 1_000_000 {
            return Err(LibraryError::Invalid(
                "search pagination is out of range".into(),
            ));
        }
        let terms = search_terms(query);
        if terms.is_empty() {
            return Err(LibraryError::Invalid(
                "search needs at least one word".into(),
            ));
        }
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let fts_query = terms
            .iter()
            .map(|term| format!("{term}*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut statement = conn.prepare(
            "SELECT d.id, d.filename, d.extracted_title, d.extracted_text, d.category, d.project, d.imported_at
             FROM documents_search
             JOIN documents_items d ON d.rowid = documents_search.rowid
             WHERE documents_search MATCH ?1 AND (?2 IS NULL OR d.status = ?2)",
        )?;
        let rows = statement.query_map(params![fts_query, status], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut matches = Vec::new();
        for row in rows {
            let (id, filename, title, text, category, project, imported_at) = row?;
            let filename = filename.to_lowercase();
            let title = title.unwrap_or_default().to_lowercase();
            let text = text.unwrap_or_default().to_lowercase();
            let category = category.unwrap_or_default().to_lowercase();
            let project = project.unwrap_or_default().to_lowercase();
            let score = terms
                .iter()
                .filter(|term| {
                    filename.contains(term.as_str())
                        || title.contains(term.as_str())
                        || text.contains(term.as_str())
                        || category.contains(term.as_str())
                        || project.contains(term.as_str())
                })
                .count();
            if score > 0 {
                matches.push((score, imported_at, id));
            }
        }
        matches.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let total = matches.len();
        let selected = matches
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, _, id)| id)
            .collect::<Vec<_>>();
        let mut documents = Vec::with_capacity(selected.len());
        let mut matched_pages = Vec::with_capacity(selected.len());
        for id in selected {
            let Some(document) = self.get(&id)? else {
                continue;
            };
            matched_pages.push(
                document
                    .pages
                    .iter()
                    .filter(|page| {
                        let page_text = page.text.to_lowercase();
                        terms.iter().any(|term| page_text.contains(term.as_str()))
                    })
                    .map(|page| page.page_number)
                    .collect(),
            );
            documents.push(self.with_configured_suggestion(document));
        }
        Ok(DocumentSearchPage {
            documents,
            matched_pages,
            total,
        })
    }

    /// Read child records for one original in page order.
    ///
    /// # Errors
    /// The parent id is invalid or the metadata query fails.
    pub fn children(&self, parent_id: &str) -> Fallible<Vec<Document>> {
        check_id(parent_id)?;
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let mut statement = conn.prepare(
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE parent_id = ?1 ORDER BY page_first, id",
        )?;
        let rows = statement.query_map([parent_id], read_document)?;
        rows.map(|row| row.map(|document| self.with_configured_suggestion(document)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Generate deterministic proposals from the library's active configuration.
    pub fn proposals(
        &self,
        document: &Document,
    ) -> Vec<crate::classification::ClassificationProposal> {
        self.classification.proposals_for(
            &document.id,
            &document.filename,
            document.extracted_text.as_deref(),
        )
    }

    /// Re-run local extraction for one original and persist page-level evidence.
    ///
    /// # Errors
    /// The id is invalid, the record is a child, extraction is unavailable, or storage fails.
    pub fn analyze(&self, id: &str) -> Fallible<Document> {
        check_id(id)?;
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        if document.parent_id.is_some() {
            return Err(LibraryError::Invalid(
                "analyze the original document before splitting it".into(),
            ));
        }
        let bytes = self.read_blob(&document.sha256)?;
        let extraction = extract(&document.media_type, &bytes)
            .ok_or_else(|| LibraryError::Invalid("no local extractor is available".into()))?;
        let pages_json = serde_json::to_string(&extraction.pages)
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        let previous_pages_json = serde_json::to_string(&document.pages)
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        let extraction_changed = document.extracted_title != extraction.title
            || document.extracted_text.as_deref() != Some(extraction.text.as_str())
            || document.extractor.as_deref() != Some(extraction.producer)
            || previous_pages_json != pages_json;
        if !extraction_changed {
            self.ensure_rule_proposals(id)?;
            return Ok(document);
        }
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        conn.execute(
            "UPDATE documents_items SET extracted_title = ?2, extracted_text = ?3, extractor = ?4, extracted_pages_json = ?5 WHERE id = ?1",
            params![id, extraction.title, extraction.text, extraction.producer, pages_json],
        )?;
        let analyzed = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("analyzed document disappeared".into()))?;
        self.generate_rule_proposals(&analyzed, new_generation_id()?)?;
        self.write_record(&analyzed)?;
        Ok(analyzed)
    }

    /// Suggest page ranges from explicit page-one markers without writing child records.
    ///
    /// # Errors
    /// The id is invalid, the record is not an original PDF, or its metadata cannot be read.
    pub fn split_suggestions(&self, id: &str) -> Fallible<crate::split::SplitSuggestion> {
        check_id(id)?;
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        if document.parent_id.is_some() || document.media_type != "application/pdf" {
            return Err(LibraryError::Invalid(
                "split suggestions require an original PDF".into(),
            ));
        }
        Ok(crate::split::suggest_ranges(&document.pages))
    }

    /// Create idempotent child PDFs for a complete, human-approved page partition.
    ///
    /// Each child has its own content-addressed bytes and retains page-level lineage to the
    /// unchanged parent record.
    ///
    /// # Errors
    /// The parent extraction is incomplete, the partition is invalid, the PDF cannot be safely
    /// split, a child filename is invalid or repeated, a prior partition conflicts, or persistence
    /// fails.
    pub fn split(
        &self,
        parent_id: &str,
        request: &crate::split::SplitRequest,
    ) -> Fallible<Vec<Document>> {
        check_id(parent_id)?;
        let parent = self
            .get(parent_id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        if parent.parent_id.is_some() || parent.media_type != "application/pdf" {
            return Err(LibraryError::Invalid(
                "only an original PDF can be split".into(),
            ));
        }
        let page_count = u32::try_from(parent.pages.len())
            .map_err(|_| LibraryError::Invalid("page count exceeds supported range".into()))?;
        if request.page_count != page_count
            || parent
                .pages
                .iter()
                .enumerate()
                .any(|(index, page)| page.page_number != index as u32 + 1)
        {
            return Err(LibraryError::Invalid(
                "analyze the original to record every page before splitting".into(),
            ));
        }
        let ranges = request
            .children
            .iter()
            .map(|child| crate::split::PageRange::new(child.first, child.last))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        crate::split::Partition::new(request.page_count, ranges.clone())
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;

        let source_pdf = self.read_blob(&parent.sha256)?;
        let mut filenames = std::collections::BTreeSet::new();
        let mut children = Vec::with_capacity(request.children.len());
        let mut child_blobs = std::collections::BTreeMap::new();
        for (input, page_range) in request.children.iter().zip(ranges) {
            let filename = safe_filename(&input.filename)?;
            if !Path::new(&filename)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                return Err(LibraryError::Invalid(
                    "child filenames must use the .pdf extension".into(),
                ));
            }
            if !filenames.insert(filename.clone()) {
                return Err(LibraryError::Invalid(
                    "child filenames must be unique".into(),
                ));
            }
            let pages: Vec<_> = parent
                .pages
                .iter()
                .filter(|page| {
                    page_range.first <= page.page_number && page.page_number <= page_range.last
                })
                .cloned()
                .collect();
            let text = pages
                .iter()
                .map(|page| page.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n\u{000c}\n\n");
            let bytes = crate::split::materialize_pdf_range(&source_pdf, page_range, page_count)
                .map_err(LibraryError::Invalid)?;
            let sha256 = hex(&Sha256::digest(&bytes));
            let byte_len = i64::try_from(bytes.len()).map_err(|_| {
                LibraryError::Invalid("child PDF exceeds the supported size".into())
            })?;
            let id = child_record_id(parent_id, page_range, &filename);
            child_blobs.insert(id.clone(), bytes);
            children.push(Document {
                id,
                sha256,
                filename,
                media_type: parent.media_type.clone(),
                byte_len,
                imported_at: chrono::Utc::now().to_rfc3339(),
                status: "inbox".into(),
                category: None,
                project: None,
                extracted_title: None,
                extracted_text: Some(text),
                extractor: parent.extractor.clone(),
                pages,
                parent_id: Some(parent_id.to_owned()),
                page_range: Some(page_range),
                suggested_category: None,
                suggestion_reason: None,
            });
        }

        let existing_children: Vec<Document> = self
            .list(None)?
            .into_iter()
            .filter(|document| document.parent_id.as_deref() == Some(parent_id))
            .collect();
        if !existing_children.is_empty() {
            if existing_children.len() != children.len()
                || !children.iter().all(|child| {
                    existing_children.iter().any(|existing| {
                        existing.id == child.id
                            && existing.page_range == child.page_range
                            && existing.filename == child.filename
                            && existing.pages == child.pages
                    })
                })
            {
                return Err(LibraryError::Invalid(
                    "this original already has a different split; child records are retained"
                        .into(),
                ));
            }

            let mut refreshed = Vec::with_capacity(children.len());
            let mut conn = self
                .pool
                .get()
                .map_err(|error| LibraryError::Store(error.to_string()))?;
            let transaction = sjel_store::write_transaction(&mut conn)?;
            for expected in &children {
                let mut existing = existing_children
                    .iter()
                    .find(|existing| existing.id == expected.id)
                    .cloned()
                    .ok_or_else(|| LibraryError::Invalid("split child disappeared".into()))?;
                if existing.sha256 == parent.sha256 {
                    let bytes = child_blobs.get(&expected.id).ok_or_else(|| {
                        LibraryError::Invalid("materialized child bytes disappeared".into())
                    })?;
                    self.write_blob(&expected.sha256, bytes)?;
                    transaction.execute(
                        "UPDATE documents_items SET sha256 = ?2, byte_len = ?3 WHERE id = ?1",
                        params![existing.id, expected.sha256, expected.byte_len],
                    )?;
                    existing.sha256 = expected.sha256.clone();
                    existing.byte_len = expected.byte_len;
                    refreshed.push(existing);
                } else {
                    refreshed.push(existing);
                }
            }
            transaction.commit()?;
            for child in &refreshed {
                self.write_record(child)?;
            }
            return Ok(refreshed);
        }
        for child in &children {
            if self
                .by_hash_and_name(&child.sha256, &child.filename)?
                .is_some()
            {
                return Err(LibraryError::Invalid(format!(
                    "child filename already exists in this library: {}",
                    child.filename
                )));
            }
            let bytes = child_blobs.get(&child.id).ok_or_else(|| {
                LibraryError::Invalid("materialized child bytes disappeared".into())
            })?;
            self.write_blob(&child.sha256, bytes)?;
        }

        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        for child in &children {
            transaction.execute(
                "INSERT INTO documents_items (id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'inbox', NULL, NULL, NULL, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![child.id, child.sha256, child.filename, child.media_type, child.byte_len,
                    child.imported_at, child.extracted_text, child.extractor,
                    serde_json::to_string(&child.pages).map_err(|error| LibraryError::Invalid(error.to_string()))?,
                    parent_id, child.page_range.map(|range| range.first), child.page_range.map(|range| range.last)],
            )?;
        }
        transaction.commit()?;
        for child in &children {
            self.generate_rule_proposals(child, new_generation_id()?)?;
            self.write_record(child)?;
        }
        Ok(children)
    }

    /// Read the immutable proposal records and their append-only review events.
    ///
    /// # Errors
    /// The document id is invalid, storage fails, or a stored record is malformed.
    pub fn proposal_records(
        &self,
        document_id: &str,
    ) -> Fallible<Vec<crate::classification::StoredProposal>> {
        check_id(document_id)?;
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let rows = {
            let mut statement = conn.prepare(
                "SELECT proposal_id, generation_id, generated_at, proposal_json
                 FROM documents_classification_proposals
                 WHERE document_id = ?1 ORDER BY generated_at, proposal_id",
            )?;
            let rows = statement
                .query_map([document_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut records = Vec::with_capacity(rows.len());
        for (id, generation_id, generated_at, proposal_json) in rows {
            let mut proposal: crate::classification::ClassificationProposal =
                serde_json::from_str(&proposal_json).map_err(|error| {
                    LibraryError::Invalid(format!("stored classification proposal: {error}"))
                })?;
            let review_rows = {
                let mut statement = conn.prepare(
                    "SELECT review_id, outcome, reviewed_at
                     FROM documents_classification_reviews
                     WHERE proposal_id = ?1 ORDER BY review_id",
                )?;
                let rows = statement
                    .query_map([&id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            };
            let review_history = review_rows
                .into_iter()
                .map(|(id, outcome, reviewed_at)| {
                    let outcome = match outcome.as_str() {
                        "accepted" => crate::classification::ProposalReview::Accepted,
                        "rejected" => crate::classification::ProposalReview::Rejected,
                        "overridden" => crate::classification::ProposalReview::Overridden,
                        _ => {
                            return Err(LibraryError::Invalid(
                                "stored proposal review is invalid".into(),
                            ));
                        }
                    };
                    Ok(crate::classification::ProposalReviewEvent {
                        id,
                        outcome,
                        reviewed_at,
                    })
                })
                .collect::<Fallible<Vec<_>>>()?;
            proposal.review = review_history.last().map(|event| event.outcome);
            records.push(crate::classification::StoredProposal {
                id,
                generation_id,
                generated_at,
                proposal,
                review_history,
            });
        }
        Ok(records)
    }

    /// Return document ids whose newest proposal run contains an unreviewed category proposal.
    ///
    /// # Errors
    /// The store cannot be read, or a stored proposal is malformed.
    pub fn pending_proposal_documents(&self, document_ids: &[String]) -> Fallible<HashSet<String>> {
        if document_ids.is_empty() {
            return Ok(HashSet::new());
        }
        for id in document_ids {
            check_id(id)?;
        }
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let mut pending = HashSet::new();
        for batch in document_ids.chunks(DOCUMENT_ID_BATCH) {
            let placeholders = vec!["?"; batch.len()].join(", ");
            let sql = format!(
                "SELECT p.document_id, p.proposal_json
                 FROM documents_classification_proposals p
                 LEFT JOIN documents_classification_reviews r ON r.proposal_id = p.proposal_id
                 WHERE p.document_id IN ({placeholders})
                   AND p.generation_id = (
                     SELECT newest.generation_id
                     FROM documents_classification_proposals newest
                     WHERE newest.document_id = p.document_id
                     ORDER BY newest.generated_at DESC, newest.generation_id DESC
                     LIMIT 1
                   )
                 GROUP BY p.proposal_id
                 HAVING COUNT(r.review_id) = 0"
            );
            let rows = {
                let mut statement = conn.prepare(&sql)?;
                let mapped = statement
                    .query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?;
                mapped.collect::<Result<Vec<_>, _>>()?
            };
            for (document_id, proposal_json) in rows {
                let proposal: crate::classification::ClassificationProposal =
                    serde_json::from_str(&proposal_json).map_err(|error| {
                        LibraryError::Invalid(format!("stored classification proposal: {error}"))
                    })?;
                if matches!(
                    proposal.outcome,
                    crate::classification::ProposalOutcome::Proposed { .. }
                ) {
                    pending.insert(document_id);
                }
            }
        }
        Ok(pending)
    }

    /// Return parent ids that already have stored split children.
    ///
    /// # Errors
    /// The store cannot be read, or a document id is invalid.
    pub fn documents_with_children(&self, document_ids: &[String]) -> Fallible<HashSet<String>> {
        if document_ids.is_empty() {
            return Ok(HashSet::new());
        }
        for id in document_ids {
            check_id(id)?;
        }
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let mut parents = HashSet::new();
        for batch in document_ids.chunks(DOCUMENT_ID_BATCH) {
            let placeholders = vec!["?"; batch.len()].join(", ");
            let sql = format!(
                "SELECT DISTINCT parent_id FROM documents_items
                 WHERE parent_id IN ({placeholders})"
            );
            let mut statement = conn.prepare(&sql)?;
            parents.extend(
                statement
                    .query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        Ok(parents)
    }

    /// Ensure the selected document has a proposal run for the active rule-set version.
    ///
    /// # Errors
    /// The document does not exist or proposal persistence fails.
    pub fn ensure_rule_proposals(&self, document_id: &str) -> Fallible<()> {
        check_id(document_id)?;
        let _guard = self
            .rule_proposal_lock
            .lock()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let document = self
            .get(document_id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let has_current_version = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM documents_classification_proposals
             WHERE document_id = ?1 AND source_kind = 'rules' AND source_version = ?2)",
            params![document_id, self.classification.version()],
            |row| row.get::<_, bool>(0),
        )?;
        if !has_current_version {
            self.generate_rule_proposals(&document, new_generation_id()?)?;
        }
        Ok(())
    }

    /// Append one review event without changing the proposal or filing state.
    ///
    /// # Errors
    /// The document or proposal does not exist, or persistence fails.
    pub fn review_proposal(
        &self,
        document_id: &str,
        proposal_id: &str,
        outcome: crate::classification::ProposalReview,
    ) -> Fallible<crate::classification::StoredProposal> {
        check_id(document_id)?;
        check_id(proposal_id)?;
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let proposal_document = transaction
            .query_row(
                "SELECT document_id FROM documents_classification_proposals WHERE proposal_id = ?1",
                [proposal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| LibraryError::Invalid("proposal not found".into()))?;
        if proposal_document != document_id {
            return Err(LibraryError::Invalid("proposal not found".into()));
        }
        transaction.execute(
            "INSERT INTO documents_classification_reviews (proposal_id, outcome, reviewed_at)
             VALUES (?1, ?2, ?3)",
            params![
                proposal_id,
                match outcome {
                    crate::classification::ProposalReview::Accepted => "accepted",
                    crate::classification::ProposalReview::Rejected => "rejected",
                    crate::classification::ProposalReview::Overridden => "overridden",
                },
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        transaction.commit()?;
        self.write_classification_sidecar(document_id)?;
        self.proposal_records(document_id)?
            .into_iter()
            .find(|record| record.id == proposal_id)
            .ok_or_else(|| LibraryError::Invalid("reviewed proposal disappeared".into()))
    }

    fn backfill_rule_proposals(&self) -> Fallible<()> {
        for document in self.list(None)? {
            let conn = self
                .pool
                .get()
                .map_err(|error| LibraryError::Store(error.to_string()))?;
            let has_current_version = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM documents_classification_proposals
                 WHERE document_id = ?1 AND source_kind = 'rules' AND source_version = ?2)",
                params![document.id, self.classification.version()],
                |row| row.get::<_, bool>(0),
            )?;
            if !has_current_version {
                self.generate_rule_proposals(&document, new_generation_id()?)?;
            } else {
                self.write_classification_sidecar(&document.id)?;
            }
        }
        Ok(())
    }

    /// Backfill active local rules for one bounded range of documents.
    ///
    /// Returns `true` when the current rule version has reached the current library tail.
    ///
    /// # Errors
    /// A proposal cannot be stored or the progress checkpoint cannot be updated.
    pub(crate) fn backfill_rule_proposal_batch(&self, limit: usize) -> Fallible<bool> {
        if !(1..=256).contains(&limit) {
            return Err(LibraryError::Invalid(
                "proposal backfill batch must be 1..=256".into(),
            ));
        }
        let version = self.classification.version();
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let high_water = conn
            .query_row(
                "SELECT high_water_rowid FROM documents_classification_backfills WHERE version = ?1",
                [version],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or_default();
        let latest: i64 = conn.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM documents_items",
            [],
            |row| row.get(0),
        )?;
        let documents = {
            let mut statement = conn.prepare(
                "SELECT item.rowid, item.id FROM documents_items AS item
                 WHERE NOT EXISTS (
                    SELECT 1 FROM documents_classification_proposals AS proposal
                    WHERE proposal.document_id = item.id
                      AND proposal.source_kind = 'rules'
                      AND proposal.source_version = ?1
                 )
                 ORDER BY item.rowid LIMIT ?2",
            )?;
            let documents = statement
                .query_map(params![version, limit as i64], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            documents
        };
        drop(conn);

        for (_, document_id) in &documents {
            self.ensure_rule_proposals(document_id)?;
        }

        let completed = documents
            .last()
            .map(|(rowid, _)| *rowid)
            .unwrap_or(latest.max(high_water));

        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        conn.execute(
            "INSERT INTO documents_classification_backfills (version, high_water_rowid)
             VALUES (?1, ?2)
             ON CONFLICT(version) DO UPDATE SET high_water_rowid = MAX(high_water_rowid, excluded.high_water_rowid)",
            params![version, completed],
        )?;
        Ok(documents.is_empty())
    }

    /// Generate a fresh rule-proposal run and append its records.
    ///
    /// # Errors
    /// The document is missing or proposal persistence fails.
    pub fn regenerate_rule_proposals(
        &self,
        document_id: &str,
    ) -> Fallible<Vec<crate::classification::StoredProposal>> {
        let document = self
            .get(document_id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        let generation_id = new_generation_id()?;
        self.generate_rule_proposals(&document, generation_id.clone())?;
        Ok(self
            .proposal_records(document_id)?
            .into_iter()
            .filter(|record| record.generation_id == generation_id)
            .collect())
    }

    /// Ask the configured local role for one reviewable classification proposal.
    pub fn generate_model_proposal(
        &self,
        document_id: &str,
    ) -> Fallible<Vec<crate::classification::StoredProposal>> {
        let document = self
            .get(document_id)?
            .ok_or_else(|| LibraryError::Invalid("no document with that id".into()))?;
        let proposal = crate::classification_model::propose(&document, &self.classification)?;
        let generation_id = new_generation_id()?;
        self.store_proposals(&document, &generation_id, vec![proposal])?;
        Ok(self
            .proposal_records(document_id)?
            .into_iter()
            .filter(|record| record.generation_id == generation_id)
            .collect())
    }

    fn generate_rule_proposals(&self, document: &Document, generation_id: String) -> Fallible<()> {
        let generated_at = chrono::Utc::now().to_rfc3339();
        let proposals = self.proposals(document);
        self.store_proposals_at(document, &generation_id, proposals, &generated_at)
    }

    fn store_proposals(
        &self,
        document: &Document,
        generation_id: &str,
        proposals: Vec<crate::classification::ClassificationProposal>,
    ) -> Fallible<()> {
        let generated_at = chrono::Utc::now().to_rfc3339();
        self.store_proposals_at(document, generation_id, proposals, &generated_at)
    }

    fn store_proposals_at(
        &self,
        document: &Document,
        generation_id: &str,
        proposals: Vec<crate::classification::ClassificationProposal>,
        generated_at: &str,
    ) -> Fallible<()> {
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        for mut proposal in proposals {
            proposal.review = None;
            let proposal_json = serde_json::to_string(&proposal)
                .map_err(|error| LibraryError::Invalid(error.to_string()))?;
            let (source_kind, identity, version) = match &proposal.source {
                crate::classification::ProposalSource::Rules { identity, version } => {
                    ("rules", identity.as_str(), version.as_str())
                }
                crate::classification::ProposalSource::Model {
                    role,
                    model,
                    revision,
                } => ("model", role.as_str(), revision.as_deref().unwrap_or(model)),
            };
            let proposal_id =
                classification_proposal_id(&document.id, generation_id, &proposal_json);
            transaction.execute(
                "INSERT INTO documents_classification_proposals
                 (proposal_id, document_id, generation_id, source_kind, source_identity, source_version, proposal_json, generated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    proposal_id,
                    document.id,
                    generation_id,
                    source_kind,
                    identity,
                    version,
                    proposal_json,
                    generated_at,
                ],
            )?;
        }
        transaction.commit()?;
        self.write_classification_sidecar(&document.id)
    }

    fn write_classification_sidecar(&self, document_id: &str) -> Fallible<()> {
        let sidecar = ClassificationSidecar {
            version: 2,
            document_id: document_id.to_string(),
            proposals: self.proposal_records(document_id)?,
        };
        let path = self.classification_record_path(document_id);
        let parent = path.parent().ok_or_else(|| {
            LibraryError::Invalid("classification sidecar path has no parent".into())
        })?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| LibraryError::Invalid(error.to_string()))?
            .as_nanos();
        let temp = parent.join(format!(
            ".{document_id}.{}.{}.tmp",
            std::process::id(),
            stamp
        ));
        let bytes = serde_json::to_vec(&sidecar)
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
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

    fn with_configured_suggestion(&self, mut document: Document) -> Document {
        let category = self.proposals(&document).into_iter().find_map(|proposal| {
            if let crate::classification::ProposalOutcome::Proposed { category, .. } =
                proposal.outcome
            {
                Some(category)
            } else {
                None
            }
        });
        document.suggested_category = category;
        document.suggestion_reason = document
            .suggested_category
            .as_ref()
            .map(|_| "a configured local rule matched; review before filing".into());
        document
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
        Ok(conn
            .query_row(
                "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE id = ?1",
                [id],
                read_document,
            )
            .optional()?
            .map(|document| self.with_configured_suggestion(document)))
    }

    fn by_hash_and_name(&self, sha256: &str, filename: &str) -> Fallible<Option<Document>> {
        check_id(sha256)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        Ok(conn
            .query_row(
                "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE sha256 = ?1 AND filename = ?2",
                params![sha256, filename],
                read_document,
            )
            .optional()?
            .map(|document| self.with_configured_suggestion(document)))
    }

    /// Every record holding these bytes, under any filename.
    ///
    /// # Errors
    /// The digest is malformed or the read fails.
    pub fn by_hash(&self, sha256: &str) -> Fallible<Vec<Document>> {
        check_id(sha256)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last FROM documents_items WHERE sha256 = ?1 ORDER BY imported_at, filename",
        )?;
        let rows = stmt.query_map([sha256], read_document)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Record that folder intake found this document at `path`.
    ///
    /// # Errors
    /// The id is malformed, the path is not UTF-8, or the write fails.
    pub fn record_source(&self, id: &str, path: &Path) -> Fallible<()> {
        check_id(id)?;
        let path = path
            .to_str()
            .ok_or_else(|| LibraryError::Invalid("source path is not UTF-8".into()))?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        conn.execute(
            "INSERT INTO documents_sources (id, path, seen_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (id, path) DO UPDATE SET seen_at = excluded.seen_at",
            params![id, path, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Every recorded source path, as `(id, path)`, ordered by id then path.
    ///
    /// # Errors
    /// The metadata query fails.
    pub fn sources(&self) -> Fallible<Vec<(String, String)>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT id, path FROM documents_sources ORDER BY id, path")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Read saved human decisions for duplicate candidates.
    ///
    /// # Errors
    /// The review table cannot be read or a persisted decision is malformed.
    pub fn duplicate_decisions(&self) -> Fallible<Vec<DuplicateDecision>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut statement = conn.prepare(
            "SELECT decision_json FROM documents_duplicate_reviews ORDER BY candidate_key",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let json = row?;
            serde_json::from_str(&json).map_err(|error| {
                LibraryError::Invalid(format!("stored duplicate decision: {error}"))
            })
        })
        .collect()
    }

    /// Persist an operator decision without deleting or changing document records or originals.
    ///
    /// # Errors
    /// The decision is invalid, a referenced record is missing, or persistence fails.
    pub fn save_duplicate_decision(&self, decision: &DuplicateDecision) -> Fallible<()> {
        decision
            .validate()
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        let decision_json = serde_json::to_string(decision)
            .map_err(|error| LibraryError::Invalid(format!("duplicate decision: {error}")))?;
        let mut conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        for id in &decision.member_ids {
            let exists = transaction
                .query_row("SELECT 1 FROM documents_items WHERE id = ?1", [id], |row| {
                    row.get::<_, i64>(0)
                })
                .optional()?
                .is_some();
            if !exists {
                return Err(LibraryError::Invalid(
                    "duplicate candidate includes a missing document".into(),
                ));
            }
        }
        transaction.execute(
            "INSERT INTO documents_duplicate_reviews (candidate_key, decision_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (candidate_key) DO UPDATE SET
                 decision_json = excluded.decision_json,
                 updated_at = excluded.updated_at",
            params![
                decision.candidate_key,
                decision_json,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        transaction.commit()?;
        Ok(())
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
            let expected_id = match (document.parent_id.as_deref(), document.page_range) {
                (None, None) => record_id(&document.sha256, &document.filename),
                (Some(parent_id), Some(range)) => {
                    check_id(parent_id)?;
                    child_record_id(parent_id, range, &document.filename)
                }
                _ => {
                    return Err(LibraryError::Invalid(
                        "sidecar has incomplete child lineage".into(),
                    ));
                }
            };
            if expected_id != document.id {
                return Err(LibraryError::Invalid(
                    "sidecar id does not match its source lineage".into(),
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
        self.restore_proposal_sidecars()?;
        self.backfill_rule_proposals()?;
        Ok((restored, unchanged))
    }

    fn restore_proposal_sidecars(&self) -> Fallible<()> {
        let directory = self.root.join("records").join("classification");
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let sidecar: ClassificationSidecar = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| {
                    LibraryError::Invalid(format!("invalid classification sidecar: {error}"))
                })?;
            check_id(&sidecar.document_id)?;
            if sidecar.version != 2
                || path.file_stem().and_then(|name| name.to_str())
                    != Some(sidecar.document_id.as_str())
            {
                return Err(LibraryError::Invalid(
                    "classification sidecar filename or version is invalid".into(),
                ));
            }
            if self.get(&sidecar.document_id)?.is_none() {
                continue;
            }
            let mut conn = self
                .pool
                .get()
                .map_err(|error| LibraryError::Store(error.to_string()))?;
            let transaction = sjel_store::write_transaction(&mut conn)?;
            for record in sidecar.proposals {
                check_id(&record.id)?;
                if record.proposal.document_id != sidecar.document_id {
                    return Err(LibraryError::Invalid(
                        "classification proposal belongs to another document".into(),
                    ));
                }
                if record
                    .review_history
                    .windows(2)
                    .any(|events| events[0].id >= events[1].id)
                    || record.review_history.iter().any(|event| event.id <= 0)
                    || record.proposal.review
                        != record.review_history.last().map(|event| event.outcome)
                {
                    return Err(LibraryError::Invalid(
                        "classification proposal review history is invalid".into(),
                    ));
                }
                let mut proposal = record.proposal;
                proposal.review = None;
                let proposal_json = serde_json::to_string(&proposal)
                    .map_err(|error| LibraryError::Invalid(error.to_string()))?;
                if classification_proposal_id(
                    &sidecar.document_id,
                    &record.generation_id,
                    &proposal_json,
                ) != record.id
                {
                    return Err(LibraryError::Invalid(
                        "classification proposal id does not match its contents".into(),
                    ));
                }
                let (source_kind, identity, version) = match &proposal.source {
                    crate::classification::ProposalSource::Rules { identity, version } => {
                        ("rules", identity.as_str(), version.as_str())
                    }
                    crate::classification::ProposalSource::Model {
                        role,
                        model,
                        revision,
                    } => ("model", role.as_str(), revision.as_deref().unwrap_or(model)),
                };
                transaction.execute(
                    "INSERT OR IGNORE INTO documents_classification_proposals
                     (proposal_id, document_id, generation_id, source_kind, source_identity, source_version, proposal_json, generated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![record.id, sidecar.document_id, record.generation_id, source_kind, identity, version, proposal_json, record.generated_at],
                )?;
                let saved_proposal = transaction.query_row(
                    "SELECT document_id, generation_id, proposal_json, generated_at
                     FROM documents_classification_proposals WHERE proposal_id = ?1",
                    [&record.id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?;
                if saved_proposal
                    != (
                        sidecar.document_id.clone(),
                        record.generation_id.clone(),
                        proposal_json,
                        record.generated_at.clone(),
                    )
                {
                    return Err(LibraryError::Invalid(
                        "classification proposal conflicts with stored history".into(),
                    ));
                }
                for event in record.review_history {
                    let outcome = match event.outcome {
                        crate::classification::ProposalReview::Accepted => "accepted",
                        crate::classification::ProposalReview::Rejected => "rejected",
                        crate::classification::ProposalReview::Overridden => "overridden",
                    };
                    transaction.execute(
                        "INSERT OR IGNORE INTO documents_classification_reviews
                         (review_id, proposal_id, outcome, reviewed_at) VALUES (?1, ?2, ?3, ?4)",
                        params![event.id, record.id, outcome, event.reviewed_at],
                    )?;
                    let saved_event = transaction.query_row(
                        "SELECT proposal_id, outcome, reviewed_at
                         FROM documents_classification_reviews WHERE review_id = ?1",
                        [event.id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )?;
                    if saved_event != (record.id.clone(), outcome.to_string(), event.reviewed_at) {
                        return Err(LibraryError::Invalid(
                            "classification review conflicts with stored history".into(),
                        ));
                    }
                }
            }
            transaction.commit()?;
        }
        Ok(())
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
        let by_id: std::collections::HashMap<&str, &Document> = docs
            .iter()
            .map(|document| (document.id.as_str(), document))
            .collect();
        let mut children_by_parent = std::collections::BTreeMap::<&str, Vec<_>>::new();
        for child in docs.iter().filter(|document| document.parent_id.is_some()) {
            let parent_id = child.parent_id.as_deref().unwrap_or_default();
            match (by_id.get(parent_id), child.page_range) {
                (Some(parent), Some(range))
                    if parent.parent_id.is_none()
                        && u32::try_from(parent.pages.len())
                            .is_ok_and(|count| range.last <= count)
                        && usize::try_from(range.last - range.first + 1)
                            .is_ok_and(|count| child.pages.len() == count)
                        && child.pages.iter().enumerate().all(|(index, page)| {
                            u32::try_from(index)
                                .ok()
                                .and_then(|offset| range.first.checked_add(offset))
                                == Some(page.page_number)
                        }) =>
                {
                    children_by_parent.entry(parent_id).or_default().push(range);
                }
                _ => issues.push(format!("{}:parent", child.id)),
            }
        }
        for (parent_id, ranges) in children_by_parent {
            let Some(parent) = by_id.get(parent_id) else {
                continue;
            };
            let mut ranges = ranges;
            ranges.sort_by_key(|range| range.first);
            if u32::try_from(parent.pages.len())
                .ok()
                .and_then(|count| crate::split::Partition::new(count, ranges).ok())
                .is_none()
            {
                issues.push(format!("{parent_id}:partition"));
            }
        }
        for doc in docs {
            match fs::read(self.blob_path(&doc.sha256)) {
                Ok(bytes) if hex(&Sha256::digest(&bytes)) == doc.sha256 => {
                    if let Some(range) = doc.page_range {
                        let expected_count = usize::try_from(range.last - range.first + 1).ok();
                        match lopdf::Document::load_mem(&bytes) {
                            Ok(pdf) if Some(pdf.get_pages().len()) == expected_count => {}
                            _ => issues.push(format!("{}:pages", doc.id)),
                        }
                    }
                }
                _ => issues.push(format!("{}:original", doc.id)),
            }
            match fs::read(self.record_path(&doc.id)) {
                Ok(bytes) if parse_sidecar(&bytes).is_ok_and(|copy| same_record(&copy, &doc)) => {}
                _ => issues.push(format!("{}:metadata", doc.id)),
            }
            let expected = ClassificationSidecar {
                version: 2,
                document_id: doc.id.clone(),
                proposals: self.proposal_records(&doc.id)?,
            };
            match fs::read(self.classification_record_path(&doc.id)) {
                Ok(bytes)
                    if serde_json::from_slice::<ClassificationSidecar>(&bytes)
                        .is_ok_and(|sidecar| sidecar == expected) => {}
                _ => issues.push(format!("{}:classification", doc.id)),
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
        for entry in fs::read_dir(self.root.join("records").join("classification"))? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
                continue;
            }
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                let id = entry
                    .path()
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !known.contains(&id) {
                    issues.push(format!("{id}:orphan-classification-sidecar"));
                }
            }
        }
        Ok(issues)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join("records").join(format!("{id}.json"))
    }

    fn classification_record_path(&self, id: &str) -> PathBuf {
        self.root
            .join("records")
            .join("classification")
            .join(format!("{id}.json"))
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
            "INSERT INTO documents_items (id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, extracted_pages_json, parent_id, page_first, page_last) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![document.id, document.sha256, document.filename, document.media_type, document.byte_len, document.imported_at,
                document.status, document.category, document.project, document.extracted_title, document.extracted_text, document.extractor,
                serde_json::to_string(&document.pages).map_err(|error| LibraryError::Invalid(error.to_string()))?, document.parent_id,
                document.page_range.map(|range| range.first), document.page_range.map(|range| range.last)],
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
            extractor TEXT,
            extracted_pages_json TEXT NOT NULL DEFAULT '[]',
            parent_id TEXT,
            page_first INTEGER,
            page_last INTEGER
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
    if !columns
        .iter()
        .any(|column| column == "extracted_pages_json")
    {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN extracted_pages_json TEXT NOT NULL DEFAULT '[]';")?;
    }
    if !columns.iter().any(|column| column == "parent_id") {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN parent_id TEXT;")?;
    }
    if !columns.iter().any(|column| column == "page_first") {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN page_first INTEGER;")?;
    }
    if !columns.iter().any(|column| column == "page_last") {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN page_last INTEGER;")?;
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS documents_items_digest_filename ON documents_items (sha256, filename);
         CREATE INDEX IF NOT EXISTS documents_items_parent_range ON documents_items (parent_id, page_first);",
    )?;
    // Where folder intake found each record. Not in the sidecars: a rescan of the
    // same folders derives it again.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents_sources (
            id TEXT NOT NULL REFERENCES documents_items(id),
            path TEXT NOT NULL,
            seen_at TEXT NOT NULL,
            PRIMARY KEY (id, path)
        );
        CREATE TABLE IF NOT EXISTS documents_duplicate_reviews (
            candidate_key TEXT PRIMARY KEY,
            decision_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS documents_classification_proposals (
            proposal_id TEXT PRIMARY KEY CHECK (length(proposal_id) = 64),
            document_id TEXT NOT NULL REFERENCES documents_items(id) ON DELETE CASCADE,
            generation_id TEXT NOT NULL,
            source_kind TEXT NOT NULL CHECK (source_kind IN ('rules', 'model')),
            source_identity TEXT NOT NULL,
            source_version TEXT NOT NULL,
            proposal_json TEXT NOT NULL,
            generated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS documents_classification_proposals_document
            ON documents_classification_proposals (document_id, generated_at, proposal_id);
        CREATE TABLE IF NOT EXISTS documents_classification_reviews (
            review_id INTEGER PRIMARY KEY AUTOINCREMENT,
            proposal_id TEXT NOT NULL REFERENCES documents_classification_proposals(proposal_id) ON DELETE CASCADE,
            outcome TEXT NOT NULL CHECK (outcome IN ('accepted', 'rejected', 'overridden')),
            reviewed_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS documents_classification_backfills (
            version TEXT PRIMARY KEY,
            high_water_rowid INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS documents_search USING fts5(
            filename, extracted_title, extracted_text, category, project,
            content='documents_items', content_rowid='rowid',
            tokenize='unicode61 remove_diacritics 2'
        );
        CREATE TABLE IF NOT EXISTS documents_search_state (
            version INTEGER NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS documents_search_after_insert
        AFTER INSERT ON documents_items BEGIN
            INSERT INTO documents_search(rowid, filename, extracted_title, extracted_text, category, project)
            VALUES (new.rowid, new.filename, new.extracted_title, new.extracted_text, new.category, new.project);
        END;
        CREATE TRIGGER IF NOT EXISTS documents_search_after_update
        AFTER UPDATE OF filename, extracted_title, extracted_text, category, project ON documents_items BEGIN
            INSERT INTO documents_search(documents_search, rowid, filename, extracted_title, extracted_text, category, project)
            VALUES ('delete', old.rowid, old.filename, old.extracted_title, old.extracted_text, old.category, old.project);
            INSERT INTO documents_search(rowid, filename, extracted_title, extracted_text, category, project)
            VALUES (new.rowid, new.filename, new.extracted_title, new.extracted_text, new.category, new.project);
        END;
        CREATE TRIGGER IF NOT EXISTS documents_search_after_delete
        AFTER DELETE ON documents_items BEGIN
            INSERT INTO documents_search(documents_search, rowid, filename, extracted_title, extracted_text, category, project)
            VALUES ('delete', old.rowid, old.filename, old.extracted_title, old.extracted_text, old.category, old.project);
        END;",
    )?;
    let search_index_version: Option<i64> = conn.query_row(
        "SELECT MAX(version) FROM documents_search_state",
        [],
        |row| row.get(0),
    )?;
    if search_index_version.unwrap_or_default() < SEARCH_INDEX_VERSION {
        conn.execute(
            "INSERT INTO documents_search(documents_search) VALUES ('rebuild')",
            [],
        )?;
        conn.execute("DELETE FROM documents_search_state", [])?;
        conn.execute(
            "INSERT INTO documents_search_state (version) VALUES (?1)",
            [SEARCH_INDEX_VERSION],
        )?;
    }
    let proposal_columns = conn
        .prepare("PRAGMA table_info(documents_classification_proposals)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !proposal_columns
        .iter()
        .any(|column| column == "generation_id")
    {
        conn.execute_batch(
            "ALTER TABLE documents_classification_proposals
             ADD COLUMN generation_id TEXT NOT NULL DEFAULT 'legacy';
             UPDATE documents_classification_proposals
             SET generation_id = proposal_id WHERE generation_id = 'legacy';",
        )?;
    }
    Ok(())
}

fn new_generation_id() -> Fallible<String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| LibraryError::Invalid(error.to_string()))?
        .as_nanos();
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(format!(
        "{timestamp:032x}-{}-{sequence:016x}",
        std::process::id()
    ))
}

fn classification_proposal_id(
    document_id: &str,
    generation_id: &str,
    proposal_json: &str,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"sjel-document-classification-proposal\0");
    hash.update(document_id.as_bytes());
    hash.update(b"\0");
    hash.update(generation_id.as_bytes());
    hash.update(b"\0");
    hash.update(proposal_json.as_bytes());
    hex(&hash.finalize())
}

fn record_id(sha256: &str, filename: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"sjel-document-record\0");
    hash.update(sha256.as_bytes());
    hash.update(b"\0");
    hash.update(filename.as_bytes());
    hex(&hash.finalize())
}

fn child_record_id(parent_id: &str, range: crate::split::PageRange, filename: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"sjel-document-child\0");
    hash.update(parent_id.as_bytes());
    hash.update(range.first.to_be_bytes());
    hash.update(range.last.to_be_bytes());
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
        && left.pages == right.pages
        && left.parent_id == right.parent_id
        && left.page_range == right.page_range
}

fn read_document(row: &Row<'_>) -> rusqlite::Result<Document> {
    let filename: String = row.get(2)?;
    let extracted_text: Option<String> = row.get(10)?;
    let page_json: String = row.get(12)?;
    let pages: Vec<sjel_extraction::ExtractedPage> =
        serde_json::from_str(&page_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let parent_id: Option<String> = row.get(13)?;
    let page_first: Option<u32> = row.get(14)?;
    let page_last: Option<u32> = row.get(15)?;
    let page_range = match (page_first, page_last) {
        (Some(first), Some(last)) => {
            Some(crate::split::PageRange::new(first, last).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    14,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error.to_string(),
                    )),
                )
            })?)
        }
        (None, None) => None,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incomplete child page range",
                )),
            ));
        }
    };
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
        pages,
        parent_id,
        page_range,
        suggested_category: None,
        suggestion_reason: None,
    })
}

fn extract(media_type: &str, bytes: &[u8]) -> Option<sjel_extraction::Extraction> {
    use sjel_extraction::{Document as Input, InputClass};
    match media_type {
        "application/pdf" => {
            let mut layer = sjel_extraction::for_class(InputClass::Pdf)
                .and_then(|rung1| rung1.extract(&Input::pdf(bytes)).ok());
            if let Some(extraction) = layer.as_mut().filter(|value| !value.pages.is_empty()) {
                let has_scanned_pages = extraction
                    .pages
                    .iter()
                    .any(|page| !has_text_layer(&page.text));
                if has_scanned_pages {
                    match sjel_extraction::vision::read_pdf(&Input::pdf(bytes)) {
                        Ok(ocr) => {
                            let recognized: Vec<_> = ocr.text.split('\u{000c}').collect();
                            if recognized.len() == extraction.pages.len() {
                                for (page, recognized_text) in
                                    extraction.pages.iter_mut().zip(recognized)
                                {
                                    if !has_text_layer(&page.text) {
                                        if has_text_layer(recognized_text) {
                                            page.text = recognized_text.trim().to_owned();
                                            page.engine = ocr.producer.to_owned();
                                            page.failure = None;
                                        } else {
                                            page.failure =
                                                Some("local OCR returned no usable text".into());
                                        }
                                    }
                                }
                            } else {
                                let message = format!(
                                    "local OCR returned {} pages for an extraction with {} pages",
                                    recognized.len(),
                                    extraction.pages.len()
                                );
                                for page in &mut extraction.pages {
                                    if !has_text_layer(&page.text) {
                                        page.failure = Some(message.clone());
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            for page in &mut extraction.pages {
                                if !has_text_layer(&page.text) {
                                    page.failure = Some(error.to_string());
                                }
                            }
                        }
                    }
                    let mut remaining = sjel_extraction::TEXT_CAP;
                    for page in &mut extraction.pages {
                        page.text = page.text.chars().take(remaining).collect();
                        remaining = remaining.saturating_sub(page.text.chars().count());
                    }
                    extraction.text = if extraction
                        .pages
                        .iter()
                        .any(|page| !page.text.trim().is_empty())
                    {
                        sjel_extraction::cap(
                            extraction
                                .pages
                                .iter()
                                .map(|page| page.text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n\u{000c}\n\n"),
                        )
                    } else {
                        String::new()
                    };
                }
                return layer;
            }
            if layer.as_ref().is_some_and(|e| has_text_layer(&e.text)) {
                return layer;
            }
            // No text layer: a scan. Rung 2 reads every page; if it is absent
            // or fails, keep whatever rung 1 returned.
            sjel_extraction::ladder::read_scanned(&Input::pdf(bytes), None)
                .ok()
                .or(layer)
        }
        "text/plain" => sjel_extraction::for_class(InputClass::PlainText)?
            .extract(&Input::text(bytes))
            .ok(),
        _ => None,
    }
}

/// Fewer than 64 non-space characters means the PDF is a scan. A scanner's
/// stamp or page number survives as a few characters; a real text layer does not
/// fit under this. My estimate, not measured.
// ponytail: whole-document threshold; a mixed PDF with one text page and many
// scanned pages keeps only the text page. Decide per page when that shows up.
fn has_text_layer(text: &str) -> bool {
    text.chars().filter(|c| !c.is_whitespace()).count() >= 64
}

#[cfg(test)]
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

fn search_terms(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "bei", "das", "dem", "den", "der", "des", "die", "ein", "eine", "einer",
        "eines", "for", "from", "in", "is", "mit", "of", "on", "or", "the", "to", "und", "von",
        "wegen", "with", "zu",
    ];
    let mut terms = Vec::new();
    for term in query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|term| term.chars().count() > 1)
    {
        let folded = term.to_lowercase();
        if !STOP_WORDS.contains(&folded.as_str()) && !terms.contains(&folded) {
            terms.push(folded);
        }
    }
    terms
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

pub(crate) fn hex(bytes: &[u8]) -> String {
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
        scratch_with_config(crate::classification::ClassificationConfig::default())
    }

    fn scratch_with_config(
        classification: crate::classification::ClassificationConfig,
    ) -> (Library, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sjel-documents-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        (
            Library::open_with_config(&path.join("test.db"), path.join("library"), classification)
                .unwrap(),
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
    fn indexed_search_tracks_filename_text_and_filing_metadata() {
        let (library, root) = scratch();
        let Stored::New(document) = library
            .intake("bahn-gutschein.pdf", b"synthetic PDF placeholder")
            .unwrap()
        else {
            panic!("new document expected")
        };

        let filename_hits = library.search("Gutschein", None, 0, 10).unwrap();
        assert_eq!(filename_hits.documents[0].id, document.id);

        let conn = library.pool.get().unwrap();
        conn.execute(
            "UPDATE documents_items SET extracted_text = ?2 WHERE id = ?1",
            params![document.id, "Fahrgastrechte Verspätungsentschädigung"],
        )
        .unwrap();
        drop(conn);
        let text_hits = library.search("Fahrgastrechte", None, 0, 10).unwrap();
        assert_eq!(text_hits.documents[0].id, document.id);
        let delay_hits = library.search("Verspätung", None, 0, 10).unwrap();
        assert_eq!(delay_hits.documents[0].id, document.id);

        library
            .review(
                &document.id,
                &Review {
                    category: "Travel".into(),
                    project: Some("Rail".into()),
                },
            )
            .unwrap();
        let filed_hits = library.search("Rail", Some("filed"), 0, 10).unwrap();
        assert_eq!(filed_hits.documents[0].id, document.id);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_rules_replace_suggestions_without_restricting_filing() {
        let classification = crate::classification::ClassificationConfig::from_json(
            r#"{
                "version": "travel-v1",
                "categories": ["Travel"],
                "rules": [{"id": "travel-keywords", "category": "Travel", "keywords": ["ticket"]}]
            }"#,
        )
        .unwrap();
        let (library, root) = scratch_with_config(classification);
        let Stored::New(document) = library
            .intake("invoice.txt", b"Synthetic ticket for a train")
            .unwrap()
        else {
            panic!("new document expected")
        };

        assert_eq!(document.suggested_category.as_deref(), Some("Travel"));
        assert_eq!(document.status, "inbox");
        assert_eq!(
            library.list(None).unwrap()[0].suggested_category.as_deref(),
            Some("Travel")
        );
        let proposals = library.proposals(&document);
        assert!(matches!(
            &proposals[0].outcome,
            crate::classification::ProposalOutcome::Proposed { category, evidence }
                if category == "Travel"
                    && evidence.source == crate::classification::EvidenceSource::ExtractedText
                    && evidence.quote == "ticket"
        ));

        let filed = library
            .review(
                &document.id,
                &Review {
                    category: "Legacy filing label".into(),
                    project: None,
                },
            )
            .unwrap();
        assert_eq!(filed.category.as_deref(), Some("Legacy filing label"));
        assert_eq!(filed.suggested_category.as_deref(), Some("Travel"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn proposal_versions_and_review_events_are_append_only_across_reopen() {
        let (library, root) = scratch();
        let Stored::New(document) = library.intake("invoice.txt", b"invoice 42").unwrap() else {
            panic!("new document expected")
        };
        let original = library.proposal_records(&document.id).unwrap().remove(0);
        library
            .review_proposal(
                &document.id,
                &original.id,
                crate::classification::ProposalReview::Accepted,
            )
            .unwrap();
        drop(library);

        let replacement = crate::classification::ClassificationConfig::from_json(
            r#"{"version":"private-v2","categories":["Tax"],"rules":[{"id":"tax-invoice","category":"Tax","keywords":["invoice"]}]}"#,
        )
        .unwrap();
        let library = Library::open_with_config(
            &root.join("test.db"),
            root.join("library"),
            replacement.clone(),
        )
        .unwrap();
        let records = library.proposal_records(&document.id).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].review_history.len(), 1);
        assert_eq!(
            records[0].proposal.review,
            Some(crate::classification::ProposalReview::Accepted)
        );
        assert_eq!(records[1].review_history.len(), 0);
        drop(library);

        let library =
            Library::open_with_config(&root.join("test.db"), root.join("library"), replacement)
                .unwrap();
        assert_eq!(library.proposal_records(&document.id).unwrap().len(), 2);
        let regenerated_first = library
            .regenerate_rule_proposals(&document.id)
            .unwrap()
            .remove(0);
        let regenerated_second = library
            .regenerate_rule_proposals(&document.id)
            .unwrap()
            .remove(0);
        assert_ne!(regenerated_first.id, regenerated_second.id);
        assert_ne!(
            regenerated_first.generation_id,
            regenerated_second.generation_id
        );
        assert_eq!(library.proposal_records(&document.id).unwrap().len(), 4);
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
        let proposal = library.proposal_records(&document.id).unwrap().remove(0);
        library
            .review_proposal(
                &document.id,
                &proposal.id,
                crate::classification::ProposalReview::Accepted,
            )
            .unwrap();
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
        let restored_proposals = library.proposal_records(&document.id).unwrap();
        assert_eq!(restored_proposals.len(), 1);
        assert_eq!(restored_proposals[0].review_history.len(), 1);
        assert_eq!(
            restored_proposals[0].proposal.review,
            Some(crate::classification::ProposalReview::Accepted)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_path_components_and_bad_digests() {
        let (library, root) = scratch();
        assert!(library.intake("../private.pdf", b"x").is_err());
        assert!(library.get("../bad").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[derive(serde::Deserialize)]
    struct SyntheticSample {
        id: String,
        language: String,
        expected: String,
        text: String,
    }

    fn rule_metrics(corpus: &str) -> serde_json::Value {
        let samples: Vec<SyntheticSample> = serde_json::from_str(corpus).unwrap();
        let mut proposed = 0usize;
        let mut correct = 0usize;
        let mut wrong = 0usize;
        let mut by_category = std::collections::BTreeMap::<String, [usize; 3]>::new();
        let mut by_language = std::collections::BTreeMap::<String, [usize; 3]>::new();
        let mut predictions = Vec::with_capacity(samples.len());
        for sample in &samples {
            let (category, _) = suggest_category("synthetic.txt", Some(&sample.text));
            let is_correct = category.as_deref() == Some(sample.expected.as_str());
            let category_metrics = by_category.entry(sample.expected.clone()).or_default();
            let language_metrics = by_language.entry(sample.language.clone()).or_default();
            if category.is_some() {
                proposed += 1;
                category_metrics[0] += 1;
                language_metrics[0] += 1;
                if is_correct {
                    correct += 1;
                    category_metrics[1] += 1;
                    language_metrics[1] += 1;
                } else {
                    wrong += 1;
                    category_metrics[2] += 1;
                    language_metrics[2] += 1;
                }
            }
            predictions.push(serde_json::json!({
                "id": sample.id,
                "language": sample.language,
                "expected": sample.expected,
                "predicted": category,
                "correct": is_correct,
            }));
        }
        let summarize = |metrics: std::collections::BTreeMap<String, [usize; 3]>| {
            metrics
                .into_iter()
                .map(|(key, counts)| {
                    (
                        key,
                        serde_json::json!({
                            "proposed": counts[0],
                            "correct": counts[1],
                            "wrong": counts[2],
                        }),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
        };
        serde_json::json!({
            "samples": samples.len(),
            "proposed": proposed,
            "correct": correct,
            "wrong": wrong,
            "unresolved": samples.len() - proposed,
            "precision": if proposed == 0 { 0.0 } else { correct as f64 / proposed as f64 },
            "by_expected_category": summarize(by_category),
            "by_language": summarize(by_language),
            "predictions": predictions,
        })
    }

    #[test]
    fn current_keyword_rules_have_frozen_synthetic_baselines() {
        let development = rule_metrics(include_str!("../eval/clm-corpus-v2.json"));
        let holdout = rule_metrics(include_str!("../eval/clm-corpus-holdout-v1.json"));
        let hybrid = rule_metrics(include_str!("../eval/clm-corpus-hybrid-v1.json"));
        let combined_holdout =
            rule_metrics(include_str!("../eval/clm-corpus-combined-holdout-v1.json"));
        println!("RULE_BASELINE_V2={development}");
        println!("RULE_BASELINE_HOLDOUT_V1={holdout}");
        println!("RULE_BASELINE_HYBRID_V1={hybrid}");
        println!("RULE_BASELINE_COMBINED_HOLDOUT_V1={combined_holdout}");
        assert_eq!(development["samples"], 48);
        assert_eq!(development["proposed"], 16);
        assert_eq!(development["correct"], 14);
        assert_eq!(development["wrong"], 2);
        assert_eq!(development["by_expected_category"]["Events"]["correct"], 2);
        assert_eq!(holdout["samples"], 48);
        assert_eq!(holdout["proposed"], 10);
        assert_eq!(holdout["correct"], 9);
        assert_eq!(holdout["wrong"], 1);
        assert_eq!(holdout["by_expected_category"]["Events"]["correct"], 0);
        assert_eq!(hybrid["samples"], 48);
        assert_eq!(hybrid["proposed"], 11);
        assert_eq!(hybrid["correct"], 9);
        assert_eq!(hybrid["wrong"], 2);
        assert_eq!(hybrid["by_expected_category"]["Events"]["correct"], 0);
        assert_eq!(combined_holdout["samples"], 48);
        assert_eq!(combined_holdout["proposed"], 13);
        assert_eq!(combined_holdout["correct"], 11);
        assert_eq!(combined_holdout["wrong"], 2);
        assert_eq!(
            combined_holdout["by_expected_category"]["Events"]["correct"],
            0
        );
    }
}

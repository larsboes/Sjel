//! The document list without extracted text.
//!
//! The list shows a character count, a page count and the workflow stage. Deriving them needs the
//! full text and page evidence, so each record caches them in `derived_json`. A change to the
//! extraction clears the cache; a list or the background pass fills it again.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{
    check_status, params, params_from_iter, read_document, sql, Connection, Document, Fallible,
    Library, LibraryError, DOCUMENT_ID_BATCH,
};

/// Bump when [`Derived::of`] changes, so every cached value is derived again.
const DERIVED_VERSION: u32 = 1;

/// The light columns: extracted text and page evidence are replaced by empty values.
const LIGHT_COLUMNS: &str = "id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, NULL, extractor, '[]', parent_id, page_first, page_last, derived_json";

/// Values the list shows that depend on the extracted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derived {
    version: u32,
    pub extracted_chars: usize,
    pub page_count: usize,
    pub analysis_gap: bool,
    pub split_proposed: bool,
}

impl Derived {
    pub fn of(document: &Document) -> Self {
        Self {
            version: DERIVED_VERSION,
            extracted_chars: document.extracted_text.as_deref().map_or(0, |text| {
                text.chars().filter(|c| !c.is_whitespace()).count()
            }),
            page_count: document.pages.len(),
            analysis_gap: crate::workflow::has_analysis_gap(document),
            split_proposed: document.media_type == "application/pdf"
                && crate::split::suggest_ranges(&document.pages).ranges.len() > 1,
        }
    }

    fn parse(json: Option<&str>) -> Option<Self> {
        json.and_then(|json| serde_json::from_str::<Self>(json).ok())
            .filter(|derived| derived.version == DERIVED_VERSION)
    }
}

/// The cached page count, when the cache is current.
pub(super) fn page_count(derived_json: Option<&str>) -> Option<usize> {
    Derived::parse(derived_json).map(|derived| derived.page_count)
}

/// One list row: metadata without extracted text, and its derived values.
pub struct ListRow {
    pub document: Document,
    pub derived: Derived,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let columns = conn
        .prepare("PRAGMA table_info(documents_items)")?
        .query_map(params![], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "derived_json") {
        conn.execute_batch("ALTER TABLE documents_items ADD COLUMN derived_json TEXT;")?;
    }
    Ok(())
}

fn read_light(row: &sql::Row<'_>) -> sql::Result<(Document, Option<String>)> {
    Ok((read_document(row)?, row.get(16)?))
}

impl Library {
    /// One page of the list, newest first, without reading extracted text for cached rows.
    ///
    /// # Errors
    /// The status is unknown or the store cannot be read or updated.
    pub fn list_rows(
        &self,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Fallible<(Vec<ListRow>, usize)> {
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
        let (total, rows): (i64, Vec<_>) = if let Some(status) = status {
            let total = conn.query_row(
                "SELECT COUNT(*) FROM documents_items WHERE status = ?1",
                params![status],
                |row| row.get(0),
            )?;
            let sql = format!("SELECT {LIGHT_COLUMNS} FROM documents_items WHERE status = ?1 ORDER BY imported_at DESC, filename LIMIT ?2 OFFSET ?3");
            let mut statement = conn.prepare(&sql)?;
            let rows = statement
                .query_map(params![status, limit, offset], read_light)?
                .collect::<sql::Result<Vec<_>>>()?;
            (total, rows)
        } else {
            let total =
                conn.query_row("SELECT COUNT(*) FROM documents_items", params![], |row| {
                    row.get(0)
                })?;
            let sql = format!("SELECT {LIGHT_COLUMNS} FROM documents_items ORDER BY imported_at DESC, filename LIMIT ?1 OFFSET ?2");
            let mut statement = conn.prepare(&sql)?;
            let rows = statement
                .query_map(params![limit, offset], read_light)?
                .collect::<sql::Result<Vec<_>>>()?;
            (total, rows)
        };
        drop(conn);
        let total = usize::try_from(total)
            .map_err(|_| LibraryError::Invalid("document count is out of range".into()))?;
        let ids: Vec<String> = rows
            .iter()
            .map(|(document, _)| document.id.clone())
            .collect();
        let categories = self.stored_rule_categories(&ids)?;
        let mut list = Vec::with_capacity(rows.len());
        for (mut document, derived) in rows {
            let derived = match Derived::parse(derived.as_deref()) {
                Some(derived) => derived,
                None => self.derive_and_store(&document.id)?,
            };
            document.suggested_category = categories.get(&document.id).cloned();
            document.suggestion_reason = document
                .suggested_category
                .as_ref()
                .map(|_| "a configured local rule matched; review before filing".into());
            list.push(ListRow { document, derived });
        }
        Ok((list, total))
    }

    /// Derive and cache the list values for one record from its full extraction.
    fn derive_and_store(&self, id: &str) -> Fallible<Derived> {
        let document = self
            .get(id)?
            .ok_or_else(|| LibraryError::Invalid("listed record disappeared".into()))?;
        let derived = Derived::of(&document);
        let json =
            serde_json::to_string(&derived).map_err(|e| LibraryError::Invalid(e.to_string()))?;
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        conn.execute(
            "UPDATE documents_items SET derived_json = ?2 WHERE id = ?1",
            params![id, json.as_str()],
        )?;
        Ok(derived)
    }

    /// Cache derived values for up to `limit` records that have none. Returns how many it
    /// derived; zero means the cache is complete.
    ///
    /// # Errors
    /// The store cannot be read or updated.
    pub fn backfill_derived(&self, limit: usize) -> Fallible<usize> {
        let stale: Vec<String> = {
            let conn = self
                .pool
                .get()
                .map_err(|e| LibraryError::Store(e.to_string()))?;
            let mut statement = conn.prepare("SELECT id, derived_json FROM documents_items")?;
            let rows = statement
                .query_map(params![], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .collect::<sql::Result<Vec<_>>>()?;
            rows.into_iter()
                .filter(|(_, derived)| Derived::parse(derived.as_deref()).is_none())
                .map(|(id, _)| id)
                .take(limit)
                .collect()
        };
        for id in &stale {
            self.derive_and_store(id)?;
        }
        Ok(stale.len())
    }

    /// The first proposed category of each record's newest stored rule run.
    fn stored_rule_categories(&self, ids: &[String]) -> Fallible<HashMap<String, String>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut categories = HashMap::new();
        let mut seen = HashSet::new();
        for batch in ids.chunks(DOCUMENT_ID_BATCH) {
            let placeholders = vec!["?"; batch.len()].join(", ");
            let sql = format!(
                "SELECT p.document_id, p.proposal_json
                 FROM documents_classification_proposals p
                 WHERE p.source_kind = 'rules' AND p.document_id IN ({placeholders})
                   AND p.generation_id = (
                     SELECT newest.generation_id
                     FROM documents_classification_proposals newest
                     WHERE newest.document_id = p.document_id AND newest.source_kind = 'rules'
                     ORDER BY newest.generated_at DESC, newest.generation_id DESC
                     LIMIT 1
                   )
                 ORDER BY p.document_id, p.proposal_id"
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement
                .query_map(params_from_iter(batch.iter().map(String::as_str)), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<sql::Result<Vec<_>>>()?;
            for (document_id, json) in rows {
                if seen.contains(&document_id) {
                    continue;
                }
                let proposal: crate::classification::ClassificationProposal =
                    serde_json::from_str(&json).map_err(|error| {
                        LibraryError::Invalid(format!("stored classification proposal: {error}"))
                    })?;
                if let crate::classification::ProposalOutcome::Proposed { category, .. } =
                    proposal.outcome
                {
                    seen.insert(document_id.clone());
                    categories.insert(document_id, category);
                }
            }
        }
        Ok(categories)
    }
}

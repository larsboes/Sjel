//! Duplicate candidates computed once per library revision.
//!
//! Comparing extracted text across the library is the slowest read the owner serves. Every
//! write that can change a candidate bumps the revision; the next read computes again, and a
//! read at an unchanged revision returns the cached candidates.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::{params, read_document, sql, Fallible, Library, LibraryError};
use crate::review::ReviewCandidate;

/// Active records with extracted text but without page evidence, which the comparison never reads.
const INPUT_SQL: &str = "SELECT id, sha256, filename, media_type, byte_len, imported_at, status, category, project, extracted_title, extracted_text, extractor, '[]', parent_id, page_first, page_last, derived_json FROM documents_items WHERE status != 'archived'";

pub(super) struct StaleAfter<'a>(&'a std::sync::atomic::AtomicU64);

impl Drop for StaleAfter<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub(super) struct DuplicateCache {
    computed: Option<(u64, Arc<Vec<ReviewCandidate>>)>,
}

impl Library {
    /// Marks cached candidates stale when the returned guard drops, after the write finished
    /// on every return path.
    pub(super) fn stale_after(&self) -> StaleAfter<'_> {
        StaleAfter(&self.revision)
    }

    /// Candidates among active records, computed at most once per revision.
    ///
    /// # Errors
    /// The store cannot be read.
    pub fn duplicate_candidates(&self) -> Fallible<Arc<Vec<ReviewCandidate>>> {
        let mut cache = self
            .duplicate_cache
            .lock()
            .map_err(|_| LibraryError::Store("duplicate cache lock poisoned".into()))?;
        let revision = self.revision.load(Ordering::SeqCst);
        if let Some((cached, candidates)) = &cache.computed {
            if *cached == revision {
                return Ok(Arc::clone(candidates));
            }
        }
        let (documents, page_counts) = self.duplicate_inputs()?;
        let ids: std::collections::HashSet<&str> = documents
            .iter()
            .map(|document| document.id.as_str())
            .collect();
        let sources: Vec<_> = self
            .sources()?
            .into_iter()
            .filter(|(id, _)| ids.contains(id.as_str()))
            .collect();
        let candidates = Arc::new(crate::duplicates::candidates_counted(
            &documents,
            &sources,
            &page_counts,
        ));
        cache.computed = Some((revision, Arc::clone(&candidates)));
        Ok(candidates)
    }

    fn duplicate_inputs(&self) -> Fallible<(Vec<super::Document>, HashMap<String, usize>)> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut statement = conn.prepare(INPUT_SQL)?;
        let rows = statement
            .query_map(params![], |row| {
                Ok((read_document(row)?, row.get::<_, Option<String>>(16)?))
            })?
            .collect::<sql::Result<Vec<_>>>()?;
        let mut page_counts = HashMap::with_capacity(rows.len());
        let mut documents = Vec::with_capacity(rows.len());
        for (document, derived) in rows {
            let pages = document.page_range.map_or_else(
                || super::listing::page_count(derived.as_deref()).unwrap_or(0),
                |range| (range.last - range.first + 1) as usize,
            );
            page_counts.insert(document.id.clone(), pages);
            documents.push(document);
        }
        Ok((documents, page_counts))
    }
}

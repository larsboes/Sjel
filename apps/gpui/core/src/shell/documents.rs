//! Documents shell client methods.

use super::*;

impl Shell {
    /// Read soundscape state (`GET /soundscape/api/soundscape/state`).
    /// Stream document metadata in batches through `tx` (typically 500 records at a time).
    pub async fn documents_stream(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        self.documents_query_stream(None, tx).await
    }

    /// Stream ranked metadata and matching page numbers through the local Documents API.
    pub async fn documents_search_stream(
        &self,
        query: &str,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        self.documents_query_stream(Some(query), tx).await
    }

    async fn documents_query_stream(
        &self,
        query: Option<&str>,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        let mut loaded = 0;
        loop {
            let path = documents_path(loaded, query);
            let page: DocumentPage = self.json(Method::GET, &path, None, 10).await?;
            let page_len = page.documents.len();
            let total = page.total;
            loaded += page_len;
            let _ = tx.send(page.documents);
            if loaded >= total || page_len == 0 {
                return Ok(loaded);
            }
        }
    }

    /// Documents metadata through the authenticated local capability proxy. Text is only read
    /// for a single document through `document_detail`.
    pub async fn documents(&self) -> ShellResult<Vec<DocumentSummary>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let stream = self.documents_stream(tx);
        let mut rows = Vec::new();
        let (res, _) = futures::join!(stream, async {
            while let Some(batch) = rx.recv().await {
                rows.extend(batch);
            }
        });
        res?;
        Ok(rows)
    }

    /// Read one page of unresolved duplicate candidates from the local operator queue.
    pub async fn duplicate_candidates(
        &self,
        kind: Option<DuplicateCandidateKind>,
        offset: usize,
        limit: usize,
    ) -> ShellResult<DuplicateCandidatePage> {
        if !(1..=100).contains(&limit) {
            return Err(ShellError::Refused(
                400,
                Some("Candidate page limit must be 1..100.".into()),
            ));
        }
        let kind = kind.map(|kind| match kind {
            DuplicateCandidateKind::ExactCopy => "exact_copy",
            DuplicateCandidateKind::Rescan => "rescan",
            DuplicateCandidateKind::Part => "part",
        });
        let kind = kind.map_or_else(String::new, |kind| format!("&kind={kind}"));
        let path =
            format!("/documents/api/documents/duplicates?limit={limit}&offset={offset}{kind}");
        self.json(Method::GET, &path, None, 30).await
    }

    /// Save an explicit human decision for a candidate without changing source files.
    pub async fn review_duplicate(&self, decision: DuplicateDecision) -> ShellResult<()> {
        let body = serde_json::to_value(decision).map_err(|_| ShellError::Unreadable)?;
        let response: DuplicateDecisionResponse = self
            .json(
                Method::POST,
                "/documents/api/documents/duplicates/review",
                Some(body),
                30,
            )
            .await?;
        let _ = response.decision;
        Ok(())
    }

    pub async fn document_detail(&self, id: &str) -> ShellResult<DocumentDetail> {
        let Some(path) = document_path(id) else {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        };
        self.json(Method::GET, &path, None, 10).await
    }

    /// Re-run local extraction and read the updated page evidence.
    pub async fn analyze_document(&self, id: &str) -> ShellResult<DocumentDetail> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let _: serde_json::Value = self
            .json(
                Method::POST,
                &format!("/documents/api/documents/{id}/analyze"),
                Some(serde_json::json!({})),
                120,
            )
            .await?;
        self.document_detail(id).await
    }

    /// Create child records for a complete, operator-approved PDF partition.
    pub async fn split_document(
        &self,
        id: &str,
        request: &DocumentSplitRequest,
    ) -> ShellResult<Vec<DocumentSummary>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let body = serde_json::to_value(request).map_err(|_| ShellError::Unreadable)?;
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/split"),
            Some(body),
            30,
        )
        .await
    }

    /// Read the metadata-only child records of one source document.
    pub async fn document_children(&self, id: &str) -> ShellResult<Vec<DocumentSummary>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::GET,
            &format!("/documents/api/documents/{id}/children"),
            None,
            10,
        )
        .await
    }

    /// Append a classification review outcome. This never files the document.
    pub async fn review_document_proposal(
        &self,
        id: &str,
        proposal_id: &str,
        outcome: ProposalReview,
    ) -> ShellResult<DocumentProposalRecord> {
        if !is_sha256(id) || !is_sha256(proposal_id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a document proposal id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/{proposal_id}/review"),
            Some(serde_json::json!({"outcome": outcome})),
            10,
        )
        .await
    }

    /// Regenerate local deterministic rule proposals and append a new run.
    pub async fn regenerate_document_proposals(
        &self,
        id: &str,
    ) -> ShellResult<Vec<DocumentProposalRecord>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/regenerate"),
            Some(serde_json::json!({})),
            10,
        )
        .await
    }

    pub async fn generate_document_model_proposal(
        &self,
        id: &str,
    ) -> ShellResult<Vec<DocumentProposalRecord>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/model"),
            Some(serde_json::json!({})),
            55,
        )
        .await
    }

    pub async fn review_document(
        &self,
        id: &str,
        category: &str,
        project: Option<&str>,
    ) -> ShellResult<DocumentSummary> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let body = serde_json::json!({"category": category, "project": project});
        self.json(
            Method::PATCH,
            &format!("/documents/api/documents/{id}"),
            Some(body),
            10,
        )
        .await
    }

    pub async fn archive_document(&self, id: &str) -> ShellResult<DocumentSummary> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::DELETE,
            &format!("/documents/api/documents/{id}"),
            None,
            10,
        )
        .await
    }

    /// The hash-verified original bytes (`GET /api/documents/{id}/original`).
    ///
    /// A successful read is the original and not a substitute: the capability checks the digest
    /// before it answers. The read is bounded by [`MAX_DOCUMENT_BYTES`] whether or not the response
    /// declares a length, because a reader that draws pages has to know the most it can be handed.
    pub async fn document_original(&self, id: &str) -> ShellResult<Vec<u8>> {
        let path = document_original_path(id)
            .ok_or_else(|| ShellError::Refused(400, Some("Not a document id.".into())))?;
        let mut response = self.send(Method::GET, &path, None, 30).await?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut original = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ShellError::Unreadable)? {
            if original.len() + chunk.len() > MAX_DOCUMENT_BYTES {
                return Err(too_large());
            }
            original.extend_from_slice(&chunk);
        }
        Ok(original)
    }

    /// Upload one file as raw bytes (`POST /api/documents?filename=...`), idempotent by bytes and
    /// filename. The capability refuses names that suggest confidential material before it reads
    /// the body, and that refusal arrives here as `ShellError::Refused(403, ...)`. Repeating an
    /// upload keeps the record and its filing decision, so `created` is false the second time.
    pub async fn upload_document(
        &self,
        filename: &str,
        bytes: Vec<u8>,
    ) -> ShellResult<DocumentUpload> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(too_large());
        }
        let path = with_query("/documents/api/documents", &[("filename", filename)]);
        let response = self
            .send_payload(Method::POST, &path, Some(Payload::Bytes(bytes)), 60)
            .await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }
}

/// One page of up to 500 metadata rows from `offset`, ranked by `query` when there is one.
fn documents_path(offset: usize, query: Option<&str>) -> String {
    let offset = offset.to_string();
    let mut pairs = vec![("limit", "500"), ("offset", offset.as_str())];
    if let Some(query) = query {
        pairs.push(("q", query));
    }
    with_query("/documents/api/documents", &pairs)
}

/// The detail path of a document, or `None` when `id` is not a SHA-256 in hex.
fn document_path(id: &str) -> Option<String> {
    (id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("/documents/api/documents/{id}"))
}

/// One page of document metadata through a paired phone's signed client.
///
/// # Errors
/// The client's reason when the Mac refuses or cannot be reached.
pub async fn device_documents_page(
    client: &crate::device::DeviceClient,
    offset: usize,
) -> Result<DocumentPage, String> {
    client.get(&documents_path(offset, None)).await
}

/// A document's extracted text and pages through a paired phone's signed client.
///
/// # Errors
/// A refusal for an `id` that is not a document id, or the client's reason.
pub async fn device_document_detail(
    client: &crate::device::DeviceClient,
    id: &str,
) -> Result<DocumentDetail, String> {
    let path = document_path(id).ok_or_else(|| "Not a document id.".to_string())?;
    client.get(&path).await
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn paths_carry_paging_query_and_a_checked_id() {
        assert_eq!(
            documents_path(500, Some("tax 2025")),
            "/documents/api/documents?limit=500&offset=500&q=tax+2025"
        );
        assert_eq!(
            documents_path(0, None),
            "/documents/api/documents?limit=500&offset=0"
        );
        let id = "a".repeat(64);
        assert_eq!(
            document_path(&id),
            Some(format!("/documents/api/documents/{id}"))
        );
        assert_eq!(document_path("../etc"), None);
    }
}

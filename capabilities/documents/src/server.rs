//! Authenticated operator UI over the document library. No agent-token admission.

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    classification::{ProposalReviewRequest, StoredProposal},
    library::MAX_BYTES,
    review::{CandidateKind, DecisionAction, DuplicateDecision},
    Document, Library, LibraryError, Review, Stored,
};

const DEFAULT_PORT: u16 = 8104;

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Check document service liveness."),
    route_manifest::get("GET", "/routes", "Describe the operator-only document API. Agent credentials are not admitted."),
    route_manifest::get("GET", "/api/documents", "List metadata only. Optional q searches local text; status filters inbox/filed/archived; sha256 returns the records holding those bytes under any filename; limit (1..500) and offset paginate."),
    route_manifest::get("GET", "/api/documents/{id}", "Read metadata, locally extracted text, and independent local rule proposals in the authenticated operator UI."),
    route_manifest::get("GET", "/api/documents/{id}/original", "Read hash-verified original bytes. download=true forces attachment disposition."),
    route_manifest::get("GET", "/api/documents/duplicates", "Read paginated exact-copy and advisory duplicate candidates without extracted text."),
    route_manifest::Route {
        method: "POST", path: "/api/documents/duplicates/review",
        request_schema: Some(route_manifest::schema_of::<DuplicateDecision>),
        summary: "Persist an operator decision for a current duplicate candidate. Originals and records are retained.",
    },
    route_manifest::Route {
        method: "POST", path: "/api/documents/{id}/proposals/{proposal_id}/review",
        request_schema: Some(route_manifest::schema_of::<ProposalReviewRequest>),
        summary: "Append an operator review outcome to one classification proposal. This does not file the document.",
    },
    route_manifest::Route {
        method: "POST", path: "/api/documents/{id}/proposals/regenerate",
        request_schema: Some(route_manifest::schema_of::<RegenerateProposalRequest>),
        summary: "Generate a new deterministic rule-proposal run. Every run is stored separately and does not file the document.",
    },
    route_manifest::Route {
        method: "POST", path: "/api/documents", request_schema: Some(binary_schema),
        summary: "Upload raw original bytes with required filename query parameter. Sensitive names are refused before reading the body. Maximum 128 MiB; idempotent by bytes and filename. Returns local rule proposals separately from filing metadata.",
    },
    route_manifest::Route {
        method: "PATCH", path: "/api/documents/{id}",
        request_schema: Some(route_manifest::schema_of::<Review>),
        summary: "File with an operator-selected category and optional project reference. Original bytes are unchanged.",
    },
    route_manifest::get("DELETE", "/api/documents/{id}", "Archive a document record; retain its original and metadata. Refile to restore."),
];

fn binary_schema() -> Value {
    json!({"type": "string", "format": "binary", "maxLength": MAX_BYTES})
}

#[derive(Serialize)]
struct Summary {
    id: String,
    filename: String,
    media_type: String,
    byte_len: i64,
    imported_at: String,
    status: String,
    category: Option<String>,
    project: Option<String>,
    extracted_title: Option<String>,
    extractor: Option<String>,
    extracted_chars: usize,
    suggested_category: Option<String>,
    suggestion_reason: Option<String>,
    data_class: &'static str,
}

impl From<&Document> for Summary {
    fn from(document: &Document) -> Self {
        Self {
            id: document.id.clone(),
            filename: document.filename.clone(),
            media_type: document.media_type.clone(),
            byte_len: document.byte_len,
            imported_at: document.imported_at.clone(),
            status: document.status.clone(),
            category: document.category.clone(),
            project: document.project.clone(),
            extracted_title: document.extracted_title.clone(),
            extractor: document.extractor.clone(),
            extracted_chars: document
                .extracted_text
                .as_deref()
                .map_or(0, |s| s.chars().count()),
            suggested_category: document.suggested_category.clone(),
            suggestion_reason: document.suggestion_reason.clone(),
            data_class: "c2",
        }
    }
}

struct ApiError(StatusCode, String);

impl From<LibraryError> for ApiError {
    fn from(error: LibraryError) -> Self {
        match error {
            LibraryError::Invalid(message) => Self(StatusCode::BAD_REQUEST, message),
            _ => Self(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Document storage could not be read or updated.".into(),
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

async fn blocking<T: Send + 'static>(
    library: Arc<Library>,
    work: impl FnOnce(&Library) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || work(&library))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Document worker failed.".into(),
            )
        })?
}

fn require_document(library: &Library, id: &str) -> Result<Document, ApiError> {
    library
        .get(id)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Document not found.".into()))
}

#[derive(Default, Deserialize)]
struct ListQuery {
    q: Option<String>,
    status: Option<String>,
    sha256: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn list(
    State(library): State<Arc<Library>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(200);
    if !(1..=500).contains(&limit) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "limit must be 1..500".into(),
        ));
    }
    let offset = query.offset.unwrap_or(0);
    blocking(library, move |library| {
        let documents = match &query.sha256 {
            Some(sha256) => library.by_hash(sha256)?,
            None => library.list(query.status.as_deref())?,
        };
        let q = query.q.unwrap_or_default().trim().to_lowercase();
        let matched: Vec<_> = documents
            .iter()
            .filter(|d| {
                q.is_empty()
                    || d.filename.to_lowercase().contains(&q)
                    || d.extracted_title
                        .as_deref()
                        .is_some_and(|s| s.to_lowercase().contains(&q))
                    || d.extracted_text
                        .as_deref()
                        .is_some_and(|s| s.to_lowercase().contains(&q))
            })
            .collect();
        let rows: Vec<_> = matched
            .iter()
            .skip(offset)
            .take(limit)
            .map(|d| Summary::from(*d))
            .collect();
        Ok(Json(
            json!({"documents": rows, "total": matched.len(), "offset": offset, "limit": limit}),
        ))
    })
    .await
}

#[derive(Default, Deserialize)]
struct DuplicateQuery {
    kind: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn duplicate_candidates(
    State(library): State<Arc<Library>>,
    Query(query): Query<DuplicateQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "limit must be 1..100".into(),
        ));
    }
    let kind = match query.kind.as_deref() {
        None => None,
        Some("exact_copy") => Some(CandidateKind::ExactCopy),
        Some("rescan") => Some(CandidateKind::Rescan),
        Some("part") => Some(CandidateKind::Part),
        Some(_) => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "kind must be exact_copy, rescan or part".into(),
            ))
        }
    };
    let offset = query.offset.unwrap_or(0);
    blocking(library, move |library| {
        let documents = library.list(None)?;
        let sources = library.sources()?;
        let reviewed: std::collections::HashSet<_> = library
            .duplicate_decisions()?
            .into_iter()
            .map(|decision| decision.candidate_key)
            .collect();
        let candidates: Vec<_> = crate::duplicates::candidates(&documents, &sources)
            .into_iter()
            .filter(|candidate| !reviewed.contains(&candidate.candidate_key))
            .filter(|candidate| kind.is_none_or(|kind| candidate.kind == kind))
            .collect();
        let total = candidates.len();
        let page: Vec<_> = candidates.into_iter().skip(offset).take(limit).collect();
        Ok(Json(
            json!({"candidates": page, "total": total, "offset": offset, "limit": limit}),
        ))
    })
    .await
}

async fn review_duplicate(
    State(library): State<Arc<Library>>,
    Json(decision): Json<DuplicateDecision>,
) -> Result<Json<Value>, ApiError> {
    decision
        .validate()
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    blocking(library, move |library| {
        let documents = library.list(None)?;
        let sources = library.sources()?;
        let candidate = crate::duplicates::candidates(&documents, &sources)
            .into_iter()
            .find(|candidate| candidate.candidate_key == decision.candidate_key)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::CONFLICT,
                    "duplicate candidate changed; refresh the review queue".into(),
                )
            })?;
        if candidate.kind != decision.kind || candidate.member_ids != decision.member_ids {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "duplicate candidate changed; refresh the review queue".into(),
            ));
        }
        if let DecisionAction::Split { parent_id, .. } = &decision.action {
            if candidate.kind != CandidateKind::Part
                || candidate.part.as_ref().map(|part| &part.parent_id) != Some(parent_id)
            {
                return Err(ApiError(
                    StatusCode::BAD_REQUEST,
                    "split parent must match the candidate's containing document".into(),
                ));
            }
        }
        library.save_duplicate_decision(&decision)?;
        Ok(Json(json!({"decision": decision})))
    })
    .await
}

async fn detail(
    State(library): State<Arc<Library>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    blocking(library, move |library| {
        let document = require_document(library, &id)?;
        let proposals: Vec<StoredProposal> = library.proposal_records(&document.id)?;
        Ok(Json(json!({
            "document": Summary::from(&document),
            "text": document.extracted_text,
            "proposals": proposals,
        })))
    })
    .await
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

async fn upload(
    State(library): State<Arc<Library>>,
    Query(query): Query<UploadQuery>,
    request: Request,
) -> Result<Response, ApiError> {
    if crate::inbox::sensitive_name(&query.filename) {
        return Err(ApiError(StatusCode::FORBIDDEN, "This filename is excluded from automatic intake. Use the local operator workflow for confidential material.".into()));
    }
    let bytes = to_bytes(request.into_body(), MAX_BYTES)
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Upload must be at most 128 MiB.".into(),
            )
        })?;
    blocking(library, move |library| {
        let (document, created) = match library.intake(&query.filename, &bytes)? {
            Stored::New(d) => (d, true),
            Stored::Unchanged(d) => (d, false),
        };
        let proposals = library.proposal_records(&document.id)?;
        Ok((
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({
                "document": Summary::from(&document),
                "proposals": proposals,
                "created": created,
            })),
        )
            .into_response())
    })
    .await
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RegenerateProposalRequest {}

async fn regenerate_proposals(
    State(library): State<Arc<Library>>,
    Path(id): Path<String>,
    Json(_request): Json<RegenerateProposalRequest>,
) -> Result<Json<Vec<StoredProposal>>, ApiError> {
    blocking(library, move |library| {
        require_document(library, &id)?;
        Ok(Json(library.regenerate_rule_proposals(&id)?))
    })
    .await
}

async fn review_proposal(
    State(library): State<Arc<Library>>,
    Path((id, proposal_id)): Path<(String, String)>,
    Json(review): Json<ProposalReviewRequest>,
) -> Result<Json<StoredProposal>, ApiError> {
    blocking(library, move |library| {
        require_document(library, &id)?;
        Ok(Json(library.review_proposal(
            &id,
            &proposal_id,
            review.outcome,
        )?))
    })
    .await
}

async fn review(
    State(library): State<Arc<Library>>,
    Path(id): Path<String>,
    Json(review): Json<Review>,
) -> Result<Json<Summary>, ApiError> {
    blocking(library, move |library| {
        require_document(library, &id)?;
        Ok(Json(Summary::from(&library.review(&id, &review)?)))
    })
    .await
}

async fn archive(
    State(library): State<Arc<Library>>,
    Path(id): Path<String>,
) -> Result<Json<Summary>, ApiError> {
    blocking(library, move |library| {
        require_document(library, &id)?;
        Ok(Json(Summary::from(&library.archive(&id)?)))
    })
    .await
}

#[derive(Default, Deserialize)]
struct OriginalQuery {
    #[serde(default)]
    download: bool,
}

fn encoded_filename(filename: &str) -> String {
    filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn original(
    State(library): State<Arc<Library>>,
    Path(id): Path<String>,
    Query(query): Query<OriginalQuery>,
) -> Result<Response, ApiError> {
    blocking(library, move |library| {
        let document = require_document(library, &id)?;
        let bytes = library.bytes(&id)?;
        let pdf = document.media_type == "application/pdf" && bytes.starts_with(b"%PDF-");
        let kind = if pdf {
            "application/pdf"
        } else {
            "application/octet-stream"
        };
        let disposition = format!(
            "{}; filename*=UTF-8''{}",
            if pdf && !query.download {
                "inline"
            } else {
                "attachment"
            },
            encoded_filename(&document.filename)
        );
        Ok((
            [
                (header::CONTENT_TYPE, kind.to_string()),
                (header::CONTENT_DISPOSITION, disposition),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                (header::CONTENT_SECURITY_POLICY, "sandbox".into()),
                (header::CACHE_CONTROL, "no-store".into()),
            ],
            Body::from(bytes),
        )
            .into_response())
    })
    .await
}

async fn private_response(request: Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn router(library: Arc<Library>) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok", "service":"documents"})) }),
        )
        .route(
            "/routes",
            get(|| async { Json(route_manifest::manifest("documents", ROUTES)) }),
        )
        .route("/api/documents", get(list).post(upload))
        .route("/api/documents/duplicates", get(duplicate_candidates))
        .route(
            "/api/documents/duplicates/review",
            axum::routing::post(review_duplicate),
        )
        .route(
            "/api/documents/{id}/proposals/regenerate",
            axum::routing::post(regenerate_proposals),
        )
        .route(
            "/api/documents/{id}/proposals/{proposal_id}/review",
            axum::routing::post(review_proposal),
        )
        .route(
            "/api/documents/{id}",
            get(detail).patch(review).delete(archive),
        )
        .route("/api/documents/{id}/original", get(original))
        .layer(middleware::from_fn(private_response))
        .layer(middleware::from_fn_with_state(
            "documents",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(library)
}

/// Serve documents through Sjel's loopback credential gate, without agent-token access.
///
/// # Errors
/// The library or asynchronous runtime cannot be opened.
pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let library = Arc::new(crate::open_default()?);
    let port = sjel_config::resolve_port(None, None, DEFAULT_PORT);
    tokio::runtime::Runtime::new()?.block_on(sjel_server::serve(
        "documents",
        sjel_server::Reach::Loopback,
        port,
        router(library),
        sjel_server::InboundAuth::from_deployment().require_credential(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    use tower::ServiceExt;

    fn fixture() -> (Router, PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sjel-documents-api-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let library = Library::open(&root.join("test.db"), root.join("library")).unwrap();
        (router(Arc::new(library)), root)
    }

    async fn call(app: &Router, method: &str, path: &str, body: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), MAX_BYTES).await.unwrap()).unwrap()
    }

    #[test]
    fn manifest_covers_routes_and_write_bodies() {
        assert!(route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    #[tokio::test]
    async fn sha256_lookup_finds_the_bytes_under_every_filename() {
        let (app, _root) = fixture();
        for name in ["a.pdf", "b.pdf"] {
            let path = format!("/api/documents?filename={name}");
            assert_eq!(
                call(&app, "POST", &path, "same bytes").await.status(),
                StatusCode::CREATED
            );
        }
        let sha = crate::library::hex(&<sha2::Sha256 as sha2::Digest>::digest(b"same bytes"));
        let found =
            json_body(call(&app, "GET", &format!("/api/documents?sha256={sha}"), "").await).await;
        assert_eq!(found["total"], 2);
        let absent = json_body(
            call(
                &app,
                "GET",
                &format!("/api/documents?sha256={}", "0".repeat(64)),
                "",
            )
            .await,
        )
        .await;
        assert_eq!(absent["total"], 0);
    }

    #[tokio::test]
    async fn duplicate_queue_requires_a_human_decision_and_keeps_both_originals() {
        let (app, root) = fixture();
        let first = call(
            &app,
            "POST",
            "/api/documents?filename=copy-a.txt",
            "Synthetic duplicate test content",
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first = json_body(first).await["document"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second = call(
            &app,
            "POST",
            "/api/documents?filename=copy-b.txt",
            "Synthetic duplicate test content",
        )
        .await;
        assert_eq!(second.status(), StatusCode::CREATED);
        let second = json_body(second).await["document"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let queue = call(
            &app,
            "GET",
            "/api/documents/duplicates?kind=exact_copy&limit=10",
            "",
        )
        .await;
        assert_eq!(queue.status(), StatusCode::OK);
        let queue = json_body(queue).await;
        assert_eq!(queue["total"], 1);
        let mut expected_members = vec![first.clone(), second.clone()];
        expected_members.sort();
        assert_eq!(
            queue["candidates"][0]["member_ids"],
            json!(expected_members)
        );
        assert!(queue["candidates"][0].get("extracted_text").is_none());
        let key = queue["candidates"][0]["candidate_key"].as_str().unwrap();
        let decision = json!({
            "candidate_key": key,
            "kind": "exact_copy",
            "member_ids": expected_members,
            "action": {"action": "confirm_group", "survivor_id": first.clone()}
        });
        let saved = call(
            &app,
            "POST",
            "/api/documents/duplicates/review",
            &decision.to_string(),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK);
        let queue = call(&app, "GET", "/api/documents/duplicates?kind=exact_copy", "").await;
        assert_eq!(json_body(queue).await["total"], 0);

        let rows = call(&app, "GET", "/api/documents?limit=10", "").await;
        assert_eq!(json_body(rows).await["total"], 2);
        assert_eq!(
            call(&app, "GET", &format!("/api/documents/{first}/original"), "")
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/api/documents/{second}/original"),
                ""
            )
            .await
            .status(),
            StatusCode::OK
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn operator_credential_is_required_for_metadata_and_originals() {
        let (app, root) = fixture();
        let app = sjel_server::authenticated(
            app,
            sjel_server::InboundAuth::resolve(Some("public-test-fixture".into()))
                .require_credential(),
        );
        assert_eq!(
            call(&app, "GET", "/health", "").await.status(),
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "GET", "/api/documents", "").await.status(),
            StatusCode::UNAUTHORIZED
        );
        let original = format!("/api/documents/{}/original", "a".repeat(64));
        assert_eq!(
            call(&app, "GET", &original, "").await.status(),
            StatusCode::UNAUTHORIZED
        );
        let admitted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/documents")
                    .header(header::AUTHORIZATION, "Bearer public-test-fixture")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admitted.status(), StatusCode::OK);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn regeneration_appends_a_distinct_run_without_filing() {
        let (app, root) = fixture();
        let upload = call(
            &app,
            "POST",
            "/api/documents?filename=guide.txt",
            "manual guide",
        )
        .await;
        let created = json_body(upload).await;
        let id = created["document"]["id"].as_str().unwrap();
        let first_id = created["proposals"][0]["id"].as_str().unwrap();
        let regenerated = call(
            &app,
            "POST",
            &format!("/api/documents/{id}/proposals/regenerate"),
            "{}",
        )
        .await;
        assert_eq!(regenerated.status(), StatusCode::OK);
        let new_run = json_body(regenerated).await;
        assert_eq!(new_run.as_array().unwrap().len(), 1);
        assert_ne!(new_run[0]["id"], first_id);
        assert_ne!(
            new_run[0]["generation_id"],
            created["proposals"][0]["generation_id"]
        );
        let detail = json_body(call(&app, "GET", &format!("/api/documents/{id}"), "").await).await;
        assert_eq!(detail["proposals"].as_array().unwrap().len(), 2);
        assert_eq!(detail["document"]["status"], "inbox");
        assert!(detail["document"]["category"].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn upload_replay_metadata_search_review_and_archive_share_one_original() {
        let (app, root) = fixture();
        let upload = call(
            &app,
            "POST",
            "/api/documents?filename=manual.txt",
            "Unique synthetic optics evidence",
        )
        .await;
        assert_eq!(upload.status(), StatusCode::CREATED);
        let created = json_body(upload).await;
        let id = created["document"]["id"].as_str().unwrap();
        assert_eq!(created["document"]["data_class"], "c2");
        assert_eq!(created["document"]["status"], "inbox");
        assert_eq!(
            created["proposals"][0]["proposal"]["outcome"]["category"],
            "Reference"
        );
        assert_eq!(
            created["proposals"][0]["proposal"]["outcome"]["evidence"]["quote"],
            "manual"
        );
        let proposal_id = created["proposals"][0]["id"].as_str().unwrap();
        for outcome in ["accepted", "rejected"] {
            let response = call(
                &app,
                "POST",
                &format!("/api/documents/{id}/proposals/{proposal_id}/review"),
                &format!(r#"{{"outcome":"{outcome}"}}"#),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let reviewed =
            json_body(call(&app, "GET", &format!("/api/documents/{id}"), "").await).await;
        assert_eq!(
            reviewed["proposals"][0]["review_history"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(reviewed["proposals"][0]["proposal"]["review"], "rejected");
        assert_eq!(reviewed["document"]["status"], "inbox");
        assert!(reviewed["document"]["category"].is_null());
        let replay = call(
            &app,
            "POST",
            "/api/documents?filename=manual.txt",
            "Unique synthetic optics evidence",
        )
        .await;
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(json_body(replay).await["created"], false);
        let listed = call(&app, "GET", "/api/documents?q=optics&limit=1", "").await;
        assert_eq!(listed.headers()[header::CACHE_CONTROL], "no-store");
        let listed = json_body(listed).await;
        assert_eq!(listed["total"], 1);
        assert!(listed["documents"][0].get("extracted_text").is_none());
        assert!(listed["documents"][0].get("text").is_none());
        let detail = json_body(call(&app, "GET", &format!("/api/documents/{id}"), "").await).await;
        assert!(detail["text"].as_str().unwrap().contains("optics"));
        assert_eq!(
            detail["proposals"][0]["proposal"]["source"]["kind"],
            "rules"
        );
        assert_eq!(
            detail["proposals"][0]["proposal"]["outcome"]["category"],
            "Reference"
        );
        assert_eq!(detail["proposals"][0]["proposal"]["review"], "rejected");
        assert_eq!(
            detail["proposals"][0]["review_history"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let filed = call(
            &app,
            "PATCH",
            &format!("/api/documents/{id}"),
            r#"{"category":"Reference","project":"Optics"}"#,
        )
        .await;
        assert_eq!(json_body(filed).await["status"], "filed");
        let archived = call(&app, "DELETE", &format!("/api/documents/{id}"), "").await;
        assert_eq!(json_body(archived).await["status"], "archived");
        let original = call(&app, "GET", &format!("/api/documents/{id}/original"), "").await;
        assert_eq!(original.status(), StatusCode::OK);
        assert!(original.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment"));
        assert_eq!(
            to_bytes(original.into_body(), MAX_BYTES).await.unwrap(),
            "Unique synthetic optics evidence"
        );
        let restored = call(
            &app,
            "PATCH",
            &format!("/api/documents/{id}"),
            r#"{"category":"Reference"}"#,
        )
        .await;
        assert_eq!(json_body(restored).await["status"], "filed");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unsafe_names_bad_ids_and_foreign_origins_are_refused() {
        let (app, root) = fixture();
        let refused = call(
            &app,
            "POST",
            "/api/documents?filename=SECRETS.txt",
            "never stored",
        )
        .await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/documents?filename=folder%2Fbad.txt",
                "bytes"
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&app, "GET", "/api/documents/not-an-id", "")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/api/documents/{}", "a".repeat(64)),
                ""
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(&app, "GET", "/api/documents?status=wrong", "")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&app, "GET", "/api/documents?limit=501", "")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let foreign = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/documents")
                    .header(header::ORIGIN, "https://attacker.invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(call(&app, "GET", "/api/documents", "").await).await["total"],
            0
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn original_disposition_cannot_execute_uploaded_html_and_pdf_bytes_stay_exact() {
        let (app, root) = fixture();
        for (filename, bytes, kind, disposition) in [
            (
                "report.pdf",
                "%PDF-1.4\nsynthetic",
                "application/pdf",
                "inline",
            ),
            (
                "fake.pdf",
                "<script>alert(1)</script>",
                "application/octet-stream",
                "attachment",
            ),
            (
                "page.html",
                "<script>alert(1)</script>",
                "application/octet-stream",
                "attachment",
            ),
        ] {
            let created = json_body(
                call(
                    &app,
                    "POST",
                    &format!("/api/documents?filename={filename}"),
                    bytes,
                )
                .await,
            )
            .await;
            let id = created["document"]["id"].as_str().unwrap();
            let response = call(&app, "GET", &format!("/api/documents/{id}/original"), "").await;
            assert_eq!(response.headers()[header::CONTENT_TYPE], kind);
            assert!(response.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with(disposition));
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                to_bytes(response.into_body(), MAX_BYTES).await.unwrap(),
                bytes
            );
            let download = call(
                &app,
                "GET",
                &format!("/api/documents/{id}/original?download=true"),
                "",
            )
            .await;
            assert!(download.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("attachment"));
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn corruption_is_never_served_as_original_bytes() {
        let (app, root) = fixture();
        let created = json_body(
            call(
                &app,
                "POST",
                "/api/documents?filename=guide.txt",
                "synthetic",
            )
            .await,
        )
        .await;
        let id = created["document"]["id"].as_str().unwrap();
        let blobs = root.join("library/blobs");
        fn damage(path: &std::path::Path) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    damage(&path);
                } else {
                    std::fs::write(path, b"corrupt").unwrap();
                }
            }
        }
        damage(&blobs);
        assert_ne!(
            call(&app, "GET", &format!("/api/documents/{id}/original"), "")
                .await
                .status(),
            StatusCode::OK
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

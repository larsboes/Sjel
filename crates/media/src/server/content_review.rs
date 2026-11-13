//! Operator-only review of immutable local evidence.

use super::*;
use media_api::{ContentDecision, ContentDetail, ContentPage, ContentProposal};

#[derive(Clone)]
struct ReviewState {
    ledger: Arc<Ledger>,
    library: Arc<attachments::Library>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueQuery {
    #[serde(default = "pending")]
    state: String,
    #[serde(default)]
    after: i64,
}

fn pending() -> String {
    "pending".into()
}

async fn queue(
    State(state): State<ReviewState>,
    Query(query): Query<QueueQuery>,
) -> Result<Json<ContentPage>, ApiError> {
    if !["pending", "accepted", "rejected", "all"].contains(&query.state.as_str())
        || query.after < 0
    {
        return Err(invalid("invalid content queue state or cursor".into()));
    }
    Ok(Json(
        blocking(state.ledger, move |ledger| {
            media::content::list(ledger, &query.state, query.after).map_err(|e| e.to_string())
        })
        .await?,
    ))
}

async fn detail(
    State(state): State<ReviewState>,
    Path(id): Path<i64>,
) -> Result<Json<ContentDetail>, ApiError> {
    blocking(state.ledger, move |ledger| {
        media::content::detail(ledger, id).map_err(|e| e.to_string())
    })
    .await?
    .map(Json)
    .ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: "content proposal not found".into(),
    })
}

async fn decision(
    State(state): State<ReviewState>,
    Path(id): Path<i64>,
    Json(body): Json<ContentDecision>,
) -> Result<Json<ContentProposal>, ApiError> {
    let saved = tokio::task::spawn_blocking(move || {
        media::content::decide(&state.ledger, &state.library, id, &body).map_err(decision_error)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Json(saved))
}

fn decision_error(error: Box<dyn std::error::Error>) -> ApiError {
    use media::content::DecisionError;
    let status = match error.downcast_ref::<DecisionError>() {
        Some(DecisionError::Invalid(_)) => StatusCode::UNPROCESSABLE_ENTITY,
        Some(DecisionError::NotFound) => StatusCode::NOT_FOUND,
        Some(DecisionError::Conflict(_)) => StatusCode::CONFLICT,
        None => return ApiError::internal(error.to_string()),
    };
    ApiError {
        status,
        message: error.to_string(),
    }
}

pub(super) fn router(ledger: Arc<Ledger>, library: Arc<attachments::Library>) -> Router {
    Router::new()
        .route("/api/content/proposals", get(queue))
        .route("/api/content/proposals/{id}", get(detail))
        .route(
            "/api/content/proposals/{id}/decision",
            axum::routing::post(decision),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(ReviewState { ledger, library })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn post_decision(app: &Router, id: i64, body: Value) -> Response {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/content/proposals/{id}/decision"))
                    .header("authorization", "Bearer public-operator-fixture")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn authorized_decisions_distinguish_refusals_from_internal_failures() {
        use base64::Engine;
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("media-review-decisions-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let ledger = Arc::new(Ledger::open(&root.join("test.db")).unwrap());
        let library =
            Arc::new(attachments::Library::open(ledger.clone(), root.join("originals")).unwrap());
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        library.upload("fixture.png", &digest, &bytes).unwrap();
        let path = root.join("report.json");
        let mut report = json!({"formatVersion":1,"engine":"apple-vision/VNClassifyImageRequest",
            "os":"Synthetic OS","revision":1,"advisory":"Review required","items":[{
                "path":"not-read.png","sha256":digest,"kind":"image","error":null,
                "frames":[{"requestedSeconds":null,"seconds":null,"previewPng":png,
                    "labels":[{"identifier":"fixture","score":0.8}]}]}]});
        for score in [0.8, 0.9] {
            report["items"][0]["frames"][0]["labels"][0]["score"] = json!(score);
            std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
            media::content::stage(&ledger, &library, &path).unwrap();
        }
        let proposals = media::content::list(&ledger, "pending", 0).unwrap().items;
        let first = &proposals[0];
        let second = &proposals[1];
        let app = sjel_server::authenticated(
            router(ledger.clone(), library),
            sjel_server::InboundAuth::with_token(Some("public-operator-fixture".into()))
                .require_credential(),
        );
        let good = json!({"fingerprint":first.fingerprint,"action":"accept","tags":["chosen"]});
        for (id, body, status) in [
            (
                first.id,
                json!({"fingerprint":first.fingerprint,"action":"accept"}),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                first.id,
                json!({"fingerprint":first.fingerprint,"action":"accept","tags":[" "]}),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                first.id,
                json!({"fingerprint":"bad","action":"accept","tags":["chosen"]}),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (999, good.clone(), StatusCode::NOT_FOUND),
            (
                first.id,
                json!({"fingerprint":"b".repeat(64),"action":"accept","tags":["chosen"]}),
                StatusCode::CONFLICT,
            ),
            (first.id, good.clone(), StatusCode::OK),
            (first.id, good.clone(), StatusCode::OK),
            (
                first.id,
                json!({"fingerprint":first.fingerprint,"action":"reject"}),
                StatusCode::CONFLICT,
            ),
        ] {
            assert_eq!(post_decision(&app, id, body).await.status(), status);
        }
        let original = root.join("originals").join(&digest);
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&original, b"corrupt-original").unwrap();
        let pending =
            json!({"fingerprint":second.fingerprint,"action":"accept","collections":["chosen"]});
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&original).unwrap();
            }
            let response = post_decision(&app, second.id, pending.clone()).await;
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                json!({"error":"media ledger operation failed"})
            );
            assert_eq!(
                media::content::detail(&ledger, second.id)
                    .unwrap()
                    .unwrap()
                    .proposal
                    .state,
                media_api::ContentState::Pending
            );
        }
        drop(app);
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn content_review_requires_operator_auth_and_bounds_queries() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("media-review-http-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ledger = Arc::new(Ledger::open(&root.join("test.db")).unwrap());
        let library =
            Arc::new(attachments::Library::open(ledger.clone(), root.join("originals")).unwrap());
        let app = sjel_server::authenticated(
            router(ledger, library),
            sjel_server::InboundAuth::with_token(Some("public-operator-fixture".into()))
                .require_credential(),
        );
        for (method, path) in [
            ("GET", "/api/content/proposals"),
            ("GET", "/api/content/proposals/1"),
            ("POST", "/api/content/proposals/1/decision"),
        ] {
            for token in [None, Some("public-agent-fixture")] {
                let mut request = axum::http::Request::builder().method(method).uri(path);
                if let Some(token) = token {
                    request = request.header("authorization", format!("Bearer {token}"));
                }
                let response = app
                    .clone()
                    .oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        for (path, expected) in [
            ("/api/content/proposals", StatusCode::OK),
            (
                "/api/content/proposals?state=bad",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "/api/content/proposals?after=-1",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("/api/content/proposals/1", StatusCode::NOT_FOUND),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .header("authorization", "Bearer public-operator-fixture")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

use super::*;
use comms::store::{split_tag, CapturePatch};

#[derive(Debug, Deserialize)]
pub(super) struct CaptureBody {
    text: Option<String>,
    url: Option<String>,
    tag: Option<String>,
    client: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CaptureQuery {
    status: Option<String>,
}

async fn with_store<T: Send + 'static>(
    f: impl FnOnce(Store) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        let store = Store::open(&Config::load().database_path).map_err(|e| e.to_string())?;
        f(store)
    })
    .await
    .map_err(|_| "task failed".to_string())?
}

/// Stores only. Nothing is fetched here, so an agent can capture without an approval; moving a
/// URL into the feed goes through `POST /ingest`, which asks.
pub(super) async fn capture_post_handler(Json(body): Json<CaptureBody>) -> HttpResponse {
    let (hashtag, text) = match body.text.as_deref() {
        Some(t) => {
            let (tag, rest) = split_tag(t);
            (tag, Some(rest))
        }
        None => (None, None),
    };
    let tag = body.tag.or(hashtag);
    match with_store(move |s| {
        s.add_capture(text.as_deref(), body.url.as_deref(), tag.as_deref(), body.client.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(c) => (StatusCode::CREATED, Json(json!(c))),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

pub(super) async fn captures_handler(Query(q): Query<CaptureQuery>) -> HttpResponse {
    match with_store(move |s| s.captures(q.status.as_deref()).map_err(|e| e.to_string())).await {
        Ok(list) => (StatusCode::OK, Json(json!(list))),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(super) async fn capture_patch_handler(
    Path(id): Path<String>,
    Json(patch): Json<CapturePatch>,
) -> HttpResponse {
    match with_store(move |s| s.patch_capture(&id, patch).map_err(|e| e.to_string())).await {
        Ok(Some(c)) => (StatusCode::OK, Json(json!(c))),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not found"),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

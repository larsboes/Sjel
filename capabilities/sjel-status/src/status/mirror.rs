//! The read-only copy of this machine's data, for a device that is not here.
//!
//! `sjel mirror build` produces it (route R1 in `research/offline-on-the-phone.md`); these two
//! routes are how a phone fetches it. The manifest is served first so a device can compare a digest
//! and size before spending the transfer, and the file is served by `ServeFile` so a range request
//! resumes rather than restarts.
//!
//! Nothing here decides what the copy contains: the manifest's `path` is the copy's location, and
//! this surface follows it rather than re-deriving the directory. One producer, one reader.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::Json;
use serde_json::json;
use tower::ServiceExt as _;

/// Where the mirror capability writes: `<overlay>/data/mirror/`.
fn mirror_dir() -> Result<std::path::PathBuf, String> {
    sjel_config::overlay_data_dir("mirror")
        .ok_or_else(|| "no overlay resolved, so this machine keeps no copy".to_owned())
}

/// The built copy's path, from its manifest.
fn built_copy() -> Result<std::path::PathBuf, String> {
    let manifest = mirror_dir()?.join("manifest.json");
    let body = std::fs::read_to_string(&manifest).map_err(|e| {
        format!(
            "no copy has been built yet ({}: {e}) — run `sjel mirror build`",
            manifest.display()
        )
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("the manifest is unreadable: {e}"))?;
    value
        .get("path")
        .and_then(|p| p.as_str())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "the manifest names no path".to_owned())
}

/// What the copy holds, when it was built and its digest — the cheap half of a fetch.
pub(crate) async fn mirror_manifest_handler() -> Response {
    let manifest = match mirror_dir() {
        Ok(dir) => dir.join("manifest.json"),
        Err(message) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": message })),
            )
                .into_response()
        }
    };
    match tokio::fs::read_to_string(&manifest).await {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => axum::Json(value).into_response(),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("the manifest is unreadable: {error}") })),
            )
                .into_response(),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no copy has been built yet — run `sjel mirror build`" })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("cannot read the manifest: {error}") })),
        )
            .into_response(),
    }
}

/// The copy itself, streamed with range support so a phone can resume.
pub(crate) async fn mirror_file_handler(request: Request) -> Response {
    let path = match built_copy() {
        Ok(path) => path,
        Err(message) => {
            let status = if message.starts_with("no copy") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            return (status, Json(json!({ "error": message }))).into_response();
        }
    };
    if !path.is_file() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("the manifest names {}, which is not there — rebuild it", path.display())
            })),
        )
            .into_response();
    }
    match tower_http::services::ServeFile::new(&path)
        .oneshot(request)
        .await
    {
        Ok(response) => {
            let (mut parts, body) = response.into_parts();
            // A copy is worth caching until its name changes; the digest is in the manifest, and a
            // device that wants determinism compares that rather than trusting a cache header.
            parts.headers.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            );
            Response::from_parts(parts, Body::new(body))
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("cannot serve {}: {error}", path.display()) })),
        )
            .into_response(),
    }
}

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

use super::axon_root;

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

/// The manifest as a value, or an error and the status that says which kind it is.
async fn read_manifest() -> Result<serde_json::Value, (StatusCode, String)> {
    let dir = mirror_dir().map_err(|message| (StatusCode::SERVICE_UNAVAILABLE, message))?;
    let body = tokio::fs::read_to_string(dir.join("manifest.json"))
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => (
                StatusCode::NOT_FOUND,
                "no copy has been built yet — run `sjel mirror build`".to_owned(),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cannot read the manifest: {error}"),
            ),
        })?;
    serde_json::from_str(&body).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the manifest is unreadable: {error}"),
        )
    })
}

/// What the copy holds, when it was built and its digest — the cheap half of a fetch.
pub(crate) async fn mirror_manifest_handler() -> Response {
    match read_manifest().await {
        Ok(value) => axum::Json(value).into_response(),
        Err((status, message)) => (status, Json(json!({ "error": message }))).into_response(),
    }
}

/// How long a rebuild may take before this route gives up. A build reads a 275 MB source and
/// copies a chosen table set, which measured under a second on this machine
/// (`research/offline-on-the-phone.md`), so this is the runaway bound rather than the expected
/// time: the launcher in front of the CLI may also have to build the crate once.
const REBUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The argv this route runs. No request value reaches it — a device asks for a fresh copy, not
/// for what the copy contains (`PHONE_SET` in `tools/sjel-cli/src/mirror.rs` decides that, and a
/// second decision point here would be a second answer to one question).
const REBUILD_ARGS: [&str; 2] = ["mirror", "build"];

/// One build at a time. A second request while one runs is refused rather than queued: the device
/// that asked twice does not need two copies, and a queued build would answer with one the first
/// request already produced.
static REBUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Holds the build lock for as long as it lives, including a panic or a cancelled request.
struct Building;

impl Drop for Building {
    fn drop(&mut self) {
        REBUILDING.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Take the build lock, or `None` when another build holds it.
fn begin_build() -> Option<Building> {
    REBUILDING
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .ok()
        .map(|_| Building)
}

/// Rebuild the copy now, for a device whose copy is older than it wants.
///
/// The answer is the fresh manifest, so an app that asked for this can go straight on to the
/// fetch without a second round trip to learn what it just built. The build itself is
/// `sjel mirror build` — this route dispatches to the one producer rather than re-implementing
/// how a copy is made, the same way the lifecycle routes hand over to `tools/service-runner.sh`.
///
/// It is a write on an operator surface, so it lands behind the same inbound gate as `start` and
/// `stop`, and no agent token reaches it (`status/runtime.rs`).
pub(crate) async fn mirror_rebuild_handler() -> Response {
    let Some(_building) = begin_build() else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "a build is already running — ask again when it is done" })),
        )
            .into_response();
    };
    let root = match axon_root() {
        Ok(root) => root,
        Err(message) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": message })),
            )
                .into_response()
        }
    };
    let mut command = tokio::process::Command::new(root.join("sjel"));
    command.args(REBUILD_ARGS);
    let ran = tokio::time::timeout(REBUILD_TIMEOUT, command.output()).await;
    match ran {
        Ok(Ok(out)) if out.status.success() => match read_manifest().await {
            Ok(value) => axum::Json(value).into_response(),
            Err((status, message)) => {
                (status, Json(json!({ "error": message }))).into_response()
            }
        },
        Ok(Ok(out)) => {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "the build failed", "detail": detail })),
            )
                .into_response()
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("could not run sjel mirror build: {error}") })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({ "error": format!("the build took longer than {} s", REBUILD_TIMEOUT.as_secs()) })),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The route grows no argv from the request: a phone asks for a rebuild, and the table set
    /// stays decided in one place.
    #[test]
    fn the_rebuild_runs_the_documented_verb() {
        assert_eq!(REBUILD_ARGS, ["mirror", "build"]);
    }

    #[test]
    fn a_second_build_is_refused_while_one_runs() {
        let first = begin_build().expect("the lock starts free");
        assert!(begin_build().is_none(), "a second request must be refused");
        drop(first);
        assert!(begin_build().is_some(), "the lock is released on drop");
    }
}

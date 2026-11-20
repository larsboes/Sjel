//! Snapshot requests on a protected local owner socket.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::{serve_unix, InboundAuth};
use axum::{http::StatusCode, routing::post, Router};

fn socket_path(owner: &str) -> Result<PathBuf, &'static str> {
    sjel_config::overlay_root()
        .map(|root| root.join("secrets").join(format!("{owner}-jobs.sock")))
        .ok_or("owner snapshot requires an overlay")
}

/// Request a snapshot from the running owner.
///
/// # Errors
/// Returns an error if the protected socket or snapshot operation is unavailable.
pub async fn submit(owner: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .unix_socket(socket_path(owner)?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?;
    let url = "http://127.0.0.1/snapshot";
    let bearer = InboundAuth::from_deployment()
        .bearer_header()
        .ok_or("owner snapshot requires the deployment credential")?;
    let response = client
        .post(url)
        .header(reqwest::header::AUTHORIZATION, bearer)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "{owner} snapshot returned {status}: {}",
            response.text().await?
        )
        .into());
    }
    Ok(())
}

/// Serve a closed snapshot job on the owner's protected socket.
pub async fn serve(
    owner: &str,
    database: Arc<PathBuf>,
    snapshot: impl Fn(&Path, &Path) -> Result<(), String> + Clone + Send + Sync + 'static,
) {
    let socket = socket_path(owner).unwrap_or_else(|error| panic!("{owner}: {error}"));
    let target = sjel_config::overlay_data_dir(owner)
        .expect("owner snapshot requires an overlay")
        .join("snapshots")
        .join(format!("{owner}.db"));
    let router = Router::new().route(
        "/snapshot",
        post(move || async move {
            tokio::task::spawn_blocking(move || snapshot(&database, &target))
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
            Ok::<_, (StatusCode, String)>(StatusCode::NO_CONTENT)
        }),
    );
    serve_unix(
        &format!("{owner}-jobs"),
        &socket,
        router,
        InboundAuth::from_deployment().require_credential(),
    )
    .await;
}

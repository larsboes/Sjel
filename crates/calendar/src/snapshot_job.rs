//! Snapshot requests on Calendar's protected local socket.

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{http::StatusCode, routing::post, Router};
use calendar::store::CalendarStore;
use sjel_server::InboundAuth;

fn socket_path() -> Result<PathBuf, &'static str> {
    sjel_config::overlay_root()
        .map(|root| root.join("secrets/calendar-jobs.sock"))
        .ok_or("Calendar snapshot requires an overlay")
}

pub(super) async fn submit() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .unix_socket(socket_path()?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?;
    let url = "http://127.0.0.1/snapshot";
    let bearer = InboundAuth::from_deployment()
        .bearer_header()
        .ok_or("Calendar snapshot requires the deployment credential")?;
    let response = client
        .post(url)
        .header(reqwest::header::AUTHORIZATION, bearer)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Calendar snapshot returned {status}: {}",
            response.text().await?
        )
        .into());
    }
    Ok(())
}

pub(super) async fn serve(database: Arc<PathBuf>) {
    let socket = socket_path().unwrap_or_else(|error| super::fatal(error));
    let target = sjel_config::overlay_data_dir("calendar")
        .unwrap_or_else(|| super::fatal("Calendar snapshot requires an overlay"))
        .join("snapshots/calendar.db");
    let router = Router::new().route(
        "/snapshot",
        post(move || async move {
            tokio::task::spawn_blocking(move || {
                CalendarStore::open(&database)
                    .and_then(|store| store.snapshot(&database, &target))
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
            Ok::<_, (StatusCode, String)>(StatusCode::NO_CONTENT)
        }),
    );
    sjel_server::serve_unix(
        "calendar-jobs",
        &socket,
        router,
        InboundAuth::from_deployment().require_credential(),
    )
    .await;
}

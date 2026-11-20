//! Authenticated local Places commands and metadata snapshots.

use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

use axum::{http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sjel_server::{owner_snapshot, InboundAuth};

use places::{config::Config, store::PlacesStore};

#[derive(Deserialize, Serialize)]
struct CommandResult {
    stdout: String,
    error: Option<String>,
}

fn client() -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
    Ok(reqwest::blocking::Client::builder()
        .unix_socket(owner_snapshot::socket_path("places")?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?)
}

/// Send a command to the running owner.
///
/// # Errors
/// The active owner, command, or output rejected the request.
pub fn request(args: &[String], out: &mut impl Write) -> Result<bool, Box<dyn std::error::Error>> {
    let url = "http://127.0.0.1/commands";
    let request = InboundAuth::with_loopback_auth(client()?.post(url).json(args), url);
    let response = match request.send() {
        Ok(response) => response.error_for_status()?,
        Err(error) if error.is_connect() => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let result: CommandResult = response.json()?;
    out.write_all(result.stdout.as_bytes())?;
    if let Some(error) = result.error {
        return Err(error.into());
    }
    Ok(true)
}

/// Snapshot through the owning process or a closed local database.
///
/// # Errors
/// The active owner or the offline database refuses the snapshot.
pub fn snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let url = "http://127.0.0.1/snapshot";
    let request = InboundAuth::with_loopback_auth(client()?.post(url), url);
    match request.send() {
        Ok(response) => {
            response.error_for_status()?;
            Ok(())
        }
        Err(error) if error.is_connect() => {
            let target = sjel_config::overlay_data_dir("places")
                .ok_or("no active Sjel overlay")?
                .join("snapshots/places.db");
            let path = Config::load().database_path;
            PlacesStore::open(&path)?.snapshot(&path, &target)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn serve(database: Arc<PathBuf>) {
    let job_database = Arc::clone(&database);
    let jobs = Router::new()
        .route(
            "/commands",
            post(move |Json(args): Json<Vec<String>>| async move {
                tokio::task::spawn_blocking(move || {
                    let mut out = Vec::new();
                    let result = PlacesStore::open(&job_database)
                        .and_then(|store| crate::commands::run(&store, &args, &mut out));
                    Json(CommandResult {
                        stdout: String::from_utf8_lossy(&out).into_owned(),
                        error: result.err().map(|error| error.to_string()),
                    })
                })
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            }),
        )
        .merge(crate::server::private_jobs(Arc::clone(&database)));
    owner_snapshot::serve_with_jobs(
        "places",
        database,
        |path, target| {
            PlacesStore::open(path)
                .and_then(|store| store.snapshot(path, target))
                .map_err(|error| error.to_string())
        },
        jobs,
    )
    .await;
}

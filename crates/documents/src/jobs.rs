//! Authenticated local Documents commands and metadata snapshots.

use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

use axum::{http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sjel_server::{owner_snapshot, InboundAuth};

use crate::Library;

#[derive(Deserialize, Serialize)]
struct CommandResult {
    stdout: String,
    error: Option<String>,
}

fn client() -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
    Ok(reqwest::blocking::Client::builder()
        .unix_socket(owner_snapshot::socket_path("documents")?)
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
            let target = sjel_config::overlay_data_dir("documents")
                .ok_or("no active Sjel overlay")?
                .join("snapshots/documents.db");
            crate::open_default()?.snapshot(&crate::database_path()?, &target)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn serve(library: Arc<Library>, database: PathBuf) {
    let handler_library = Arc::clone(&library);
    let jobs = Router::new().route(
        "/commands",
        post(move |Json(args): Json<Vec<String>>| async move {
            tokio::task::spawn_blocking(move || {
                let mut stdout = Vec::new();
                let error = crate::commands::run(&handler_library, &args, &mut stdout)
                    .err()
                    .map(|error| error.to_string());
                Json(CommandResult {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    error,
                })
            })
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }),
    );
    owner_snapshot::serve_with_jobs(
        "documents",
        Arc::new(database),
        move |database, target| {
            library
                .snapshot(database, target)
                .map_err(|error| error.to_string())
        },
        jobs,
    )
    .await;
}

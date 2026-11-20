//! Local Finance commands and closed snapshots.

use std::time::Duration;

use axum::{http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sjel_server::{owner_snapshot, InboundAuth};

use crate::{commands, config::Config, store::FinanceStore};

#[derive(Deserialize, Serialize)]
struct Reply {
    stdout: String,
    result: Result<bool, String>,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    let socket = owner_snapshot::socket_path("finance")?;
    reqwest::blocking::Client::builder()
        .unix_socket(socket)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())
}

/// Submit a command. A closed socket permits an exclusive offline open.
///
/// # Errors
/// The live owner or its response rejected the request.
pub fn request(args: &[String], stdout: &mut String) -> Result<Option<bool>, String> {
    let url = "http://127.0.0.1/commands";
    let request = InboundAuth::with_loopback_auth(client()?.post(url).json(args), url);
    let reply = match request.send() {
        Ok(response) => response
            .error_for_status()
            .and_then(|r| r.json::<Reply>())
            .map_err(|error| error.to_string())?,
        Err(error) if error.is_connect() => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    stdout.push_str(&reply.stdout);
    reply.result.map(Some)
}

/// Snapshot from the running owner or the closed local database.
///
/// # Errors
/// The database or backup destination could not be opened.
pub fn snapshot() -> Result<(), String> {
    let url = "http://127.0.0.1/snapshot";
    match InboundAuth::with_loopback_auth(client()?.post(url), url).send() {
        Ok(response) => response
            .error_for_status()
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Err(error) if error.is_connect() => {
            let target = sjel_config::overlay_data_dir("finance")
                .ok_or("Finance snapshot requires an overlay")?
                .join("snapshots/finance.db");
            let path = Config::load().database_path;
            if path == sjel_config::database_path() {
                return Err("Turso requires a separate Finance owner database".into());
            }
            FinanceStore::open(&path)
                .and_then(|store| store.snapshot(&path, &target))
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

pub fn router() -> Router {
    Router::new().route(
        "/commands",
        post(|Json(args): Json<Vec<String>>| async move {
            tokio::task::spawn_blocking(move || {
                let mut stdout = String::new();
                let result = commands::run(&args, &mut stdout);
                Json(Reply { stdout, result })
            })
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }),
    )
}

//! Punctuality CLI commands served by the local owner.

use std::sync::{Arc, Mutex};

use axum::{http::StatusCode, routing::post, Json, Router};
use punctuality::{commands, config::Config, store::Store};

pub(super) fn router(config: Arc<Config>, store: Arc<Store>) -> Router {
    let ingest = Arc::new(Mutex::new(()));
    Router::new().route(
        "/commands",
        post(move |Json(args): Json<Vec<String>>| async move {
            tokio::task::spawn_blocking(move || {
                let _guard = match args.first().map(String::as_str) {
                    Some("ingest") => Some(ingest.try_lock().map_err(|_| {
                        (StatusCode::CONFLICT, "ingest is already running".to_owned())
                    })?),
                    _ => None,
                };
                commands::run(&args, &config, &store)
                    .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))
            })
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        }),
    )
}

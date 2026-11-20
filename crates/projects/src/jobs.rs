//! Projects commands sent to the running owner over its credentialed local socket.

use std::{sync::Arc, time::Duration};

use axum::{routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sjel_server::{owner_snapshot, InboundAuth};

use crate::Projects;

const COMMANDS: &str = "http://127.0.0.1/commands";

/// What the owner returns for one command.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct CommandResult {
    /// The command's own output.
    pub stdout: String,
    /// Notes from the implicit reindex.
    pub stderr: String,
    /// The refusal, when the command failed.
    pub error: Option<String>,
}

/// Send `args` to the running owner. `Ok(None)` means no owner answers on the socket.
///
/// # Errors
/// The socket path cannot be resolved, or the owner rejects the request.
pub fn request(
    args: &[String],
) -> Result<Option<CommandResult>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::blocking::Client::builder()
        .unix_socket(owner_snapshot::socket_path("projects")?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?;
    let response =
        match InboundAuth::with_loopback_auth(client.post(COMMANDS).json(args), COMMANDS).send() {
            Ok(response) => response.error_for_status()?,
            Err(error) if error.is_connect() => return Ok(None),
            Err(error) => return Err(error.into()),
        };
    Ok(Some(response.json()?))
}

/// The `/commands` route for the owner's job socket.
pub fn router(projects: Arc<Projects>) -> Router {
    Router::new().route(
        "/commands",
        post(move |Json(args): Json<Vec<String>>| {
            let projects = Arc::clone(&projects);
            async move {
                let mut result = CommandResult::default();
                result.error =
                    crate::commands::run(&projects, &args, &mut result.stdout, &mut result.stderr)
                        .await
                        .err()
                        .map(|error| error.to_string());
                Json(result)
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::util::ServiceExt;

    async fn call(router: &Router, args: &[&str]) -> CommandResult {
        let body = serde_json::to_vec(&args).unwrap();
        let response = router
            .clone()
            .oneshot(
                Request::post("/commands")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn the_owner_runs_cli_commands_and_returns_their_output() {
        let (projects, dir) = crate::store::tests::scratch("jobs-commands").await;
        std::fs::create_dir_all(dir.join("rendered")).unwrap();
        let router = router(Arc::new(projects));
        let added = call(&router, &["new-project", "Pilot"]).await;
        assert_eq!(added.error, None);
        assert!(added.stdout.starts_with("added Pilot/"), "{}", added.stdout);
        let listed = call(&router, &["projects"]).await;
        assert!(listed.stdout.contains("Pilot"), "{}", listed.stdout);
        let path = added.stdout.trim().trim_start_matches("added ").to_owned();
        let shown = call(&router, &["show", &path]).await;
        assert!(shown.stdout.contains("type: project"), "{}", shown.stdout);
        let edit = call(&router, &["edit", &path]).await;
        assert!(edit.error.is_some_and(|e| e.contains("runs in the CLI")));
        let unknown = call(&router, &["nonsense"]).await;
        assert!(unknown.error.is_some_and(|e| e.starts_with("usage:")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

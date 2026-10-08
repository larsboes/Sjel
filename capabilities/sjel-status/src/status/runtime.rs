use super::*;
use sjel_runtime::{Files, Power, Selection, Status, Update};

type Error = (StatusCode, Json<Value>);

fn failure(code: StatusCode, message: impl Into<String>) -> Error {
    (code, Json(json!({ "error": message.into() })))
}

fn update_failure(message: String) -> Error {
    let code = if message.starts_with("runtime preferences changed") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    failure(code, message)
}

fn unconfigured() -> Status {
    Status {
        device: String::new(),
        selection: Selection::Normal,
        effective: Selection::Normal,
        power: Power::Unknown,
        power_fresh: false,
        allow: Default::default(),
        revision: 0,
        detail: Some("No deployment is configured; runtime controls cannot be saved.".into()),
        configured: false,
    }
}

pub(crate) async fn runtime_handler() -> Result<Json<Status>, Error> {
    tokio::task::spawn_blocking(|| {
        match Files::from_deployment().map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e))? {
            Some(files) => files
                .status()
                .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e)),
            None => Ok(unconfigured()),
        }
    })
    .await
    .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map(Json)
}

// sjel-status deliberately does not admit_agents: the existing credential middleware
// rejects agent tokens before any operator control, including this route, can run.
pub(crate) async fn runtime_update_handler(
    Json(update): Json<Update>,
) -> Result<Json<Status>, Error> {
    if update.selection.is_none() && update.allow.is_none() {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "Specify selection or allow.",
        ));
    }
    tokio::task::spawn_blocking(move || {
        let files = Files::from_deployment()
            .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| {
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "No deployment is configured; runtime controls cannot be saved.",
                )
            })?;
        if update.selection.is_none()
            && !files
                .status()
                .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e))?
                .configured
        {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "Save a selection before setting exceptions.",
            ));
        }
        files.update(update).map_err(update_failure)
    })
    .await
    .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map(Json)
}

/// Startup refresh completes before serving; the owned task then refreshes every 30 seconds.
pub(crate) struct RuntimeMonitor(tokio::task::JoinHandle<()>);

impl RuntimeMonitor {
    pub(crate) async fn start() -> Self {
        refresh().await;
        Self(tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                refresh().await;
            }
        }))
    }
}

async fn refresh() {
    let result = tokio::task::spawn_blocking(|| {
        if let Some(files) = Files::from_deployment()? {
            files.refresh_power()?;
        }
        Ok::<_, String>(())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("[sjel-status] runtime power refresh: {e}"),
        Err(e) => eprintln!("[sjel-status] runtime power refresh task: {e}"),
    }
}

impl Drop for RuntimeMonitor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn runtime_writes_require_owner_credentials_not_agent_tokens() {
        // Use the real shell router and the same auth policy main installs. Not admitting
        // agent access is intentional: even an enrolled token cannot act as the owner.
        let router = || {
            sjel_server::authenticated(
                crate::build_router(crate::proxy::Proxy::new(
                    &[],
                    "8082",
                    "dashboard/dist".into(),
                )),
                sjel_server::InboundAuth::with_token(Some("owner-test-token".into()))
                    .require_credential(),
            )
        };
        for token in [None, Some("agent-test-token")] {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/sjel-status/runtime")
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let response = router()
                .oneshot(
                    request
                        .body(Body::from(r#"{"selection":"on-the-go"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        // Invalid owner input reaches JSON validation but cannot write any preferences.
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sjel-status/runtime")
                    .header("authorization", "Bearer owner-test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"selection":"invalid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn empty_updates_are_bad_requests_without_touching_deployment_files() {
        let error = runtime_update_handler(Json(Update {
            selection: None,
            allow: None,
            ..Default::default()
        }))
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(unconfigured().effective, Selection::Normal);
        assert!(!unconfigured().configured);
    }

    #[test]
    fn stale_revision_errors_are_conflicts_and_preserve_the_reason() {
        let message = "runtime preferences changed: expected revision 1, found 2";
        let error = update_failure(message.into());
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert_eq!(error.1 .0["error"], message);
        assert_eq!(
            update_failure("cannot save preferences".into()).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn runtime_updates_reject_unknown_fields_and_categories() {
        for body in [
            r#"{"selection":"battery"}"#,
            r#"{"allow":["unknown"]}"#,
            r#"{"enabled":true}"#,
        ] {
            assert!(serde_json::from_str::<Update>(body).is_err(), "{body}");
        }
        let update: Update =
            serde_json::from_str(r#"{"selection":"auto","allow":["transcription"]}"#).unwrap();
        assert!(update.selection.is_some());
        assert_eq!(update.allow.unwrap().len(), 1);
    }
}

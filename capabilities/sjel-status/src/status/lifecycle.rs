use super::*;

/// Run one allow-listed capability lifecycle action through service-runner.
pub(crate) async fn lifecycle(
    name: String,
    action: &'static str,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let Some(service) = services.into_iter().find(|s| s.name == name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": format!("'{name}' is not an enabled capability on this machine") }),
            ),
        ));
    };

    // An external capability is in the registry so its health can be READ, and for no other
    // reason (retired-tracker#169). Independently managed overlays stay separate operational
    // authorities: whoever owns that host owns its lifecycle, its secrets and its backups.
    // Refused here rather than left to fail downstream — `service-runner.sh` would look for a
    // local process, not find one, and report something that reads like an outage on a service
    // that is running perfectly well somewhere else.
    if service.is_external() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": format!("'{name}' is provided by another deployment — this machine may read its health, not {action} it"),
            })),
        ));
    }

    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/service-runner.sh"))
        .arg(action)
        .arg(&service.name)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/service-runner.sh: {e}")))?;

    let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{action} {name} failed"), "detail": detail })),
        ));
    }

    let up = is_up(&reqwest::Client::new(), &service).await;
    Ok(Json(json!({
        "name": service.name,
        "action": action,
        "up": up,
        "detail": detail,
    })))
}

pub(crate) async fn start_handler(
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // `resume`, not `start`: a capability stopped through this API carries a
    // maintenance hold, and `start` deliberately no-ops while one is set. Asking for it
    // from the UI means you want it back.
    lifecycle(name, "resume").await
}

pub(crate) async fn stop_handler(
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    lifecycle(name, "stop").await
}

pub(crate) fn bad_gateway(msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": msg.into() })),
    )
}

// --- backups ------------------------------------------------------------------------
//
// A backup you cannot see the age of is a backup you find out about during a restore.
// `tools/backup.sh` has written a receipt per capability since it was built, and the
// manifests have declared what timely means for that data for just as long — but nothing
// read either, so the two numbers sat in the schema with a comment admitting the gap.
// This is the reader, plus the one button that fixes what it reports.
//
// The security model is `lifecycle`'s, unchanged: a capability NAME the registry already
// lists, passed to a tool, never a command, never a path, never a destination. This
// handler additionally refuses a name with no backup contract, so the reachable set is
// smaller than the reachable set for start/stop rather than larger.

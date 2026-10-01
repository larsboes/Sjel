use super::*;

/// Body of `POST .../upstreams/watch`.
#[derive(Deserialize)]
pub(crate) struct WatchRequest {
    pub(crate) url: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
}

/// Append a `watch` row to upstreams.toml from the dashboard's Projects page.
///
/// Shells out for the same reason `packs_handler` does: `tools/research-registers.ts` owns
/// the row format, the refusals and the license lookup, and the CLI runs the same code. A
/// refusal from the tool is the caller's input, so it answers 400 with the tool's one-line
/// reason. Failing to run the tool at all is this host's problem, so it answers 502.
///
/// A watch row grants nothing (upstreams.toml header): no code lands on it, so writing one
/// from a browser changes what the register lists, not what the build trusts.
pub(crate) async fn upstream_watch_handler(
    Json(request): Json<WatchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let mut command = tokio::process::Command::new(root.join("tools/research-registers.ts"));
    command
        .arg("watch")
        .arg("--url")
        .arg(&request.url)
        .arg("--summary")
        .arg(&request.summary)
        .current_dir(&root);
    if let Some(name) = request.name.as_deref().filter(|n| !n.trim().is_empty()) {
        command.arg("--name").arg(name);
    }
    let out = command
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/research-registers: {e}")))?;
    if !out.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": String::from_utf8_lossy(&out.stderr).trim() })),
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map(Json)
        .map_err(|e| bad_gateway(format!("tools/research-registers did not emit JSON: {e}")))
}

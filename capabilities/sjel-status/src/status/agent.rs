use super::*;

use sjel_server::agent_policy::{Mode, PolicyFiles};

/// The agent's reach on this machine, for the Systems page (ISA F10).
///
/// This process owns the page, so it owns the writes to the policy and the decisions on
/// approvals. Each capability's gate reads the same files on its next request
/// (`libs/sjel-server/src/agent_policy.rs`), so nothing restarts when a mode changes.
fn files() -> Result<PolicyFiles, (StatusCode, Json<Value>)> {
    PolicyFiles::from_deployment().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "no overlay is configured, so there is no agent policy" })),
        )
    })
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
}

/// `GET /api/sjel-status/agent`: whether an agent is enrolled, each gate with its mode, the
/// writes waiting for the owner, and the latest calls.
pub(crate) async fn agent_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let files = files()?;
    let enrolled = sjel_config::overlay_root()
        .is_some_and(|root| root.join("config/agent-token.sha256").is_file());
    let modes = files.modes().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("agent policy: {e}") })),
        )
    })?;
    let capabilities: Vec<Value> = files
        .gates()
        .into_iter()
        .map(|mut gate| {
            let name = gate["capability"].as_str().unwrap_or_default().to_string();
            let mode = modes
                .get(&name)
                .copied()
                .unwrap_or(sjel_server::agent_policy::DEFAULT_MODE);
            gate["mode"] = Value::from(mode.as_str());
            gate
        })
        .collect();
    Ok(Json(json!({
        "enrolled": enrolled,
        "modes": Mode::ALL.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
        "default_mode": sjel_server::agent_policy::DEFAULT_MODE.as_str(),
        "capabilities": capabilities,
        "pending": files.approvals(Some("pending")),
        "calls": files.recent_calls(50),
    })))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModeBody {
    capability: String,
    mode: String,
}

/// `POST /api/sjel-status/agent/mode`: set one capability's mode.
pub(crate) async fn agent_mode_handler(
    Json(body): Json<ModeBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let files = files()?;
    let mode = Mode::parse(&body.mode).ok_or_else(|| {
        bad_request(format!(
            "mode must be one of: {}",
            Mode::ALL
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let known = files
        .gates()
        .iter()
        .any(|gate| gate["capability"] == body.capability.as_str());
    if !known {
        return Err(bad_request(format!(
            "{} has no agent gate on this machine",
            body.capability
        )));
    }
    files.set_mode(&body.capability, mode).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
    })?;
    Ok(Json(
        json!({ "capability": body.capability, "mode": mode.as_str() }),
    ))
}

#[derive(Debug, Deserialize)]
pub(crate) struct DecisionBody {
    allow: bool,
}

/// `POST /api/sjel-status/agent/approvals/{id}`: the owner's Allow or Deny.
pub(crate) async fn agent_decision_handler(
    Path(id): Path<String>,
    Json(body): Json<DecisionBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let files = files()?;
    files
        .decide(&id, body.allow)
        .map(Json)
        .map_err(|e| (StatusCode::CONFLICT, Json(json!({ "error": e }))))
}

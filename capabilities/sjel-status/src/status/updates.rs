//! Software installed outside this checkout, straight from `tools/updates --json`, and the one
//! write route that moves it.
//!
//! ## Why this capability serves a tool's report, again
//!
//! `tools/updates` is operator machinery with no server, exactly as `tools/storage` is, and the
//! Systems page is where a person asks "is anything stale". The measurement stays with the tool:
//! re-deriving the `cargo install --list` parse or the brew registry query here would be a second
//! answer to a question that already has one, and the two would disagree the first time a class
//! was added to `SURFACES` in `tools/sjel-cli/src/updates/report.rs`. `storage.rs` states the
//! same rule for the same reason.
//!
//! ## Why the apply route answers before the work is done
//!
//! This is the one thing the storage panel does not have to solve. `tools/updates apply --only
//! cargo` runs `cargo install --locked --force` per crate, which compiles from source and takes
//! minutes — measured 2026-10-01 at about four minutes for three crates. Holding the HTTP
//! response open for that would make the browser's own timeout the error message while the
//! install succeeded, which is the worst of the available outcomes: the owner sees a failure and
//! the machine is changed.
//!
//! So the handler validates the class, spawns the tool detached, and returns `202 Accepted`
//! immediately. The tool writes `<overlay>/data/updates/last-apply.json` — the same receipt shape
//! the two scheduled jobs use, and for the same reason — and every `GET` carries it under
//! `lastApply`, so the panel learns the outcome by polling the report it already reads. No second
//! endpoint, no held request, and no state in this process: a restart mid-apply loses nothing
//! because the receipt is the state.
//!
//! ## What this route will and will not accept
//!
//! `class` is checked against [`APPLY_CLASSES`] before anything is spawned. That list is a
//! deliberate allowlist and not a passthrough: the class becomes an argv element, so accepting a
//! caller's string would let a request choose which program runs. A class outside the list is a
//! 400 that names the ones that exist.
//!
//! ## The gate it inherits rather than restates
//!
//! Nothing here checks a credential or an origin. Both come from the layers `main.rs` applies to
//! every route above its fallback — `sjel_server::origin::refuse_foreign_origins` and the
//! session verifier on the loopback listener — and this route is mounted above that line for
//! exactly that reason. sjel-status is also the one capability that does not admit the agent
//! token at all (ISA ISC-47), so an apply is reachable only by the owner's browser session or the
//! deployment credential, never by an agent.

use super::*;

/// The classes a caller may ask `apply` to move.
///
/// Every entry is an id `tools/updates` knows, and the list is narrower than `SURFACES` on
/// purpose: `containers` belongs to `capabilities/container-refresh`, `checkout` to
/// `tools/update.sh`, and `vendor` to the vendor. Offering them here would put a second mover
/// behind a button for something that already has one — the two-owners failure `tools/host-patch.sh`
/// names. `brew`, `uv` and `rustup` ARE here because asking for them delegates straight back to
/// `tools/host-patch.sh`; the tool never runs a package manager it does not own.
pub(crate) const APPLY_CLASSES: &[&str] = &[
    "brew",
    "uv",
    "rustup",
    "graphify",
    "interceptor",
    "cargo",
    "npm",
];

#[derive(serde::Deserialize)]
pub(crate) struct ApplyRequest {
    pub(crate) class: String,
}

/// `GET /api/sjel-status/updates` — the tool's own report, unchanged.
///
/// `report` exits 1 whenever anything is stale, which is its normal answer on a healthy machine
/// and the whole point of the panel. The exit code is therefore only consulted when there is
/// nothing to parse, via the shared [`interpret_tool_report`].
pub(crate) async fn updates_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/updates"))
        .arg("report")
        .arg("--json")
        .current_dir(&root)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/updates: {e}")))?;
    interpret_tool_report("tools/updates", &out.stdout, &out.stderr, out.status.code())
        .map(Json)
        .map_err(bad_gateway)
}

/// `POST /api/sjel-status/updates/apply` — start one class moving, and answer at once.
///
/// Body: `{ class }`, one of [`APPLY_CLASSES`]. `202` means started, not finished; the caller
/// reads `lastApply` from the next `GET`.
pub(crate) async fn updates_apply_handler(
    Json(request): Json<ApplyRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let class = request.class.trim();
    if !APPLY_CLASSES.contains(&class) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "'{class}' is not a class this route applies. One of: {}",
                    APPLY_CLASSES.join(", ")
                ),
            })),
        ));
    }
    let root = axon_root().map_err(bad_gateway)?;
    // Detached: the child outlives this request by design, and its stdout is dropped because the
    // receipt — not this process — is what reports the outcome. `--yes` because there is no TTY
    // here and the tool refuses to install unattended without it.
    tokio::process::Command::new(root.join("tools/updates"))
        .arg("apply")
        .arg("--only")
        .arg(class)
        .arg("--yes")
        .current_dir(&root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| bad_gateway(format!("could not start tools/updates apply: {e}")))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "started": true, "class": class })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is the security boundary: the class becomes an argv element, so this asserts the
    /// shape rather than a count — every entry is a plain id with nothing that could read as a
    /// flag, a path or a second command.
    #[test]
    fn every_apply_class_is_a_bare_id() {
        for class in APPLY_CLASSES {
            assert!(
                class.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "'{class}' is not a bare lowercase id"
            );
            assert!(!class.starts_with('-'), "'{class}' could be read as a flag");
        }
    }

    /// The classes that belong to someone else must not be reachable through this route. A
    /// `containers` or `checkout` entry here would be a second mover for a thing that has one.
    #[test]
    fn classes_owned_by_another_tool_are_not_offered() {
        for owned_elsewhere in ["containers", "checkout", "vendor"] {
            assert!(
                !APPLY_CLASSES.contains(&owned_elsewhere),
                "'{owned_elsewhere}' is moved by another owner and must not be applied here"
            );
        }
    }
}

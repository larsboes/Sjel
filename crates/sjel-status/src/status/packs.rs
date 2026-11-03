use super::*;

/// What Sjel has deployed into each agent harness on this machine.
///
/// ## Why this capability serves another capability's state
///
/// `packs` is `kind = "data"`: it owns the deployment ledgers and nothing starts, so it
/// cannot serve them and something that is always up has to. That is this process, for
/// the same reason it publishes `host-watch`'s findings and `backup.sh`'s receipts — both
/// written by things with no port. Ownership does not move with the surface: `packs` owns
/// the ledgers, `tools/sjel-cli/src/harnesses/` owns their format and every mutating verb,
/// and this keeps no copy of either — it reads, and it shells the verbs the CLI runs.
///
/// ## Why the write is owner-only, and why it is here
///
/// `tools/harnesses` has no `deploy` or `remove` verb, and its `sync` refuses a registry
/// harness with a hint rather than a sync — so a route routed through it would leave the pi
/// column's buttons doing nothing. Every verb goes to the harness's own launcher
/// (`tools/packs-<harness>`), which carries `deploy`, `sync` and `remove` in one shape, and the
/// route dispatches rather than re-implements: it hands over argv and reports what came back.
///
/// A harness may have no launcher at all. sjel-agent is a registry harness declared in its own
/// overlay config, and its Pack set is changed there; the route says so instead of spawning a
/// binary that does not exist.
///
/// An agent cannot reach this route. `sjel-status` does not admit agent tokens (see
/// `status/runtime.rs`), and that is deliberate here rather than incidental: a deploy writes
/// outside Sjel into a real skill root, and `remove` cannot be undone. An agent that wants one
/// runs `tools/harnesses` in its own shell, where the harness's own permission gate — not this
/// capability — decides.
///
/// ## Why it shells out
///
/// Which harnesses exist, which are installed, what each ledger claims and what sits
/// unowned at each destination are all answered by `tools/harnesses`, which is also what
/// the CLI and the session hook read. Re-deriving any of it here would be a second
/// implementation of the one question, and the two would disagree the first time a
/// harness was added. Same pattern as `tools/capability.sh registry`, `tools/repos`,
/// `tools/backup.sh` and `tools/service-runner.sh`, which this process already runs.
///
/// Uncached on purpose: the call reads a handful of small JSON ledgers and the digests of
/// the deployed trees, the dashboard polls it on the slow beat, and a cache keyed on
/// anything less than every source file under `Packs/` would report a stale matrix after
/// exactly the edit a reader came to check.
///
/// `status --json` is Rust since 2026-10-02 (tools/sjel-cli/src/harnesses/), reached through
/// the launcher like every other tool this process runs. It is the same reader the doctor and
/// `tools/harnesses status` use, so the page, the doctor and the CLI cannot disagree.
pub(crate) async fn packs_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/harnesses"))
        .arg("status")
        .arg("--json")
        .current_dir(&root)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/harnesses: {e}")))?;
    if !out.status.success() {
        return Err(bad_gateway(format!(
            "tools/harnesses failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map(Json)
        .map_err(|e| bad_gateway(format!("tools/harnesses did not emit JSON: {e}")))
}

/// Body of `POST /api/sjel-status/packs`.
#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct PackWriteRequest {
    pub(crate) action: String,
    pub(crate) harness: String,
    #[serde(default)]
    pub(crate) packs: Vec<String>,
}

/// Deploy a Pack, sync what a harness already holds, or remove it.
///
/// One route rather than three because every case turns on the same two facts — which harness,
/// which Packs — and the verbs differ only in the word handed to the launcher.
pub(crate) async fn packs_write_handler(
    Json(request): Json<PackWriteRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Validation before the environment lookup or any filesystem read: a request this surface
    // will not act on should not depend on how this host is configured to find out.
    let (tool, args) = plan_pack_write(&request.action, request.harness.trim(), &request.packs)
        .map_err(bad_request)?;
    let root = axon_root().map_err(bad_gateway)?;

    // A registry harness may have no per-harness launcher at all — sjel-agent is declared in its
    // own overlay config and reached with `tools/harnesses use`. Saying so is the answer; spawning
    // a missing binary and reporting ENOENT is not.
    if !root.join(&tool).exists() {
        return Err(bad_request(format!(
            "no deploy launcher for harness '{}' — {tool} does not exist; its Pack set is declared in that harness's own config",
            request.harness.trim()
        )));
    }

    let out = tokio::process::Command::new(root.join(&tool))
        .args(&args)
        .current_dir(&root)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run {tool}: {e}")))?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        // A refusal from the launcher is the caller's input: a drifted copy, a name another
        // Pack already owns, a Pack that is not deployed. Same reading as `upstreams/watch`.
        return Err(bad_request(if stderr.is_empty() { stdout } else { stderr }));
    }
    Ok(Json(json!({
        "action": request.action,
        "harness": request.harness.trim(),
        "packs": request.packs,
        "detail": stdout,
    })))
}

/// Which launcher a request means, as a path relative to `SJEL_ROOT` plus its argv.
///
/// Per-harness launchers for every verb, not `tools/harnesses`: that tool refuses a registry
/// harness with a hint rather than a sync (measured 2026-10-13 — `tools/harnesses sync harness`
/// answers "registry harness — run: tools/packs-pi deploy harness" for pi), so routing `sync`
/// through it would leave the pi column's buttons doing nothing. Every `tools/packs-*` launcher
/// carries `deploy`, `sync` and `remove` with the same shape.
///
/// Split out from the handler so the mapping is asserted without running a deployer: a test that
/// executed one would deploy Packs as a side effect of the test run.
fn plan_pack_write(
    action: &str,
    harness: &str,
    packs: &[String],
) -> Result<(String, Vec<String>), String> {
    plain_name(harness, "harness")?;
    for pack in packs {
        plain_name(pack, "pack")?;
    }
    match action {
        "deploy" | "sync" | "remove" => {
            if packs.is_empty() {
                return Err(format!("{action} needs at least one Pack"));
            }
            let mut args = vec![action.to_string()];
            args.extend(packs.iter().cloned());
            Ok((format!("tools/packs-{harness}"), args))
        }
        other => Err(format!("unknown action '{other}' — deploy, sync or remove")),
    }
}

/// A name that reaches an argv, and for deploy and remove one that also reaches a path under
/// `tools/`. The first character is alphanumeric so a leading `-` cannot be read as an option
/// by the launcher, and the rest cannot leave the directory or the argument.
pub(crate) fn plain_name(name: &str, what: &str) -> Result<(), String> {
    let head = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let tail = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !head || !tail {
        return Err(format!(
            "{what} '{name}' is not a plain name (letters, digits, '-' and '_', no leading dash)"
        ));
    }
    Ok(())
}

/// A refusal from the launcher is the caller's input, not a broken host — the same reading
/// `upstreams/watch` gives its own tool.
pub(crate) fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.into() })),
    )
}

#[cfg(test)]
mod write_tests {
    use super::*;

    fn packs(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// Every verb goes to the harness's own launcher. Routing `sync` through
    /// `tools/harnesses` was the first attempt and is wrong for a registry harness: that tool
    /// prints a hint instead of syncing, so the pi column's buttons would do nothing.
    #[test]
    fn every_verb_reaches_the_harness_own_launcher() {
        let (tool, args) = plan_pack_write("deploy", "claude", &packs(&["coding"])).unwrap();
        assert_eq!(tool, "tools/packs-claude");
        assert_eq!(args, vec!["deploy", "coding"]);

        let (tool, args) = plan_pack_write("sync", "pi", &packs(&["harness"])).unwrap();
        assert_eq!(tool, "tools/packs-pi");
        assert_eq!(args, vec!["sync", "harness"]);

        let (tool, args) = plan_pack_write("remove", "codex", &packs(&["writing"])).unwrap();
        assert_eq!(tool, "tools/packs-codex");
        assert_eq!(args, vec!["remove", "writing"]);

        let (_, args) = plan_pack_write("sync", "claude", &packs(&["coding", "writing"])).unwrap();
        assert_eq!(args, vec!["sync", "coding", "writing"]);
    }

    /// A name that reaches an argv must not be able to become an option or a path.
    #[test]
    fn a_name_that_could_escape_its_argument_is_refused() {
        for bad in ["../../etc", "--all", "", "-x", "co ding", "cod/ing"] {
            assert!(
                plan_pack_write("deploy", "claude", &packs(&[bad])).is_err(),
                "pack '{bad}' must be refused"
            );
        }
        for bad in ["../claude", "--all", ""] {
            assert!(
                plan_pack_write("deploy", bad, &packs(&["coding"])).is_err(),
                "harness '{bad}' must be refused"
            );
        }
        assert_eq!(plain_name("3d-printing", "pack"), Ok(()));
        assert_eq!(plain_name("svelte-core-bestpractices", "pack"), Ok(()));
    }

    /// An action with nothing to act on says so, rather than running a launcher with no
    /// arguments and reporting whatever it prints.
    #[test]
    fn an_action_with_nothing_to_do_is_refused() {
        assert!(plan_pack_write("deploy", "claude", &[]).is_err());
        assert!(plan_pack_write("sync", "claude", &[]).is_err());
        assert!(plan_pack_write("nonsense", "claude", &packs(&["coding"])).is_err());
    }
}

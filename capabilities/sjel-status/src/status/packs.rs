use super::*;

/// What Axon has deployed into each agent harness on this machine.
///
/// ## Why this capability serves another capability's state
///
/// `packs` is `kind = "data"`: it owns the deployment ledgers and nothing starts, so it
/// cannot serve them and something that is always up has to. That is this process, for
/// the same reason it publishes `host-watch`'s findings and `backup.sh`'s receipts — both
/// written by things with no port. Ownership does not move with the surface: `packs` owns
/// the ledgers, `tools/lib/pack-deploy.ts` owns their format, and this reads and never
/// writes.
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
pub(crate) async fn packs_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/harnesses.ts"))
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

use super::*;

/// What fills this machine's disk, straight from `tools/storage report --json`.
///
/// ## Why this capability serves a tool's report
///
/// `tools/storage` is operator machinery with no server: it measures, prints and exits.
/// The dashboard's Systems page is where a person asks "what is filling me", and
/// something always up has to answer, which is this process — the same arrangement
/// `/packs` and `/host-watch` already have, and `tools/host-watch` reads this very
/// tool the same way. The measurement stays with the tool: re-deriving the `du` walks
/// here would be a second answer to a question that already has one, and the two would
/// disagree the first time a class was added to the overlay's policy.
///
/// ## Why a non-zero exit is data, not an error
///
/// `report` exits 1 when free space is below the policy's critical threshold. That is the
/// loudest thing it can say, not a failure — refusing the answer here would blank the
/// panel on exactly the machine that most needs to see it. So stdout is parsed first and
/// the exit code is only consulted when there is nothing to parse, which is the same rule
/// `tools/host-watch` applies when it runs this tool.
///
/// The launcher, not the binary: `tools/storage/storage` sources `tools/lib/paths.sh`,
/// which is what resolves the overlay the policy lives in, and builds the release binary
/// on the first run after a checkout.
pub(crate) async fn storage_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/storage/storage"))
        .arg("report")
        .arg("--json")
        .current_dir(&root)
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/storage: {e}")))?;
    interpret_tool_report("tools/storage", &out.stdout, &out.stderr, out.status.code())
        .map(Json)
        .map_err(bad_gateway)
}

/// The handler's whole decision lives in [`super::interpret_tool_report`], shared with
/// `updates.rs` because the rule is the same rule: a parseable report is the answer whatever
/// the exit code says, and the exit code and stderr are context for the failure when it is not.
#[cfg(test)]
mod tests {
    use super::*;

    /// The case that decides whether a full disk blanks its own panel. `report` exits 1
    /// when free space crosses the policy's critical threshold, and that is the report a
    /// reader most needs; treating the exit code as failure would serve a 502 instead.
    #[test]
    fn a_report_that_exits_over_threshold_is_still_the_answer() {
        let report = interpret_tool_report(
            "tools/storage",
            br#"{"state":"critical","disk":{"free":1}}"#,
            b"",
            Some(1),
        )
        .expect("an over-threshold report is data, not a failure");
        assert_eq!(report["state"], "critical");
    }

    /// The other half: no JSON at all is a real failure, and the message has to carry the
    /// tool's own stderr and exit code or it says nothing a reader can act on.
    #[test]
    fn unparseable_output_reports_the_exit_code_and_stderr() {
        let error = interpret_tool_report(
            "tools/storage",
            b"",
            b"storage: no policy at /nowhere\n",
            Some(2),
        )
        .expect_err("empty output is not a report");
        assert!(error.contains("exit 2"), "{error}");
        assert!(error.contains("no policy at /nowhere"), "{error}");
    }
}

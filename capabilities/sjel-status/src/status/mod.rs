use super::*;

mod agent;
mod backup;
mod health;
mod host_watch;
mod lifecycle;
mod links;
mod packs;
mod reaper;
mod registry;
mod runtime;
mod storage;
mod updates;
mod upstreams;

pub(crate) use agent::*;
pub(crate) use backup::*;
pub(crate) use health::*;
pub(crate) use host_watch::*;
pub(crate) use lifecycle::*;
pub(crate) use links::*;
pub(crate) use packs::*;
pub(crate) use reaper::*;
pub(crate) use registry::*;
pub(crate) use runtime::*;
pub(crate) use storage::*;
pub(crate) use updates::*;
pub(crate) use upstreams::*;

/// The rule every "serve a tool's own report" handler shares: a parseable report IS the answer
/// whatever the exit code says, and the exit code and stderr are context for the failure when it
/// is not. Two tools now exit non-zero over a report a reader most needs — `tools/storage report`
/// when free space crosses the policy's critical threshold, `tools/updates report` whenever
/// anything is stale — and a handler that treated either as an error would blank the panel on
/// exactly the machine that most needs to see it.
///
/// One function rather than one per tool: the rule is the same rule, and two copies of it would
/// disagree the first time one of them was fixed.
pub(crate) fn interpret_tool_report(
    tool: &str,
    stdout: &[u8],
    stderr: &[u8],
    exit_code: Option<i32>,
) -> Result<Value, String> {
    match serde_json::from_slice(stdout) {
        Ok(report) => Ok(report),
        Err(parse_error) => Err(format!(
            "{tool} did not emit JSON (exit {}): {} — {parse_error}",
            match exit_code {
                Some(code) => code.to_string(),
                None => "killed by signal".to_string(),
            },
            String::from_utf8_lossy(stderr).trim(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case that decides whether a panel blanks itself on the machine that most needs it.
    /// `tools/storage report` exits 1 when free space crosses the policy's critical threshold and
    /// `tools/updates report` exits 1 whenever anything is stale; both are the loudest thing the
    /// tool can say, not a failure.
    #[test]
    fn a_report_that_exits_non_zero_is_still_the_answer() {
        let report = interpret_tool_report(
            "tools/storage",
            br#"{"state":"critical","disk":{"free":1}}"#,
            b"",
            Some(1),
        )
        .expect("an over-threshold report is data, not a failure");
        assert_eq!(report["state"], "critical");

        let stale = interpret_tool_report(
            "tools/updates",
            br#"{"rows":[{"status":"stale"}]}"#,
            b"",
            Some(1),
        )
        .expect("a stale report is data, not a failure");
        assert_eq!(stale["rows"][0]["status"], "stale");
    }

    /// The other half: no JSON at all is a real failure, and the message has to name the tool,
    /// its stderr and its exit code or it says nothing a reader can act on.
    #[test]
    fn unparseable_output_reports_the_tool_the_exit_code_and_stderr() {
        let error = interpret_tool_report("tools/updates", b"", b"updates: no overlay\n", Some(2))
            .expect_err("empty output is not a report");
        assert!(error.contains("tools/updates"), "{error}");
        assert!(error.contains("exit 2"), "{error}");
        assert!(error.contains("no overlay"), "{error}");
    }
}

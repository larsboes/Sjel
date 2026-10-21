use super::*;
use std::path::Path;

/// What this machine still needs for a component, and what applying it would do.
///
/// ## Why this is a child process and not a re-implementation
///
/// `sjel setup` owns the probes, the `--confirm` guard and the idempotent writer
/// (`tools/sjel-cli/src/setup/`). Copying any of that here would create the second writer of a rule
/// that already has one, and the two would disagree the first time one of them moved — which on
/// this route means a dashboard button that writes something the CLI would have refused.
///
/// ## Where the credential goes
///
/// `SetupApplyRequest` carries a component, a URL, an interval and — since 2026-10-21 — the
/// primary's credential, because the alternative was telling the operator to open a terminal and
/// redirect a file into a command. Three rules keep that from becoming an access-log entry:
///
/// 1. It is never put in argv. The child gets it on stdin (`--token-stdin`), which is also the only
///    path the tool accepts one from.
/// 2. It is never echoed. This handler returns the tool's JSON, and the tool reports the setting by
///    name ("stored, value hidden") rather than by value.
/// 3. It is never logged. Nothing in this module or in `interpret_tool_report` prints a request
///    body, and a failure message quotes the tool's stderr, not the request.
///
/// The remaining exposure is the loopback hop itself: browser → this process on 127.0.0.1. That is
/// the same surface that already starts and stops capabilities, and it refuses cross-origin writes
/// (src/lib/inbound-auth.ts), so it is not a new reachability, only a new field.
///
/// ## Why `--json` and one document
///
/// The tool prints one JSON document per verb (`{ "components": [...] }`), so this handler parses
/// and forwards it unchanged. The dashboard renders what the CLI computed; it does not decide
/// anything.
const COMPONENT: &str = "turso";
/// A plan is filesystem-only; a verify reaches the primary with the tool's own 10 s timeout. The
/// margin covers a launcher that has to build `sjel-cli` on a machine that has not run it yet.
const SETUP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct SetupApplyRequest {
    /// Defaults to `turso`, which is the only component today.
    #[serde(default)]
    component: Option<String>,
    url: Option<String>,
    interval_secs: Option<u64>,
    /// The primary's credential. Write-only: it is piped to the tool's stdin and never returned.
    #[serde(default, skip_serializing)]
    token: Option<String>,
}

/// The plan and the verification, in one answer.
///
/// Two shell-outs per request is the honest cost of not duplicating the tool: the plan cannot see
/// whether a primary answers, and the verification cannot see what a consumer is built with. The
/// dashboard polls neither of them on a timer — both are asked for by a click, because a verify
/// leaves the machine.
pub(crate) async fn setup_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let plan = run_setup(&["plan", COMPONENT, "--json"])
        .await
        .map_err(bad_gateway)?;
    Ok(Json(plan))
}

pub(crate) async fn setup_verify_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // A failed check exits 1 and still prints the probes: that report is the answer, which is the
    // rule `interpret_tool_report` states for every tool this capability serves.
    run_setup(&["verify", COMPONENT, "--json"])
        .await
        .map(Json)
        .map_err(bad_gateway)
}

pub(crate) async fn setup_apply_handler(
    Json(request): Json<SetupApplyRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let component = request
        .component
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(COMPONENT);
    if component != COMPONENT {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!("'{component}' is not a component this route sets up; the one is {COMPONENT}"),
            })),
        ));
    }

    let mut args: Vec<&str> = vec!["apply", COMPONENT, "--confirm", "--json"];
    let url = request
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty());
    if let Some(url) = url {
        args.push("--url");
        args.push(url);
    }
    let interval = request.interval_secs.map(|secs| secs.to_string());
    if let Some(interval) = interval.as_deref() {
        args.push("--interval-secs");
        args.push(interval);
    }

    let token = request
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if token.is_some() {
        args.push("--token-stdin");
    }

    // `--confirm` is on the command line by construction: this route IS the confirmation, and it
    // exists only after the operator has read the plan in the UI.
    run_setup_with(&args, token)
        .await
        .map(Json)
        .map_err(bad_gateway)
}

/// Run one `sjel setup` verb and return its JSON.
async fn run_setup(args: &[&str]) -> Result<Value, String> {
    run_setup_with(args, None).await
}

/// [`run_setup`] with an optional value for the child's stdin.
///
/// `stdin_value` is the only way a credential leaves this process, and it leaves through a pipe:
/// never argv (visible in `ps`), never the environment, never this process's stdout.
async fn run_setup_with(args: &[&str], stdin_value: Option<&str>) -> Result<Value, String> {
    run_setup_at(&axon_root()?, args, stdin_value).await
}

/// The two functions above with the checkout handed in, so a test can run a fake `sjel` and check
/// what actually crossed the process boundary.
async fn run_setup_at(
    root: &Path,
    args: &[&str],
    stdin_value: Option<&str>,
) -> Result<Value, String> {
    let mut command = tokio::process::Command::new(root.join("sjel"));
    // `setup` is the command; each caller passes only the verb and its flags.
    command
        .arg("setup")
        .args(args)
        .current_dir(root)
        .stdin(if stdin_value.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // The launcher prints its lazy-build note on stderr; stdout is the document.
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run sjel setup: {e}"))?;
    if let Some(value) = stdin_value {
        use tokio::io::AsyncWriteExt as _;
        if let Some(mut stdin) = child.stdin.take() {
            // A closed pipe means the tool exited without reading — a refusal it already explained,
            // so the wait below reports that rather than a write error.
            let _ = stdin.write_all(value.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.shutdown().await;
        }
    }
    let output = tokio::time::timeout(SETUP_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "sjel setup {} did not answer within {}s",
                args.first().copied().unwrap_or(""),
                SETUP_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("could not run sjel setup: {e}"))?;
    interpret_tool_report(
        "sjel setup",
        &output.stdout,
        &output.stderr,
        output.status.code(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A submitted credential is accepted, and it never comes back out.
    ///
    /// The field exists because the alternative was telling the operator to run a command, but it
    /// is write-only: the type skips it when serializing, so no response, log line or schema dump
    /// carries the value. `run_setup_with` hands it to the child on stdin and nowhere else.
    #[test]
    fn a_submitted_credential_is_write_only() {
        let request: SetupApplyRequest = serde_json::from_value(serde_json::json!({
            "component": "turso",
            "url": "libsql://example.turso.io",
            "interval_secs": 120,
            "token": "primary-credential",
        }))
        .expect("the form body deserializes");
        assert_eq!(request.component.as_deref(), Some("turso"));
        assert_eq!(request.interval_secs, Some(120));
        assert_eq!(request.token.as_deref(), Some("primary-credential"));

        let echoed = serde_json::to_value(&request).expect("serializes");
        assert!(
            !echoed.to_string().contains("primary-credential"),
            "the credential must not come back out: {echoed}"
        );
        assert!(!echoed.to_string().contains("token"), "{echoed}");
    }

    /// The credential reaches the tool on stdin and nowhere else.
    ///
    /// Measured through a real child process, because the property is about a process boundary:
    /// the fake `sjel` reports its argv and its stdin, so an implementation that passed the value
    /// as an argument — where `ps` and a shell history can see it — fails here.
    #[tokio::test]
    async fn the_credential_reaches_the_tool_on_stdin_and_not_in_argv() {
        let dir = std::env::temp_dir().join(format!("sjel-status-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("sjel");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '{\"argv\":\"%s\",\"stdin\":\"%s\"}' \"$*\" \"$(cat)\"\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        let report = run_setup_at(
            &dir,
            &["apply", "turso", "--confirm", "--json", "--token-stdin"],
            Some("primary-credential"),
        )
        .await
        .expect("the fake tool answered");

        assert_eq!(report["stdin"], "primary-credential");
        let argv = report["argv"].as_str().unwrap_or_default();
        assert!(!argv.contains("primary-credential"), "argv: {argv}");
        assert!(argv.contains("--token-stdin"), "argv: {argv}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_absent_component_defaults_to_the_only_one() {
        let request: SetupApplyRequest = serde_json::from_value(serde_json::json!({}))
            .expect("an empty body is a plan-with-defaults");
        assert_eq!(request.component, None);
        assert_eq!(request.token, None);
    }
}

use super::*;

/// What this machine still needs for a component, and what applying it would do.
///
/// ## Why this is a child process and not a re-implementation
///
/// `sjel setup` owns the probes, the `--confirm` guard and the idempotent writer
/// (`tools/sjel-cli/src/setup/`). Copying any of that here would create the second writer of a rule
/// that already has one, and the two would disagree the first time one of them moved — which on
/// this route means a dashboard button that writes something the CLI would have refused.
///
/// ## Why the credential has no route
///
/// `SetupApplyRequest` carries a component, a URL and an interval. It deliberately has no token
/// field: a credential belongs on stdin of the CLI the operator runs, never in an HTTP body that a
/// browser, a reverse proxy and an access log each get a copy of. A replica cannot be set up from
/// this surface alone, and saying so is better than accepting the field.
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

    // `--confirm` is on the command line by construction: this route IS the confirmation, and it
    // exists only after the operator has read the plan in the UI.
    run_setup(&args).await.map(Json).map_err(bad_gateway)
}

/// Run one `sjel setup` verb and return its JSON.
async fn run_setup(args: &[&str]) -> Result<Value, String> {
    let root = axon_root()?;
    let mut command = tokio::process::Command::new(root.join("sjel"));
    // `setup` is the command; each caller passes only the verb and its flags.
    command
        .arg("setup")
        .args(args)
        .current_dir(&root)
        // The launcher prints its lazy-build note on stderr; stdout is the document.
        .kill_on_drop(true);
    let output = tokio::time::timeout(SETUP_TIMEOUT, command.output())
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

    /// The credential has no way in. A field added here would be the first place a token travels
    /// over HTTP, through a proxy and into a log, and this test is what makes that a deliberate
    /// commit rather than a convenient one.
    #[test]
    fn the_apply_body_carries_no_credential() {
        let body = serde_json::json!({
            "component": "turso",
            "url": "libsql://example.turso.io",
            "interval_secs": 120,
        });
        let request: SetupApplyRequest =
            serde_json::from_value(body).expect("a body without a credential deserializes");
        assert_eq!(request.component.as_deref(), Some("turso"));
        assert_eq!(request.interval_secs, Some(120));

        let rest = serde_json::to_value(SetupApplyRequest {
            component: None,
            url: None,
            interval_secs: None,
        })
        .expect("serializes");
        assert!(
            !rest.to_string().to_lowercase().contains("token"),
            "the request type must not grow a token field: {rest}"
        );
    }

    #[test]
    fn an_absent_component_defaults_to_the_only_one() {
        let request: SetupApplyRequest = serde_json::from_value(serde_json::json!({}))
            .expect("an empty body is a plan-with-defaults");
        assert_eq!(request.component, None);
    }
}

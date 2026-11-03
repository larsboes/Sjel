//! Operator runtime controls use the existing capability caller, including its agent-token
//! selection. Never resolve an owner credential here to get around an agent refusal.
use sjel_runtime::{Category, Selection};
use std::{
    collections::BTreeSet,
    path::Path,
    process::{Command, ExitCode},
};

const API: &str = "/api/sjel-status/runtime";

enum Action {
    Status,
    Mode(Selection),
    Allow(Category, bool),
}

fn parse(args: &[String]) -> Result<Action, String> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["status"] => Ok(Action::Status),
        ["mode", mode] => serde_json::from_value(serde_json::json!(mode))
            .map(Action::Mode)
            .map_err(|_| "mode must be auto, normal or on-the-go".into()),
        ["allow", category, enabled @ ("on" | "off")] => {
            serde_json::from_value(serde_json::json!(category))
                .map(|category| Action::Allow(category, *enabled == "on"))
                .map_err(|_| "unknown runtime category; run 'sjel help runtime'".into())
        }
        _ => Err(crate::help::RUNTIME.into()),
    }
}

fn call(root: &Path, method: &str, body: Option<String>) -> ExitCode {
    let mut args = vec![
        "call".into(),
        "sjel-status".into(),
        method.into(),
        API.into(),
    ];
    if let Some(body) = body {
        args.push(body);
    }
    args.extend(["--connect-timeout", "2", "--max-time", "15"].map(String::from));
    crate::capability::command(root, &args)
}

fn allow_update(
    status: serde_json::Value,
    category: Category,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    if status["configured"] != true {
        return Err(
            "Save a selection first with 'sjel runtime mode auto|normal|on-the-go'.".into(),
        );
    }
    let revision = status["revision"]
        .as_u64()
        .ok_or("runtime status has no valid revision")?;
    let mut allow: BTreeSet<Category> = serde_json::from_value(
        status
            .get("allow")
            .cloned()
            .ok_or("runtime status has no allow list")?,
    )
    .map_err(|e| format!("invalid runtime allow list: {e}"))?;
    if enabled {
        allow.insert(category);
    } else {
        allow.remove(&category);
    }
    Ok(serde_json::json!({ "allow": allow, "expected_revision": revision }))
}

fn run_action(root: &Path, action: Action) -> Result<ExitCode, String> {
    let body = match action {
        Action::Status => return Ok(call(root, "get", None)),
        Action::Mode(selection) => serde_json::json!({ "selection": selection }),
        Action::Allow(category, enabled) => {
            // Capture the existing caller's GET without duplicating its registry or auth rules.
            let out = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .args([
                    "capability",
                    "call",
                    "sjel-status",
                    "get",
                    API,
                    "--connect-timeout",
                    "2",
                    "--max-time",
                    "15",
                ])
                .output()
                .map_err(|e| format!("cannot read runtime status: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "cannot read runtime status: {} {}. Runtime controls require owner credentials; agent tokens are refused.",
                    String::from_utf8_lossy(&out.stdout).trim(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            let status = serde_json::from_slice(&out.stdout)
                .map_err(|e| format!("invalid runtime status: {e}"))?;
            allow_update(status, category, enabled)?
        }
    };
    let result = call(root, "post", Some(body.to_string()));
    if result != ExitCode::SUCCESS {
        eprintln!("Runtime controls are owner-only. Agent credentials cannot save changes; run this command from the operator's shell.");
    }
    Ok(result)
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    match parse(args).and_then(|action| run_action(root, action)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sjel runtime: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn validates_commands_before_any_request() {
        for words in [
            vec!["status"],
            vec!["mode", "auto"],
            vec!["mode", "normal"],
            vec!["mode", "on-the-go"],
            vec!["allow", "remote-models", "off"],
        ] {
            assert!(parse(&args(&words)).is_ok());
        }
        for words in [
            vec![],
            vec!["mode", "battery"],
            vec!["status", "extra"],
            vec!["allow", "remote-models", "true"],
            vec!["allow", "unknown", "on"],
        ] {
            assert!(parse(&args(&words)).is_err());
        }
    }

    #[test]
    fn switching_one_exception_preserves_the_others_and_never_changes_selection() {
        let update = allow_update(
            serde_json::json!({"configured": true, "revision": 7, "allow": ["transcription", "remote-models"]}),
            Category::RemoteModels,
            false,
        )
        .unwrap();
        assert_eq!(
            update,
            serde_json::json!({"allow": ["transcription"], "expected_revision": 7})
        );
        let update = allow_update(
            serde_json::json!({"configured": true, "revision": 8, "allow": ["transcription"]}),
            Category::RemoteModels,
            true,
        )
        .unwrap();
        assert_eq!(update["allow"].as_array().unwrap().len(), 2);
        assert_eq!(update["expected_revision"], 8);
        assert!(allow_update(
            serde_json::json!({"configured": true, "allow": []}),
            Category::RemoteModels,
            true
        )
        .is_err());
        assert!(allow_update(serde_json::json!({}), Category::RemoteModels, true).is_err());
        assert!(allow_update(
            serde_json::json!({"configured": false, "allow": []}),
            Category::RemoteModels,
            true
        )
        .is_err());
    }
}

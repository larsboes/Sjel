use std::{
    fs,
    io::Write,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mail-eval-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_comms-mail-evidence-eval"));
        command
            .env("SJEL_OVERLAY_ROOT", &self.0)
            .env("SJEL_PERSONAL_ROOT", &self.0)
            .env("SJEL_AGENT", "1");
        command
    }

    fn freeze(&self, path: &std::path::Path) -> std::process::Output {
        let review = serde_json::json!({
            "reviewed_by": "synthetic-test", "reviewed_at": "2026-11-16", "representation": "synthetic",
            "cases": [{"source": {"id":"case-1", "source_id":"mail:v1:thread:message", "data_class":"c1", "stage":"inventory", "mail":"Private footer sentinel"}, "expected": []}]
        });
        let mut child = self
            .command()
            .arg("freeze")
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(review.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn native_commands_refuse_agents_before_files_config_or_discovery() {
    let scratch = Scratch::new();
    for args in [
        vec!["freeze-native", "missing-parent/corpus.json"],
        vec![
            "run-native",
            "missing-corpus.json",
            "missing-parent/report.json",
        ],
    ] {
        let output = scratch
            .command()
            .env("PATH", "")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires an owner terminal"));
        assert!(fs::read_dir(&scratch.0).unwrap().next().is_none());
    }
}

#[test]
fn shared_artifact_writer_is_private_body_free_and_refuses_overwrite() {
    let scratch = Scratch::new();
    let path = scratch.0.join("corpus.json");
    let output = scratch.freeze(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let original = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&original).contains("Private footer sentinel"));
    assert_eq!(scratch.freeze(&path).status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn paths_outside_overlay_and_symlinked_parents_are_refused() {
    let scratch = Scratch::new();
    let outside = Scratch::new();
    let path = outside.0.join("corpus.json");
    assert_eq!(scratch.freeze(&path).status.code(), Some(2));
    assert!(!path.exists());
    symlink(&outside.0, scratch.0.join("escape")).unwrap();
    assert_eq!(
        scratch
            .freeze(&scratch.0.join("escape/corpus.json"))
            .status
            .code(),
        Some(2)
    );
    assert!(!path.exists());
}

#[test]
fn stdin_rules_workflow_still_produces_a_private_report_without_models() {
    let scratch = Scratch::new();
    let corpus = scratch.0.join("corpus.json");
    assert!(scratch.freeze(&corpus).status.success());
    let report = scratch.0.join("report.json");
    let sources = serde_json::json!([{
        "id": "case-1", "source_id": "mail:v1:thread:message", "stage": "inventory",
        "data_class": "c1", "mail": "Private footer sentinel"
    }]);
    let mut child = scratch
        .command()
        .env("SJEL_COMMS_CONFIG", scratch.0.join("absent.json"))
        .arg("run")
        .arg(corpus)
        .arg(&report)
        .arg("rules")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(sources.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(&report).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = fs::read(report).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["model"], "none");
    assert_eq!(report["cases"]["case-1"]["abstained"], true);
    assert!(!String::from_utf8_lossy(&bytes).contains("Private footer sentinel"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Private footer sentinel"));
}

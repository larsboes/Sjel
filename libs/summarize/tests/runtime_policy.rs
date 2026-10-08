use sjel_summarize::{ask, Admission, LocalGate, Outcome, Reach, Target};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn policy(root: &Path, selection: &str) {
    let path = root.join("config/runtime/test.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::json!({"selection":selection,"manual_power":"unknown","allow":[]}).to_string()).unwrap();
}

struct ChangePolicy {
    root: std::path::PathBuf,
    calls: Arc<AtomicUsize>,
}
impl LocalGate for ChangePolicy {
    fn acquire(&self) -> Result<Admission, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        policy(&self.root, "on-the-go");
        Ok(Admission::free())
    }
}

#[test]
fn runtime_child() {
    let Ok(root) = std::env::var("SJEL_RUNTIME_TEST_CHILD") else { return };
    let root = Path::new(&root);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut target = Target {
        backend_name: "other".into(), max_input_tokens: Some(4096),
        endpoint: "http://127.0.0.1:9/v1/chat/completions".into(), model: "small".into(), api_key: None,
        loopback: true, operator_owned: true,
        gate: Some(Arc::new(ChangePolicy { root: root.to_owned(), calls: Arc::clone(&calls) })),
    };
    // Policy is checked before queuing and again after a potentially long wait for admission.
    let blocked = ask(Some(&target), "summarize this", 100, Reach::LoopbackOnly);
    assert!(matches!(blocked, Outcome::PolicyDeferred(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(!blocked.retryable());
    assert_eq!(blocked.state(), "policy_deferred");
    policy(root, "normal");
    assert!(matches!(ask(Some(&target), "summarize this", 100, Reach::LoopbackOnly), Outcome::PolicyDeferred(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    target.backend_name = "foundation-models".into();
    target.model = "apple-foundationmodel".into();
    assert!(matches!(ask(Some(&target), &"x".repeat(4096), 100, Reach::LoopbackOnly), Outcome::PolicyDeferred(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 1, "oversized AFM work must not queue or send");
}

#[test]
fn cached_targets_obey_policy_changes_and_context_limits() {
    let root = std::env::temp_dir().join(format!("sjel-summarize-runtime-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    policy(&root, "on-the-go");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_child", "--nocapture"])
        .env("SJEL_RUNTIME_TEST_CHILD", &root)
        .env("SJEL_PERSONAL_ROOT", &root)
        .env("SJEL_MACHINE_TOML", root.join("config/machines/test.toml"))
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(root).unwrap();
}

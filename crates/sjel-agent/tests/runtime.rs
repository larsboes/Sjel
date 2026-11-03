use sjel_agent::{Agent, AgentError, Message};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn runtime_child() {
    if std::env::var_os("SJEL_RUNTIME_TEST_CHILD").is_none() {
        return;
    }
    let config: sjel_inference::InferenceConfig = r#"{
        "backends":{"other":{"api":"openai","base_url":"http://127.0.0.1:9/v1"}},
        "roles":{"agent":{"backend":"other","model":"small"}}
    }"#
    .parse()
    .unwrap();
    let agent = Agent::new(
        config.role("agent").unwrap(),
        Vec::new(),
        Vec::new(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let mut messages = vec![Message::User {
        content: "Hi".into(),
    }];
    let error = agent.run(&mut messages, |_| {}).unwrap_err();
    assert!(matches!(error, AgentError::RequestRefused(_)));
    assert!(error
        .to_string()
        .starts_with("deferred by runtime profile:"));
    assert_eq!(messages.len(), 1);
}

#[test]
fn a_cached_agent_role_cannot_bypass_the_device_profile() {
    let root = std::env::temp_dir().join(format!(
        "sjel-agent-runtime-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("config/runtime")).unwrap();
    std::fs::write(
        root.join("config/runtime/test.json"),
        r#"{"selection":"on-the-go","allow":[]}"#,
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_child", "--nocapture"])
        .env("SJEL_RUNTIME_TEST_CHILD", &root)
        .env("SJEL_PERSONAL_ROOT", &root)
        .env("SJEL_MACHINE_TOML", root.join("config/machines/test.toml"))
        .env_remove("SJEL_INFERENCE_BACKEND")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

use std::collections::HashMap;
use sjel_containment::{
    sanitize_environment, ContainedCommand, EgressPolicy, EgressProxy, ProvidersFile, Ring,
    SeatbeltProfile,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn ring_classification_follows_doctrine() {
    let toml = r#"
[openai]
highest_data_class = "c1"
reviewed_at = "2026-08-25"
why = "OpenAI zero-retention"

[deepseek]
highest_data_class = "c0"
reviewed_at = "2026-10-24"
why = "DeepSeek public only"
"#;
    let file: ProvidersFile = toml::from_str(toml).unwrap();
    assert_eq!(file.classify_ring("openai"), Ring::Trusted);
    assert_eq!(file.classify_ring("deepseek"), Ring::Contained);
    assert_eq!(file.classify_ring("unknown-provider"), Ring::Contained);
}

#[test]
fn seatbelt_profile_contains_deny_default_and_project_paths() {
    let tmp = std::env::temp_dir().join("sjel-containment-test");
    let _ = std::fs::create_dir_all(&tmp);
    let profile = SeatbeltProfile::new(&tmp).render();

    assert!(profile.contains("(version 1)"));
    assert!(profile.contains("(deny default)"));
    assert!(profile.contains("file-read* file-write*"));
    assert!(profile.contains(&tmp.to_string_lossy().to_string()) || profile.contains("/private"));
    assert!(profile.contains("(remote tcp \"localhost:*\")"));
}

#[test]
fn environment_sanitization_removes_sensitive_secrets() {
    let mut vars = HashMap::new();
    vars.insert("PATH".into(), "/usr/bin".into());
    vars.insert("HOME".into(), "/Users/test".into());
    vars.insert("SSH_AUTH_SOCK".into(), "/tmp/ssh.sock".into());
    vars.insert("GITHUB_TOKEN".into(), "ghp_secret".into());
    vars.insert("OPENAI_API_KEY".into(), "sk-secret".into());
    vars.insert("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into());

    let cleaned = sanitize_environment(vars, None);

    assert_eq!(cleaned.get("PATH"), Some(&"/usr/bin".to_string()));
    assert_eq!(cleaned.get("HOME"), Some(&"/Users/test".to_string()));
    assert!(!cleaned.contains_key("SSH_AUTH_SOCK"));
    assert!(!cleaned.contains_key("GITHUB_TOKEN"));
    assert!(!cleaned.contains_key("OPENAI_API_KEY"));
    assert!(!cleaned.contains_key("AWS_SECRET_ACCESS_KEY"));
}

#[test]
fn egress_policy_admits_package_registries_and_refuses_unauthorized_hosts() {
    let policy = EgressPolicy::new().with_provider_host("api.deepseek.com");

    assert!(policy.is_host_allowed("crates.io"));
    assert!(policy.is_host_allowed("static.crates.io"));
    assert!(policy.is_host_allowed("registry.npmjs.org"));
    assert!(policy.is_host_allowed("api.deepseek.com"));
    assert!(policy.is_host_allowed("127.0.0.1"));

    assert!(!policy.is_host_allowed("attacker.com"));
    assert!(!policy.is_host_allowed("unreviewed-external.net"));
}

#[tokio::test]
async fn egress_proxy_refuses_unauthorized_destination() {
    let policy = EgressPolicy::new().with_provider_host("api.deepseek.com");
    let proxy = EgressProxy::bind(policy).await.unwrap();
    let addr = proxy.local_addr();
    let audit = proxy.audit();

    tokio::spawn(async move {
        let _ = proxy.run().await;
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"CONNECT forbidden-exfiltration.com:443 HTTP/1.1\r\nHost: forbidden-exfiltration.com:443\r\n\r\n")
        .await
        .unwrap();

    let mut response = [0u8; 1024];
    let n = stream.read(&mut response).await.unwrap();
    let resp_str = String::from_utf8_lossy(&response[..n]);

    assert!(resp_str.starts_with("HTTP/1.1 403 Forbidden"));
    assert_eq!(audit.refused_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(audit.refusals.read().await.as_slice(), &["forbidden-exfiltration.com"]);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_sandbox_exec_enforces_directory_and_file_boundaries() {
    let tmp = std::env::temp_dir().join(format!("sjel-seatbelt-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);

    let profile = SeatbeltProfile::new(&tmp);
    let project_file = tmp.join("allowed.txt");
    let mut cmd_allowed = ContainedCommand::new("/bin/sh", profile)
        .arg("-c")
        .arg(format!("echo 'ok' > '{}'", project_file.display()))
        .build_command();

    let output = cmd_allowed.output().expect("execute sandbox-exec");
    assert!(output.status.success(), "writing inside project should succeed");
    assert_eq!(std::fs::read_to_string(&project_file).unwrap().trim(), "ok");

    let denied_file = std::env::temp_dir()
        .join("..")
        .join(format!("sjel-denied-{}.txt", std::process::id()));
    let profile2 = SeatbeltProfile::new(&tmp);
    let mut cmd_denied = ContainedCommand::new("/bin/sh", profile2)
        .arg("-c")
        .arg(format!("echo 'bad' > '{}'", denied_file.display()))
        .build_command();

    let output_denied = cmd_denied.output().expect("execute sandbox-exec");
    assert!(!output_denied.status.success(), "writing outside project should fail");
    assert!(!denied_file.exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

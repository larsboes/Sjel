use sjel_containment::{
    classify_ring, provider_hosts, sanitize_environment, ContainedCommand, EgressPolicy,
    EgressProxy, Ring, SeatbeltProfile,
};
use sjel_inference::ReviewedProvidersList;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn ring_classification_follows_doctrine() {
    let toml = r#"
[openai]
highest_data_class = "c1"
reviewed_at = "2026-08-25"
ring = "trusted"
hosts = ["api.openai.com"]

[deepseek]
highest_data_class = "c0"
reviewed_at = "2026-10-24"
hosts = ["api.deepseek.com"]

[expired]
highest_data_class = "c1"
reviewed_at = "2024-01-01"
ring = "trusted"
"#;
    let list = ReviewedProvidersList::parse_toml(toml).unwrap();
    assert_eq!(classify_ring(&list, "openai", "2026-11-04"), Ring::Trusted);
    assert_eq!(
        classify_ring(&list, "deepseek", "2026-11-04"),
        Ring::Contained
    );
    assert_eq!(
        classify_ring(&list, "expired", "2026-11-04"),
        Ring::Contained
    );
    assert_eq!(
        classify_ring(&list, "unknown-provider", "2026-11-04"),
        Ring::Contained
    );
    assert_eq!(provider_hosts(&list, "deepseek"), vec!["api.deepseek.com"]);
    assert!(provider_hosts(&list, "unknown-provider").is_empty());
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
    assert!(profile.contains("(remote ip \"localhost:*\")"));
    // Each ancestor is a literal, so a walk with openat resolves and no file below one is read.
    assert!(profile.contains("(allow file-read-data (literal \"/\"))"));
    assert!(profile.contains("(allow file-read-data (literal \"/private\"))"));
    assert!(!profile.contains("(allow file-read* (subpath \"/private\"))"));
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
    assert!(
        !policy.is_host_allowed("127.0.0.1"),
        "the proxy never forwards to this machine"
    );
    assert!(!policy.is_host_allowed("localhost"));

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
    assert_eq!(
        audit
            .refused_count
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        audit.refusals.read().await.as_slice(),
        &["forbidden-exfiltration.com"]
    );
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
    assert!(
        output.status.success(),
        "writing inside project should succeed"
    );
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
    assert!(
        !output_denied.status.success(),
        "writing outside project should fail"
    );
    assert!(!denied_file.exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Plan item 1's done-when (docs/plans/containment.md §5): a sibling file is unreadable and a
/// write outside the project fails, while a walk through the ancestors lists their names, a
/// project write succeeds and `cargo test` passes. Decision C6 admits the ancestors.
#[cfg(target_os = "macos")]
#[test]
fn macos_sandbox_admits_ancestor_names_and_refuses_their_contents() {
    let base = std::env::temp_dir().join(format!("sjel-seatbelt-read-{}", std::process::id()));
    let project = base.join("project");
    let outside = base.join("sibling");
    std::fs::create_dir_all(project.join("probe/src")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("canary"), "canary").unwrap();
    std::fs::write(
        project.join("probe/Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        project.join("probe/src/lib.rs"),
        "#[test]\nfn adds() { assert_eq!(1 + 1, 2); }\n",
    )
    .unwrap();

    let home = std::env::var("HOME").unwrap();
    let cargo_bin = format!("{home}/.cargo/bin");
    let run = |script: &str| {
        let mut cmd = ContainedCommand::new("/bin/sh", SeatbeltProfile::new(&project))
            .arg("-c")
            .arg(script)
            .build_command();
        cmd.env("PATH", format!("{cargo_bin}:/usr/bin:/bin"))
            .env("TMPDIR", project.join(".tmp"))
            .env("CARGO_TARGET_DIR", project.join("target"));
        cmd.output().expect("execute sandbox-exec")
    };
    std::fs::create_dir_all(project.join(".tmp")).unwrap();

    let sibling = run(&format!("cat '{}'", outside.join("canary").display()));
    assert!(
        !sibling.status.success(),
        "a sibling file must not be readable"
    );

    let listing = run("ls \"$HOME\"");
    assert!(
        listing.status.success(),
        "an ancestor's names are listable so a path walk resolves"
    );
    let inside_home = run(&format!(
        "ls '{}/.ssh' 2>/dev/null | head -1",
        std::env::var("HOME").unwrap()
    ));
    assert!(
        !inside_home.status.success() || inside_home.stdout.is_empty(),
        "a directory below an unopened ancestor must stay closed"
    );

    let write = run(&format!("echo ok > '{}'", project.join("w.txt").display()));
    assert!(write.status.success(), "a project write must succeed");

    if std::path::Path::new(&cargo_bin).join("cargo").exists() {
        let test = run(&format!(
            "cd '{}' && cargo test -q --offline",
            project.join("probe").display()
        ));
        assert!(
            test.status.success(),
            "cargo test in the project must pass: {}",
            String::from_utf8_lossy(&test.stderr)
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}

/// The request head ends at the blank line. Header bytes after the request line must not
/// reach the target, or a TLS handshake through the tunnel fails.
#[tokio::test]
async fn egress_proxy_tunnels_only_the_bytes_after_the_request_head() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_addr = target.local_addr().unwrap();
    let proxy = EgressProxy::bind(EgressPolicy::new().with_provider_host("127.0.0.1"))
        .await
        .unwrap();
    let proxy_addr = proxy.local_addr();
    tokio::spawn(async move {
        let _ = proxy.run().await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(
            format!(
                "CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\nUser-Agent: probe\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let (mut upstream, _) = target.accept().await.unwrap();
    let mut established = [0u8; 39];
    client.read_exact(&mut established).await.unwrap();
    assert!(established.starts_with(b"HTTP/1.1 200"));

    client.write_all(b"payload").await.unwrap();
    let mut got = [0u8; 7];
    upstream.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"payload");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_sandbox_leaves_the_keychain_to_the_harness() {
    let tmp = std::env::temp_dir().join(format!("sjel-seatbelt-keychain-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let profile = SeatbeltProfile::new(&tmp).render();
    // Decision C3: the broker stays reachable. The question about a missing item is answered by
    // the store, not the sandbox; an item whose ACL trusts only another binary stays refused.
    assert!(!profile.contains("com.apple.SecurityServer"));
    let output = ContainedCommand::new("/usr/bin/security", SeatbeltProfile::new(&tmp))
        .args(["find-generic-password", "-s", "sjel-containment-probe-item"])
        .build_command()
        .output()
        .expect("run security inside the profile");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Operation not permitted"),
        "the Keychain answers a contained run: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_run_is_pointed_at_the_roots_the_launcher_exported() {
    let cmd = ContainedCommand::new("/bin/true", SeatbeltProfile::new(std::env::temp_dir()))
        .with_ca_bundle(std::path::Path::new("/tmp/roots.pem"));
    for name in sjel_containment::CA_VARIABLES {
        assert_eq!(cmd.env[*name], "/tmp/roots.pem", "{name}");
    }
}

#[test]
fn a_contained_run_points_at_the_demo_node() {
    let cmd = ContainedCommand::new("/bin/true", SeatbeltProfile::new(std::env::temp_dir()))
        .with_dev_node("http://127.0.0.1:18082");
    assert_eq!(cmd.env["SJEL_STATUS_URL"], "http://127.0.0.1:18082");
    assert_eq!(cmd.env["SJEL_ENV"], "dev");
    assert_eq!(cmd.env["NO_PROXY"], "127.0.0.1,localhost");
}

/// `--read`, `--write` and `--keep-env` through the real launcher: the read directory is
/// readable but not writable, the write directory is writable, and only the named variable
/// survives the sanitizer.
#[cfg(target_os = "macos")]
#[test]
fn the_launcher_opens_only_the_named_directories_and_variable() {
    let base = std::env::temp_dir().join(format!("sjel-contain-flags-{}", std::process::id()));
    let (project, ro, rw) = (base.join("project"), base.join("ro"), base.join("rw"));
    for dir in [&project, &ro, &rw] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ro.join("config"), "cfg").unwrap();
    let script = format!(
        "cat '{ro}/config'; echo; echo s > '{rw}/session' && echo write-ok; \
         echo x > '{ro}/x' 2>/dev/null || echo ro-denied; \
         echo \"keep=${{KEEP_PROBE:-unset}} drop=${{DROP_PROBE_TOKEN:-unset}} set=${{SET_PROBE:-unset}}\"",
        ro = ro.display(),
        rw = rw.display()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sjel-contain"))
        .args(["/bin/sh", "--provider", "unlisted", "--project"])
        .arg(&project)
        .arg("--read")
        .arg(&ro)
        .arg("--write")
        .arg(&rw)
        .args(["--keep-env", "KEEP_PROBE", "--env", "SET_PROBE=named", "--"])
        .args(["-c", &script])
        .env("KEEP_PROBE", "kept")
        .env("DROP_PROBE_TOKEN", "dropped")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cfg"), "{stdout}");
    assert!(stdout.contains("write-ok"), "{stdout}");
    assert!(stdout.contains("ro-denied"), "{stdout}");
    assert!(
        stdout.contains("keep=kept drop=unset set=named"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// `--credential` resolves in the launcher, before the proxy or the harness start. A slug the
/// private catalogue does not hold stops the run, and the harness never runs.
#[test]
fn a_credential_the_catalogue_cannot_resolve_stops_the_run() {
    let project = std::env::temp_dir().join(format!("sjel-contain-cred-{}", std::process::id()));
    std::fs::create_dir_all(&project).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sjel-contain"))
        .args(["/bin/sh", "--provider", "unlisted", "--project"])
        .arg(&project)
        .args(["--credential", "no-such-provider", "--", "-c", "echo ran"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(!stdout.contains("ran"), "{stdout}");
    assert!(
        stderr.contains("--credential no-such-provider: no inference-no-such-provider credential"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&project);
}

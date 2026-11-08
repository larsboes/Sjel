use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("sjel-capability-cli-{}", std::process::id()));
        fs::create_dir_all(root.join("tools/lib")).unwrap();
        fs::create_dir_all(root.join("crates/example")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(
            root.join("tools/lib/paths.sh"),
            r#"
SJEL_ROOT="$1"
SJEL_OVERLAY_ROOT="$1/overlay"
SJEL_MACHINE_TOML="$1/overlay/config/machine.toml"
SJEL_CAPS_DIR="$1/capabilities"
SJEL_OVERLAY_CAPS_DIR="$1/overlay/capabilities"
"#,
        )
        .unwrap();
        fs::write(
            root.join("crates/example/service.toml"),
            "kind = 'process'\n",
        )
        .unwrap();
        fs::write(
            root.join("crates/example/Cargo.toml"),
            "[[bin]]\nname = 'example-server'\n[[bin]]\nname = 'example-cli'\n",
        )
        .unwrap();
        let cargo = root.join("bin/cargo");
        fs::write(&cargo, "#!/bin/sh\nprintf '%s\\000' \"$SJEL_PERSONAL_ROOT\" \"$SJEL_MACHINE_TOML\" \"$@\"\nexit 17\n").unwrap();
        fs::set_permissions(cargo, fs::Permissions::from_mode(0o755)).unwrap();
        Self(root)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sjel-cli"));
        command
            .env("SJEL_ROOT", &self.0)
            .env("SJEL_ALLOW_LOCAL_BUILD", "1")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.0.join("bin").display()),
            );
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn dispatch_preserves_arguments_deployment_and_child_status_and_refuses_unknowns() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args([
            "example",
            "put",
            "A title; $(literal)",
            "two\nlines",
            "--flag",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(17),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let words: Vec<_> = stdout.split_terminator('\0').collect();
    assert_eq!(
        &words[..2],
        [
            fixture.0.join("overlay").to_str().unwrap(),
            fixture
                .0
                .join("overlay/config/machine.toml")
                .to_str()
                .unwrap()
        ]
    );
    assert_eq!(
        &words[2..],
        [
            "run",
            "--locked",
            "--manifest-path",
            fixture
                .0
                .join("crates/example/Cargo.toml")
                .to_str()
                .unwrap(),
            "--bin",
            "example-cli",
            "--",
            "put",
            "A title; $(literal)",
            "two\nlines",
            "--flag"
        ]
    );

    for command in ["absent", "../example"] {
        let output = fixture.command().arg(command).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command"));
    }
    fs::create_dir_all(fixture.0.join("overlay/capabilities/example")).unwrap();
    fs::write(
        fixture.0.join("overlay/capabilities/example/service.toml"),
        "kind = 'process'\n",
    )
    .unwrap();
    let duplicate = fixture.command().arg("example").output().unwrap();
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(duplicate.stdout.is_empty());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("both"));
}

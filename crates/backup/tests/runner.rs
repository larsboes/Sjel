use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, script: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sjel-backup-runner-{}-{name}", std::process::id()));
        fs::create_dir_all(root.join("tools")).unwrap();
        let path = root.join("tools/backup.sh");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        Self(root)
    }

    fn run(&self) -> backup::runner::Outcome {
        backup::runner::run(&self.0, &self.0.join("overlay"), "store", "local")
    }

    fn attempt(&self) -> serde_json::Value {
        serde_json::from_slice(
            &fs::read(self.0.join("overlay/backup/receipts/attempts/store.json")).unwrap(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn a_zero_exit_without_a_new_receipt_is_a_failed_attempt() {
    let fixture = Fixture::new("missing", "exit 0");
    let outcome = fixture.run();
    assert_ne!(outcome.exit_code, 0);
    assert!(outcome.archive.is_none());
    assert!(outcome.detail.contains("without a new archive receipt"));
    assert_eq!(fixture.attempt()["exit_code"], outcome.exit_code);
}

#[test]
fn an_old_receipt_cannot_turn_a_noop_into_a_success() {
    let fixture = Fixture::new("old", "exit 0");
    let history = fixture.0.join("overlay/backup/receipts/history/store");
    fs::create_dir_all(&history).unwrap();
    fs::write(
        history.join("store-old.tar.gz.json"),
        r#"{"sha256":"old-digest","bytes":10,"tarball":"store-old.tar.gz"}"#,
    )
    .unwrap();
    assert_ne!(fixture.run().exit_code, 0);
    assert_eq!(fixture.attempt()["capability"], "store");
}

#[test]
fn a_missing_target_failure_retains_its_reason_in_the_attempt() {
    let fixture = Fixture::new(
        "target",
        "echo \"backup.sh: target 'local' path does not exist: /absent\" >&2\n\
         echo 'Attach the volume it lives on.' >&2\nexit 1",
    );
    let outcome = fixture.run();
    assert_eq!(outcome.exit_code, 1);
    assert_eq!(
        outcome.detail,
        "backup.sh: target 'local' path does not exist: /absent"
    );
    let attempt = fixture.attempt();
    assert_eq!(attempt["target"], "local");
    assert_eq!(attempt["detail"], outcome.detail);
    assert_eq!(attempt["exit_code"], 1);
}

#[test]
fn a_new_receipt_records_success_and_clears_the_failed_attempt() {
    let fixture = Fixture::new(
        "new",
        r#"
history="$(dirname "$0")/../overlay/backup/receipts/history/store"
mkdir -p "$history"
printf '%s\n' '{"sha256":"new-digest","bytes":10,"tarball":"store-new.tar.gz"}' > "$history/store-new.tar.gz.json"
"#,
    );
    let attempt = fixture
        .0
        .join("overlay/backup/receipts/attempts/store.json");
    fs::create_dir_all(attempt.parent().unwrap()).unwrap();
    fs::write(&attempt, "{}").unwrap();
    let outcome = fixture.run();
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.archive.unwrap().name, "store-new.tar.gz");
    assert!(!attempt.exists());
}

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

const ADAPTERS: [(&str, &str, &str); 4] = [
    ("claude", "CLAUDE_SKILLS_DIR", "SJEL_CLAUDE_STATE_FILE"),
    ("codex", "CODEX_SKILLS_DIR", "SJEL_CODEX_STATE_FILE"),
    (
        "opencode",
        "OPENCODE_SKILLS_DIR",
        "SJEL_OPENCODE_PACKS_STATE_FILE",
    ),
    ("copilot", "COPILOT_SKILLS_DIR", "SJEL_COPILOT_STATE_FILE"),
];

struct Fixture {
    root: PathBuf,
    adapter: &'static str,
    destination_env: &'static str,
    state_env: &'static str,
}

impl Fixture {
    fn new(adapter: (&'static str, &'static str, &'static str)) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sjel-packs-sync-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        for dir in ["home", "state", "overlay", "config", "dest"] {
            fs::create_dir(root.join(dir)).unwrap();
        }
        let fixture = Self {
            root,
            adapter: adapter.0,
            destination_env: adapter.1,
            state_env: adapter.2,
        };
        for pack in ["first", "second"] {
            let dir = fixture.root.join("Packs").join(pack);
            fs::create_dir_all(dir.join("skills").join(pack)).unwrap();
            fs::write(
                dir.join("pack.toml"),
                format!("name = '{pack}'\ndescription = 'test'\nskills = ['{pack}']\n"),
            )
            .unwrap();
            fixture.update_source(pack, "initial");
        }
        fixture.success(&["deploy", "first", "second"]);
        for pack in ["first", "second"] {
            fixture.assert_deployed(pack, "initial");
        }
        assert!(fixture.root.join("ledger.json").is_file());
        fixture
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sjel-cli"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("SJEL_ROOT", &self.root)
            .env("HOME", self.root.join("home"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("SJEL_OVERLAY_ROOT", self.root.join("overlay"))
            .env(self.destination_env, self.root.join("dest"))
            .env(self.state_env, self.root.join("ledger.json"))
            .env("CLAUDE_AGENTS_DIR", self.root.join("agents"))
            .current_dir(&self.root)
            .arg(format!("packs-{}", self.adapter))
            .args(args)
            .output()
            .unwrap()
    }

    fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{} {args:?}: {output:?}",
            self.adapter
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn source(&self, pack: &str) -> PathBuf {
        self.root
            .join(format!("Packs/{pack}/skills/{pack}/SKILL.md"))
    }

    fn skill(pack: &str, body: &str) -> String {
        format!("---\nname: {pack}\ndescription: test skill\n---\n\n{body}\n")
    }

    fn update_source(&self, pack: &str, body: &str) {
        fs::write(self.source(pack), Self::skill(pack, body)).unwrap();
    }

    fn assert_deployed(&self, pack: &str, body: &str) {
        assert_eq!(
            fs::read_to_string(self.root.join(format!("dest/{pack}/SKILL.md"))).unwrap(),
            Self::skill(pack, body),
            "{}: {pack}",
            self.adapter
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn sync_updates_every_explicit_pack_with_labeled_success_output() {
    for adapter in ADAPTERS {
        let fixture = Fixture::new(adapter);
        for pack in ["first", "second"] {
            fixture.update_source(pack, "updated");
        }
        let stdout = fixture.success(&["sync", "first", "second"]);
        for pack in ["first", "second"] {
            fixture.assert_deployed(pack, "updated");
            assert!(stdout.lines().any(|line| line == pack), "{stdout}");
            assert!(stdout.contains(&format!("{pack} synced")), "{stdout}");
        }
    }
}

#[test]
fn sync_reports_a_later_failure_and_continues_to_a_subsequent_pack() {
    for adapter in ADAPTERS {
        let fixture = Fixture::new(adapter);
        for pack in ["first", "second"] {
            fixture.update_source(pack, "updated");
        }
        let output = fixture.run(&["sync", "first", "missing", "second"]);
        assert!(!output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("missing"), "{stderr}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        for pack in ["first", "second"] {
            fixture.assert_deployed(pack, "updated");
            assert!(stdout.lines().any(|line| line == pack), "{stdout}");
        }
    }
}

#[test]
fn sync_all_refreshes_all_deployed_packs() {
    for adapter in ADAPTERS {
        let fixture = Fixture::new(adapter);
        for pack in ["first", "second"] {
            fixture.update_source(pack, "updated");
        }
        fixture.success(&["sync", "--all"]);
        for pack in ["first", "second"] {
            fixture.assert_deployed(pack, "updated");
        }
    }
}

#[test]
fn sync_rejects_all_mixed_with_names_before_writing() {
    for adapter in ADAPTERS {
        let fixture = Fixture::new(adapter);
        for pack in ["first", "second"] {
            fixture.update_source(pack, "updated");
        }
        let ledger = fs::read(fixture.root.join("ledger.json")).unwrap();
        for args in [
            vec!["sync", "--all", "first", "second"],
            vec!["sync", "first", "--all", "second"],
            vec!["sync", "first", "second", "--all"],
            vec!["sync", "--all", "--all"],
            vec!["sync"],
        ] {
            let output = fixture.run(&args);
            assert!(!output.status.success(), "{args:?}: {output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("--all"), "{stderr}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert_eq!(fs::read(fixture.root.join("ledger.json")).unwrap(), ledger);
            for pack in ["first", "second"] {
                fixture.assert_deployed(pack, "initial");
            }
        }
    }
}

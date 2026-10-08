//! The CLI's startup rules, checked by running the binary (AGT-8, AGT-11).
//!
//! The extension set is the first thing a run resolves, before the inference role or the working
//! directory, so a name this build does not carry stops the run with nothing else to wait for,
//! and the line saying what runs — and whether the guard is among it — is out before anything
//! else can fail. `SJEL_PERSONAL_ROOT` is removed in most of these tests: the operator's
//! `agent.toml` must not decide the outcome, and neither must their inference config.

use std::path::PathBuf;
use std::process::{Command, Output};

/// `sjel-agent [args...] "hi"` in a scratch directory, with no overlay unless one is named.
fn run(args: &[&str], overlay: Option<&PathBuf>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sjel-agent"));
    command.args(args).arg("hi");
    match overlay {
        Some(root) => command.env("SJEL_PERSONAL_ROOT", root),
        None => command.env_remove("SJEL_PERSONAL_ROOT"),
    };
    command
        .current_dir(std::env::temp_dir())
        .output()
        .expect("the binary did not run")
}

fn run_ext(flag: &str) -> Output {
    run(&["--ext", flag], None)
}

/// A scratch overlay holding just `config/agent.toml`, so the test never reads the operator's.
fn overlay_with(name: &str, agent_toml: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sjel-agent-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(root.join("config/agent.toml"), agent_toml).unwrap();
    root
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_unknown_extension_stops_the_startup() {
    let out = run_ext("+typo");
    assert!(!out.status.success(), "`--ext +typo` started anyway");
    let said = stderr(&out);
    assert!(said.contains("unknown extension `typo`"), "{said}");
    assert!(
        said.contains("guard"),
        "the names it does carry are not listed: {said}"
    );
}

#[test]
fn a_name_without_a_sign_is_not_a_guess() {
    let out = run_ext("typo");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("use `none` to clear the set"));
}

#[test]
fn the_guard_is_on_by_default() {
    let out = run(&[], None);
    let said = stderr(&out);
    assert!(said.contains("[extensions] guard"), "{said}");
    assert!(!said.contains("secrets-guard is off"), "{said}");
}

#[test]
fn none_clears_the_set_and_says_the_guard_is_off() {
    // It gets past the extension step, which is the form claim: the run then stops on the
    // missing inference role, because this test's overlay holds nothing.
    let out = run_ext("none");
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("secrets-guard is off"), "{said}");
    assert!(said.contains("inference role `coding`"), "{said}");
}

#[test]
fn removing_the_guard_alone_says_so() {
    let out = run_ext("-guard");
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("[extensions] none"), "{said}");
    assert!(said.contains("secrets-guard is off"), "{said}");
}

#[test]
fn a_config_that_names_no_extensions_says_the_guard_is_off() {
    let root = overlay_with("empty", "[agent]\nextensions = []\n");
    let out = run(&[], Some(&root));
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("[extensions] none"), "{said}");
    assert!(said.contains("secrets-guard is off"), "{said}");
}

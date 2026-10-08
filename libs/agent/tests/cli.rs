//! The CLI's startup rules, checked by running the binary (AGT-8).
//!
//! The extension set is the first thing a run resolves, before the inference role or the
//! working directory, so a name this build does not carry stops the run with nothing else to
//! wait for. `SJEL_PERSONAL_ROOT` is removed in these tests: the overlay's `agent.toml` must
//! not decide the outcome, and the failure has to be the flag's.

use std::process::{Command, Output};

/// `sjel-agent --ext <flag> "hi"` in a scratch directory with no overlay.
fn run_ext(flag: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sjel-agent"))
        .args(["--ext", flag, "hi"])
        .current_dir(std::env::temp_dir())
        .env_remove("SJEL_PERSONAL_ROOT")
        .output()
        .expect("the binary did not run")
}

#[test]
fn an_unknown_extension_stops_the_startup() {
    let out = run_ext("+typo");
    assert!(!out.status.success(), "`--ext +typo` started anyway");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("unknown extension `typo`"), "{said}");
}

#[test]
fn a_name_without_a_sign_is_not_a_guess() {
    let out = run_ext("typo");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("use `none` to clear the set"), "{said}");
}

#[test]
fn none_is_a_form_and_clears_the_set() {
    // It gets past the extension step, which is the whole claim here: the run then stops on
    // the missing inference role, because this test has no overlay.
    let out = run_ext("none");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!said.contains("extension"), "{said}");
    assert!(said.contains("inference role `coding`"), "{said}");
}

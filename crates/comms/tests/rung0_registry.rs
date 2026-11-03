//! Rung 0 end to end: a bare first name is redacted, and it is the registry
//! doing it rather than the salutation heuristic.
//!
//! This lives in `tests/` rather than beside the module because the registry is
//! a process-wide `OnceLock`. A unit test cannot choose which fixture wins the
//! initialisation race with its siblings; an integration test gets a fresh
//! process and can set the path before anything reads it.
//!
//! One `#[test]`, on purpose. Two tests in this binary still share the process,
//! so whichever ran first would initialise the `OnceLock` — and a test that had
//! not set the fixture path would load the machine's real overlay registry.
//! That race was invisible for as long as the fixture names happened to exist
//! in the real registry, which is exactly the failure a fixture exists to
//! prevent. One test, fixture first, then every assertion.

use std::io::Write;

fn fixture(names: &[&str]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("axon-comms-rung0");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("registry-{}.json", std::process::id()));
    let tokens = names
        .iter()
        .map(|n| format!("{n:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut f = std::fs::File::create(&p).unwrap();
    write!(
        f,
        r#"{{"people":1,"tokens":[{tokens}],"withheld":[],"refused":[]}}"#
    )
    .unwrap();
    p
}

#[test]
fn a_bare_first_name_is_redacted_and_an_unknown_word_survives() {
    let p = fixture(&["Erika", "Mustermann"]);
    std::env::set_var("SJEL_PEOPLE_REGISTRY", &p);

    let mut findings = Vec::new();
    let out = comms::cloud_derivative::redact_review_field(
        Some("Erika said the Lisbon trip is on, and Mustermann agreed."),
        &mut findings,
    )
    .expect("a value in gives a value out");

    assert!(!out.contains("Erika"), "rung 0 did not fire: {out}");
    assert!(!out.contains("Mustermann"), "surname not redacted: {out}");
    assert!(out.contains("Lisbon"), "a place is not a person: {out}");

    // The receipt Q9b needs: entity_type and a count, already carried by
    // RedactionFinding. Two names, one aggregated finding.
    let person = findings
        .iter()
        .find(|f| f.entity_type == "person")
        .expect("no person finding recorded");
    assert_eq!(
        person.count, 2,
        "both names must be counted for the receipt"
    );

    // A token absent from the registry must pass through untouched rather than
    // being guessed at. Asserted against the same loaded registry, which is
    // exactly the state a running service is in.
    let mut findings = Vec::new();
    let out = comms::cloud_derivative::redact_review_field(
        Some("The deployment finished on Tuesday."),
        &mut findings,
    )
    .expect("a value in gives a value out");
    assert!(
        out.contains("Tuesday"),
        "an ordinary word was redacted: {out}"
    );
    assert!(
        out.contains("deployment"),
        "an ordinary word was redacted: {out}"
    );

    std::env::remove_var("SJEL_PEOPLE_REGISTRY");

    let _ = std::fs::remove_file(&p);
}

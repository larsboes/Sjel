//! The command policy's corpus: every rule in `docs/plans/agent.md` §3, and the evasions that
//! must never be Allow.
//!
//! The cases live in `policy_cases.toml` so a rule change is a diff to data rather than to a test
//! body. A case names the class it must reach and, for the rules, the rule ID: an evasion case
//! names only the class, because which rule catches it is the policy's choice as long as it is
//! not Allow.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;
use sjel_agent::ext::policy::{Class, Policy};

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    command: String,
    class: String,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    why: String,
}

fn corpus() -> Corpus {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/policy_cases.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The scope every case is judged against: a checkout a session may write inside.
fn policy() -> Policy {
    Policy::new(Some(PathBuf::from("/Users/someone/Developer/Sjel")))
}

#[test]
fn every_case_reaches_the_class_it_names() {
    let corpus = corpus();
    assert!(!corpus.case.is_empty(), "the corpus is empty");
    let mut failures = Vec::new();
    for case in &corpus.case {
        let expected = Class::parse(&case.class)
            .unwrap_or_else(|| panic!("{}: `{}` is not a class", case.command, case.class));
        let decision = policy().bash(&case.command);
        if decision.class != expected {
            failures.push(format!(
                "{}  ({})\n  expected {} got {} by rule {}: {}",
                case.command,
                case.why,
                case.class,
                decision.class.as_str(),
                decision.rule,
                decision.reason
            ));
            continue;
        }
        if let Some(named) = &case.rule {
            if decision.rule != named {
                failures.push(format!(
                    "{}  ({})\n  expected rule {named}, got {}",
                    case.command, case.why, decision.rule
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n\n{}",
        failures.len(),
        corpus.case.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_rule_in_the_plan_has_a_case() {
    let corpus = corpus();
    let named: BTreeSet<&str> = corpus
        .case
        .iter()
        .filter_map(|case| case.rule.as_deref())
        .collect();
    for rule in [
        "B1", "B2", "B3", "B4", "B5", "B6", "B7", "A1", "A2", "A3", "A4", "A5", "A6", "R1", "R2",
        "R3",
    ] {
        assert!(named.contains(rule), "no case names rule {rule}");
    }
}

#[test]
fn an_evasion_is_never_allowed() {
    let corpus = corpus();
    for case in corpus.case.iter().filter(|case| case.rule.is_none()) {
        let decision = policy().bash(&case.command);
        assert_ne!(
            decision.class,
            Class::Allow,
            "{}  ({}) was allowed by {}",
            case.command,
            case.why,
            decision.rule
        );
    }
}

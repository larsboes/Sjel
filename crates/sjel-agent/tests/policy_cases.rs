//! The command policy's corpus: every case in `docs/plans/agent.md` §3, read from the one table
//! the pi extension also reads, so the two implementations cannot drift apart.
//!
//! A case names the class it must reach and, where the rule itself is the decision, the rule ID.
//! An evasion case names only the class: which rule catches it is the policy's choice as long as
//! it is not Allow.
//!
//! The scope is the fixture the table is written against (`cwd = /home/u/proj`, `home = /home/u`,
//! `writable = [/tmp]`), never the machine's, so a case means the same on every host.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;
use sjel_agent::ext::policy::{Class, Policy};

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    case: Vec<Case>,
    #[serde(default)]
    write: Vec<WriteCase>,
}

#[derive(Debug, Deserialize)]
struct Case {
    command: String,
    class: String,
    #[serde(default)]
    rule: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WriteCase {
    path: String,
    class: String,
}

fn corpus() -> Corpus {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../Packs/security/extensions/command-policy/cases.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The fixture scope the table is written against.
fn policy() -> Policy {
    Policy::new(Some(PathBuf::from("/home/u/proj")))
        .with_home("/home/u")
        .with_writable(vec![PathBuf::from("/tmp")])
}

#[test]
fn every_command_case_reaches_the_class_it_names() {
    let corpus = corpus();
    assert!(!corpus.case.is_empty(), "the corpus is empty");
    let mut failures = Vec::new();
    for case in &corpus.case {
        let expected = Class::parse(&case.class)
            .unwrap_or_else(|| panic!("{}: `{}` is not a class", case.command, case.class));
        let decision = policy().bash(&case.command);
        if decision.class != expected {
            failures.push(format!(
                "{}: expected {}, got {} by rule {}: {}",
                case.command,
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
                    "{}: expected rule {named}, got {}: {}",
                    case.command, decision.rule, decision.reason
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
fn every_write_case_reaches_the_class_it_names() {
    let corpus = corpus();
    assert!(!corpus.write.is_empty(), "the table has no write cases");
    let mut failures = Vec::new();
    for case in &corpus.write {
        let expected = Class::parse(&case.class)
            .unwrap_or_else(|| panic!("{}: `{}` is not a class", case.path, case.class));
        let decision = policy().write(&case.path);
        if decision.class != expected {
            failures.push(format!(
                "{}: expected {}, got {} by rule {}: {}",
                case.path,
                case.class,
                decision.class.as_str(),
                decision.rule,
                decision.reason
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} write cases failed:\n\n{}",
        failures.len(),
        corpus.write.len(),
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
        "R3", "R4",
    ] {
        assert!(named.contains(rule), "no case names rule {rule}");
    }
}

/// A case that names no rule but expects something other than Allow is an evasion: the shape
/// must not slip through. Several Allow cases leave the rule off, so they are skipped here and
/// held to their class by the case test above.
#[test]
fn an_evasion_is_never_allowed() {
    let corpus = corpus();
    for case in corpus
        .case
        .iter()
        .filter(|case| case.rule.is_none() && case.class != "allow")
    {
        let decision = policy().bash(&case.command);
        assert_ne!(
            decision.class,
            Class::Allow,
            "{} was allowed by {}",
            case.command,
            decision.rule
        );
    }
}

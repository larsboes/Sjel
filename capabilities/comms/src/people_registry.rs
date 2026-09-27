//! Rung 0 of the redaction ladder: the names this operator actually knows.
//!
//! `cloud_derivative::transform_text` is rung 1 — it matches *shapes*. A URL
//! looks like a URL, an IBAN looks like an IBAN. Its person detector is the one
//! rule there that cannot work on shape alone, so it is gated on a salutation:
//! "Herr Müller" is caught because "Herr" precedes it, and a bare "Erika" is
//! not caught at all.
//!
//! Measured across 353 `Journal/` notes in the operator's vault on 2026-08-23:
//! 2,802 occurrences of people the vault already names, of which the salutation
//! gate catches **2**. This module closes that gap by consulting a list instead
//! of guessing.
//!
//! ## Why this loads from a file rather than taking a parameter
//!
//! `redact_review_field` and `prepare` are public and called from `digest.rs`.
//! Threading a registry through them would change three signatures and their
//! tests to deliver what is, in substance, deployment configuration. The
//! registry is loaded once per process and consulted, the same way a config
//! value would be.
//!
//! ## Why the file lives in the overlay
//!
//! **The registry is C2 data.** It is a list of the real names of the
//! operator's friends, family and colleagues, which is precisely the class §6
//! of the PRD says never leaves the host in raw form. It belongs in the private
//! overlay and must never be committed to this repository. Nothing here writes
//! it; `vault names --json` produces it.
//!
//! ## Absent is not empty
//!
//! If the file is missing this returns an empty registry and rung 1 behaves
//! exactly as it did before. That is a deliberate downgrade rather than a
//! failure: a comms deployment on a machine with no vault should still redact
//! IBANs. The caller can ask [`state`] which case it is, so "no names loaded"
//! is reportable rather than indistinguishable from "no names matched".

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Overlay-relative location of the artifact `vault names --json` writes.
const OVERLAY_REL: &str = "data/vault/people-registry.json";

/// Environment override, for tests and for a deployment that keeps it elsewhere.
const ENV_PATH: &str = "SJEL_PEOPLE_REGISTRY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No path resolved, or the file is not there. Rung 1 only.
    Absent,
    /// Loaded, with this many single-token names.
    Loaded(usize),
    /// The file exists and could not be parsed. Distinguished from `Absent`
    /// because a corrupt registry is an operator problem and a missing one is a
    /// deployment choice.
    Unreadable,
}

struct Registry {
    /// Single-word names, compared case-insensitively via lowercase storage.
    single: BTreeSet<String>,
    /// Every entry of at least [`MIN_NAME_CHARS`] characters as written, phrases included,
    /// for the reversible path. Its dictionary matcher scans the text rather than single
    /// words, so a phrase can match there.
    all: Vec<String>,
    state: State,
}

/// Shorter entries are skipped on both paths. `Jan` survives; a two-letter initial would
/// match inside every other sentence.
const MIN_NAME_CHARS: usize = 3;

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static ENTITY_REGISTRY: OnceLock<sjel_pseudonymize::EntityRegistry> = OnceLock::new();

fn path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(ENV_PATH) {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    sjel_config::overlay_root().map(|r| r.join(OVERLAY_REL))
}

fn load() -> Registry {
    load_from_path(path().as_deref())
}

/// The whole loader, as a pure function of a path.
///
/// Split out so the cases below are testable without setting a process-wide
/// environment variable: Rust runs tests in parallel threads of one process, so
/// a test that mutates the environment is a test that fails whenever a sibling
/// happens to read it.
fn load_from_path(p: Option<&std::path::Path>) -> Registry {
    let Some(p) = p else {
        return Registry {
            single: BTreeSet::new(),
            all: Vec::new(),
            state: State::Absent,
        };
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Registry {
            single: BTreeSet::new(),
            all: Vec::new(),
            state: State::Absent,
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Registry {
            single: BTreeSet::new(),
            all: Vec::new(),
            state: State::Unreadable,
        };
    };
    let Some(tokens) = value.get("tokens").and_then(|t| t.as_array()) else {
        return Registry {
            single: BTreeSet::new(),
            all: Vec::new(),
            state: State::Unreadable,
        };
    };
    // Multi-word entries are carried in the artifact and ignored here on
    // purpose: `transform_text` walks whitespace-separated tokens, so a phrase
    // can never match one. Storing them anyway would report a registry size the
    // matcher cannot deliver.
    let single: BTreeSet<String> = tokens
        .iter()
        .filter_map(|t| t.as_str())
        .filter(|t| !t.contains(' '))
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let all: Vec<String> = tokens
        .iter()
        .filter_map(|t| t.as_str())
        .map(str::trim)
        .filter(|t| t.chars().count() >= MIN_NAME_CHARS)
        .map(str::to_string)
        .collect();
    let n = single.len();
    Registry {
        single,
        all,
        state: State::Loaded(n),
    }
}

fn entity_registry_from(registry: &Registry) -> sjel_pseudonymize::EntityRegistry {
    sjel_pseudonymize::EntityRegistry::builder()
        .add_people(registry.all.iter().cloned())
        .build()
}

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(load)
}

/// Whether the registry loaded, and how many names it holds.
///
/// Exposed so a receipt can say "0 names loaded" rather than leaving a reader to
/// infer that nothing sensitive was present.
pub fn state() -> State {
    registry().state
}

/// Rung 0 for the reversible path (PRD §6.2c, Q112): the same names as
/// [`is_known_person`], as the dictionary `libs/pseudonymize` matches.
///
/// Built once per process from the same file. Every comms caller of
/// `cloud_derivative::prepare_pseudonymized` passes this, so the two paths protect the same
/// set of people. Empty when the registry is absent or unreadable, exactly as rung 0 of the
/// destructive path is; [`state`] tells the two cases apart.
pub fn entity_registry() -> &'static sjel_pseudonymize::EntityRegistry {
    ENTITY_REGISTRY.get_or_init(|| entity_registry_from(registry()))
}

/// Is this token a person this operator knows?
///
/// Surrounding punctuation is stripped before the comparison because the caller
/// splits on whitespace, so a name at the end of a sentence arrives as `Erika,`.
pub fn is_known_person(token: &str) -> bool {
    let cleaned = token.trim_matches(|c: char| !c.is_alphanumeric());
    if cleaned.chars().count() < MIN_NAME_CHARS {
        return false;
    }
    registry().single.contains(&cleaned.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a uniquely named fixture and loads it by path. No environment
    /// mutation, so these are safe under the parallel test runner.
    fn load_json(name: &str, json: &str) -> Registry {
        let f = std::env::temp_dir().join(format!("comms-people-registry-{name}.json"));
        std::fs::write(&f, json).unwrap();
        let r = load_from_path(Some(&f));
        let _ = std::fs::remove_file(&f);
        r
    }

    #[test]
    fn loads_single_tokens_and_ignores_phrases() {
        let r = load_json("phrases", r#"{"tokens":["Erika","Mustermann","About Me"]}"#);
        assert_eq!(
            r.state,
            State::Loaded(2),
            "a phrase cannot match a whitespace token"
        );
        assert!(r.single.contains("erika"));
        assert!(r.single.contains("mustermann"));
    }

    /// The reversible path gets the same people, phrases included, and matches them in any
    /// case — the gap `is_known_person` closes by lowercasing, closed by folding there.
    #[test]
    fn the_reversible_dictionary_holds_the_same_names() {
        let r = load_json("entity", r#"{"tokens":["Erika","Erika Mustermann","Jo"]}"#);
        let registry = entity_registry_from(&r);
        assert_eq!(
            registry.len(),
            2,
            "names under three characters are skipped"
        );
        let mut session = sjel_pseudonymize::PseudonymizerSession::new();
        let out = session.tokenize_text("ERIKA MUSTERMANN und Erika", &registry);
        assert!(!out.to_lowercase().contains("erika"), "{out}");
        assert!(!out.to_lowercase().contains("mustermann"), "{out}");
    }

    #[test]
    fn a_missing_file_downgrades_to_rung_one_rather_than_failing() {
        let r = load_from_path(Some(std::path::Path::new(
            "/nonexistent/people-registry.json",
        )));
        assert_eq!(r.state, State::Absent);
        assert_eq!(
            load_from_path(None).state,
            State::Absent,
            "no path resolves the same way"
        );
        assert!(r.single.is_empty());
    }

    #[test]
    fn a_corrupt_file_is_distinguishable_from_a_missing_one() {
        let r = load_json("corrupt", "{ this is not json");
        assert_eq!(r.state, State::Unreadable);
    }

    #[test]
    fn matching_ignores_case_and_trailing_punctuation() {
        let r = load_json("case", r#"{"tokens":["Erika"]}"#);
        // is_known_person() reads the process-wide OnceLock, so the comparison
        // logic is asserted against the loaded set directly.
        for probe in ["Erika,", "erika.", "ERIKA!"] {
            let cleaned = probe
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase();
            assert!(r.single.contains(&cleaned), "{probe} should match");
        }
    }
}

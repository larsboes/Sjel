//! The known-person registry: rung 0 of the redaction ladder in `capabilities/comms`.
//!
//! Rung 1 (`comms/src/cloud_derivative.rs`) matches shapes, and its person detector fires only
//! after a salutation, so a bare first name passes it. Rung 0 is a lookup over the names this
//! operator actually knows (`comms/src/people_registry.rs`, which reads the file this produces).
//! Until 2026-10-20 `vault names` built it from the `Atlas/People` notes; the people live here
//! now, so the registry is built from every person and the operator's own entity.
//!
//! The rules are the ones `vault/src/names.rs` recorded (git history before 2026-10-20):
//!
//! - A name that joins two things (`&`, ` and `) or runs past four words is refused and
//!   reported, because a misfiled entry once put "Social" and "Personal" into the registry.
//! - Tokens shorter than three characters are dropped: they match everywhere and identify
//!   nobody.
//! - A name that is also an ordinary word ([`STOPLIST`]) is withheld and reported, never
//!   silently dropped.
//! - A multi-word alias is kept whole; the consumer matches it as a phrase.
//!
//! Where a token is ambiguous this prefers to redact: a wrongly redacted month costs a worse
//! cloud answer, a leaked name costs what PRD §6 exists to prevent.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::store::{EntitiesStore, StoreError};

/// Tokens that are a name here and an ordinary word somewhere. Carried over unchanged.
const STOPLIST: &[&str] = &[
    // Months, in both languages the operator writes in.
    "Jan", "Mai", "Juni", "Juli", "March", "May", "June", "July", "August",
    // German function words that survive the length filter.
    "Alle", "Aber", "Auch", "Dann", "Denn", "Mehr", "Sehr", "Viel", "Ende",
    // English words that are also surnames.
    "Personal", "Social", "Best", "Young", "Green", "Brown", "Church",
];

const MIN_TOKEN: usize = 3;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Registry {
    /// How many entities were read as people.
    pub people: usize,
    /// Names refused as not one person, with the reason.
    pub refused: Vec<Refused>,
    /// The matchable tokens, sorted and deduplicated. A multi-word alias stays whole.
    pub tokens: Vec<String>,
    /// Tokens the stoplist held back.
    pub withheld: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Refused {
    pub name: String,
    pub reason: &'static str,
}

fn looks_like_a_person(name: &str) -> Result<(), &'static str> {
    if name.contains('&') || name.contains(" and ") {
        return Err("names more than one thing");
    }
    if name.split_whitespace().count() > 4 {
        return Err("too long to be a name");
    }
    Ok(())
}

/// Build the registry from `(name, aliases)` pairs.
pub fn build(people: &[(String, Vec<String>)]) -> Registry {
    let mut tokens = BTreeSet::new();
    let mut withheld = BTreeSet::new();
    let mut registry = Registry::default();
    for (name, aliases) in people {
        if let Err(reason) = looks_like_a_person(name) {
            registry.refused.push(Refused {
                name: name.clone(),
                reason,
            });
            continue;
        }
        registry.people += 1;
        let candidates = name
            .split_whitespace()
            .map(str::to_string)
            .chain(aliases.iter().cloned());
        for candidate in candidates {
            let c = candidate
                .trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_string();
            if c.chars().count() < MIN_TOKEN {
                continue;
            }
            if STOPLIST.iter().any(|s| s.eq_ignore_ascii_case(&c)) {
                withheld.insert(c);
            } else {
                tokens.insert(c);
            }
        }
    }
    registry.tokens = tokens.into_iter().collect();
    registry.withheld = withheld.into_iter().collect();
    registry
}

/// Every person and the operator's own entity, with their `aliases`.
pub fn from_store(store: &EntitiesStore) -> Result<Registry, StoreError> {
    let mut people = Vec::new();
    for kind in ["person", "self"] {
        for entity in store.list(Some(kind), None)? {
            let aliases = entity
                .values
                .get("aliases")
                .and_then(|v| v.value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            people.push((entity.name, aliases));
        }
    }
    Ok(build(&people))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, aliases: &[&str]) -> (String, Vec<String>) {
        (
            name.to_string(),
            aliases.iter().map(|a| a.to_string()).collect(),
        )
    }

    #[test]
    fn names_and_aliases_become_tokens_and_the_traps_are_reported() {
        let registry = build(&[
            person("Erika Mustermann", &["Riki", "Tante Erika"]),
            person("Jan Ng", &[]),
            person("Social & Personal Links", &[]),
        ]);
        assert_eq!(registry.people, 2);
        assert_eq!(
            registry.tokens,
            vec!["Erika", "Mustermann", "Riki", "Tante Erika"]
        );
        assert_eq!(
            registry.withheld,
            vec!["Jan"],
            "a month is withheld, not dropped"
        );
        assert_eq!(registry.refused.len(), 1);
    }
}

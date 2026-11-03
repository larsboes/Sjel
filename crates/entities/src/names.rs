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
    // Name particles and prepositions inside a multi-word name ("Max aus Bonn", "van Dijk").
    // They identify nobody and match every German sentence: the agent view turned "Paper aus
    // der Bachelorarbeit" into "Paper <TRAVELER_…> der Bachelorarbeit" (2026-10-23).
    "aus", "von", "van", "del", "den", "der", "die", "das", "und", "bei",
    // Ordinary words a contact name carried, found in the registry on 2026-10-23. "Events"
    // tokenized every task's `events:` key.
    "Events", "Support", "Arbeit", "Bar", "Bikes",
];

/// Particles after which a contact name describes where someone is from or works, not who
/// they are: "Max aus Bonn", "Anna from Berlin", "Tom bei SAP". The words after one are
/// withheld, because a city in the registry redacts that city in every mail.
const QUALIFIERS: &[&str] = &["aus", "from", "in", "bei", "at", "@"];

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
    build_with_places(people, &BTreeSet::new())
}

/// [`build`], withholding every token that is a known place (lowercase). "Bonn" at the end of a
/// contact name is where that contact lives; in the registry it redacted Bonn in every mail.
pub fn build_with_places(people: &[(String, Vec<String>)], places: &BTreeSet<String>) -> Registry {
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
        let mut qualified = false;
        let mut in_brackets = false;
        let mut name_tokens = Vec::new();
        for word in name.split_whitespace() {
            in_brackets |= word.starts_with('(');
            if qualified || in_brackets {
                let c = word.trim_matches(|ch: char| !ch.is_alphanumeric());
                if c.chars().count() >= MIN_TOKEN {
                    withheld.insert(c.to_string());
                }
            } else {
                name_tokens.push(word.to_string());
            }
            qualified |= QUALIFIERS.iter().any(|q| q.eq_ignore_ascii_case(word));
            in_brackets &= !word.ends_with(')');
        }
        let candidates = name_tokens.into_iter().chain(aliases.iter().cloned());
        for candidate in candidates {
            let c = candidate
                .trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_string();
            if c.chars().count() < MIN_TOKEN {
                continue;
            }
            if STOPLIST.iter().any(|s| s.eq_ignore_ascii_case(&c))
                || places.contains(&c.to_lowercase())
            {
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
    let mut places = BTreeSet::new();
    for entity in store.list(None, None)? {
        if entity.kind == "place" {
            places.insert(entity.name.trim().to_lowercase());
        }
        for fact in &entity.facts {
            if let Some(city) = fact.place.split(',').next() {
                places.insert(city.trim().to_lowercase());
            }
        }
    }
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
    Ok(build_with_places(&people, &places))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particles_and_ordinary_words_in_a_name_are_withheld_not_matched() {
        let r = build(&[
            person("Max aus Bonn", &[]),
            person("Events Team Hamburg", &[]),
            person("eliah", &[]),
        ]);
        assert!(
            !r.tokens.iter().any(|t| t == "aus" || t == "Events"),
            "{:?}",
            r.tokens
        );
        assert!(
            r.withheld.contains(&"aus".to_string()) && r.withheld.contains(&"Events".to_string())
        );
        assert!(!r.tokens.iter().any(|t| t == "Bonn"), "{:?}", r.tokens);
        assert!(r.withheld.contains(&"Bonn".to_string()));
        // A real name typed in lowercase still redacts: a leak costs more than a wrong match.
        assert!(r.tokens.contains(&"eliah".to_string()));
        assert!(r.tokens.contains(&"Max".to_string()));
    }

    #[test]
    fn a_bracketed_affiliation_is_not_part_of_the_name() {
        let r = build(&[person("Erika (Uni Berlin) Tester", &[])]);
        assert!(r.tokens.contains(&"Erika".to_string()));
        assert!(r.tokens.contains(&"Tester".to_string()));
        assert!(!r.tokens.iter().any(|t| t == "Berlin" || t == "Uni"));
    }

    #[test]
    fn a_known_place_is_withheld_from_a_name() {
        let places = BTreeSet::from(["bonn".to_string()]);
        let r = build_with_places(&[person("Erika Mustermann Bonn", &[])], &places);
        assert!(r.tokens.contains(&"Mustermann".to_string()));
        assert!(!r.tokens.iter().any(|t| t == "Bonn"));
        assert!(r.withheld.contains(&"Bonn".to_string()));
    }

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

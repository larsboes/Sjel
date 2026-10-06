//! Which person a name or an email written elsewhere means (libs/links/ISA.md D8, LNK-14).
//!
//! Trips and places store people as text. They ask here rather than match names themselves, so
//! there is one set of name rules: [`duplicates`]'s `norm_name` and its first-name rule, where a
//! single word that begins a full name may be that person.
//!
//! Only an `exact` answer may be linked without asking the operator. `first` and `ambiguous` go
//! to the review list, and `none` is offered as a new person, never created here (D9).

use std::collections::BTreeMap;

pub use sjel_links::{Candidate, EmailAnswer, Match, NameAnswer, ResolveAnswer, ResolveRequest};

use crate::duplicates::{norm_name, strings};
use crate::model::Entity;

fn candidate(person: &Entity) -> Candidate {
    Candidate {
        id: person.id.clone(),
        name: person.name.clone(),
    }
}

/// Answers `request` against `entities`; anything that is not a person is ignored.
pub fn resolve(entities: &[Entity], request: &ResolveRequest) -> ResolveAnswer {
    let people: Vec<(&Entity, String)> = entities
        .iter()
        .filter(|e| e.kind == "person")
        .map(|p| (p, norm_name(&p.name)))
        .collect();

    let names = request
        .names
        .iter()
        .map(|name| {
            let wanted = norm_name(name);
            let exact: Vec<&Entity> = people
                .iter()
                .filter(|(_, n)| !wanted.is_empty() && *n == wanted)
                .map(|(p, _)| *p)
                .collect();
            let (status, found) = match exact.len() {
                1 => (Match::Exact, exact),
                n if n > 1 => (Match::Ambiguous, exact),
                _ if wanted.is_empty() || wanted.contains(' ') => (Match::None, vec![]),
                _ => {
                    let first: Vec<&Entity> = people
                        .iter()
                        .filter(|(_, n)| n.split(' ').next() == Some(wanted.as_str()))
                        .map(|(p, _)| *p)
                        .collect();
                    match first.len() {
                        0 => (Match::None, first),
                        1 => (Match::First, first),
                        _ => (Match::Ambiguous, first),
                    }
                }
            };
            NameAnswer {
                name: name.clone(),
                status,
                entity_id: (status == Match::Exact).then(|| found[0].id.clone()),
                candidates: found.into_iter().map(candidate).collect(),
            }
        })
        .collect();

    let mut by_email: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (person, _) in &people {
        for email in strings(person, "emails") {
            by_email
                .entry(email.trim().to_lowercase())
                .or_default()
                .push(&person.id);
        }
    }
    let emails = request
        .emails
        .iter()
        .map(|email| EmailAnswer {
            email: email.clone(),
            entity_id: match by_email
                .get(&email.trim().to_lowercase())
                .map(Vec::as_slice)
            {
                Some([one]) => Some((*one).to_string()),
                _ => None,
            },
        })
        .collect();

    ResolveAnswer { names, emails }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FieldValue;
    use serde_json::json;

    fn person(id: &str, name: &str, emails: &[&str]) -> Entity {
        let mut values = BTreeMap::new();
        if !emails.is_empty() {
            values.insert(
                "emails".to_string(),
                FieldValue {
                    value: json!(emails),
                    source: "operator".into(),
                    updated_at: String::new(),
                },
            );
        }
        Entity {
            id: id.into(),
            kind: "person".into(),
            name: name.into(),
            note_ref: None,
            revision: 1,
            created_at: String::new(),
            updated_at: String::new(),
            values,
            facts: vec![],
        }
    }

    fn statuses(answer: &ResolveAnswer) -> Vec<(Match, Option<&str>)> {
        answer
            .names
            .iter()
            .map(|a| (a.status, a.entity_id.as_deref()))
            .collect()
    }

    #[test]
    fn only_an_exact_full_name_carries_an_id() {
        let people = [
            person("ent:1", "Lucia García", &[]),
            person("ent:2", "Maria Lopez", &[]),
            person("ent:3", "Maria Schmidt", &[]),
            person("ent:4", "Jonas  Weber", &[]),
        ];
        let request = ResolveRequest {
            names: [
                "lucia garcía",
                "Lucia",
                "Maria",
                "jonas weber",
                "Nobody",
                "Ana Nobody",
                "",
            ]
            .map(String::from)
            .to_vec(),
            emails: vec![],
        };
        assert_eq!(
            statuses(&resolve(&people, &request)),
            [
                (Match::Exact, Some("ent:1")),
                (Match::First, None),
                (Match::Ambiguous, None),
                (Match::Exact, Some("ent:4")),
                (Match::None, None),
                (Match::None, None),
                (Match::None, None),
            ]
        );
    }

    #[test]
    fn two_people_with_one_full_name_are_ambiguous() {
        let people = [
            person("ent:1", "Anna Berg", &[]),
            person("ent:2", "Anna Berg", &[]),
        ];
        let request = ResolveRequest {
            names: vec!["Anna Berg".into()],
            emails: vec![],
        };
        let answer = resolve(&people, &request);
        assert_eq!(answer.names[0].status, Match::Ambiguous);
        assert_eq!(answer.names[0].candidates.len(), 2);
    }

    #[test]
    fn an_email_links_only_when_one_person_carries_it() {
        let people = [
            person("ent:1", "Lucia García", &["Lucia@Example.org"]),
            person("ent:2", "A", &["shared@example.org"]),
            person("ent:3", "B", &["shared@example.org"]),
        ];
        let request = ResolveRequest {
            names: vec![],
            emails: ["lucia@example.org", "shared@example.org", "x@example.org"]
                .map(String::from)
                .to_vec(),
        };
        let ids: Vec<_> = resolve(&people, &request)
            .emails
            .into_iter()
            .map(|e| e.entity_id)
            .collect();
        assert_eq!(ids, [Some("ent:1".to_string()), None, None]);
    }
}

//! Finding people who are probably the same person, and asking the local model about the
//! unclear ones.
//!
//! Rules find the candidates, because the strong signals are exact: two records that share
//! an email address or a phone number are almost always one person, and two with the same
//! name usually are. Those need no model. The unclear case is a first name alone against a
//! full name ("Ron" from a note, "Ron Mustermann" from Google) with nothing else shared. Only
//! those go to the on-device model (`capabilities/foundation-models`, loopback), which may
//! see C2 because it never leaves the machine (PRD §6.1). The model advises; the operator
//! decides every merge.

use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::model::{located_on, Entity};

/// How sure the rules are, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// A shared email address or phone number.
    Strong,
    /// The same name, nothing else shared.
    Name,
    /// One name is a first name that begins the other.
    Partial,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub a: String,
    pub b: String,
    pub strength: Strength,
    pub reasons: Vec<String>,
}

/// The pair key, order-free, as stored in `entities_distinct`.
pub fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn norm_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The last nine digits: `+49 228 1234567`, `0228 1234567` and `02281234567` agree. Shorter
/// numbers are too ambiguous to match on and are dropped.
fn norm_phone(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 7).then(|| digits[digits.len().saturating_sub(9)..].to_string())
}

fn strings(entity: &Entity, key: &str) -> Vec<String> {
    entity
        .values
        .get(key)
        .and_then(|v| v.value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Every candidate pair among `people`, strongest first. A pair the operator marked as
/// different people is never proposed again.
pub fn candidates(people: &[Entity], distinct: &HashSet<(String, String)>) -> Vec<Candidate> {
    let prepared: Vec<(&Entity, String, HashSet<String>, HashSet<String>)> = people
        .iter()
        .map(|p| {
            let emails = strings(p, "emails")
                .iter()
                .map(|e| e.trim().to_lowercase())
                .collect();
            let phones = strings(p, "phones")
                .iter()
                .filter_map(|n| norm_phone(n))
                .collect();
            (p, norm_name(&p.name), emails, phones)
        })
        .collect();
    let mut out = Vec::new();
    for (i, (a, a_name, a_emails, a_phones)) in prepared.iter().enumerate() {
        for (b, b_name, b_emails, b_phones) in prepared.iter().skip(i + 1) {
            if a.kind != b.kind || distinct.contains(&pair(&a.id, &b.id)) {
                continue;
            }
            let mut reasons = Vec::new();
            for email in a_emails.intersection(b_emails) {
                reasons.push(format!("same email {email}"));
            }
            if a_phones.intersection(b_phones).next().is_some() {
                reasons.push("same phone number".to_string());
            }
            let strength = if !reasons.is_empty() {
                Strength::Strong
            } else if a_name == b_name && !a_name.is_empty() {
                reasons.push("same name".to_string());
                Strength::Name
            } else {
                let (short, long) = if a_name.len() <= b_name.len() {
                    (a_name, b_name)
                } else {
                    (b_name, a_name)
                };
                let first_name_only = !short.is_empty()
                    && !short.contains(' ')
                    && long.split(' ').next() == Some(short.as_str());
                if !first_name_only {
                    continue;
                }
                reasons.push(format!("\"{short}\" could be \"{long}\""));
                Strength::Partial
            };
            out.push(Candidate {
                a: a.id.clone(),
                b: b.id.clone(),
                strength,
                reasons,
            });
        }
    }
    out.sort_by_key(|c| c.strength);
    out
}

/// What the model is shown about one record: the fields that tell people apart, and where
/// they came from. No note text.
pub fn profile(entity: &Entity, today: &str) -> Value {
    let value = |key: &str| entity.values.get(key).map(|v| v.value.clone());
    json!({
        "name": entity.name,
        "lives_in": located_on(&entity.facts, today).map(|f| f.place.clone()),
        "company": value("company"),
        "role": value("role"),
        "relation": value("relation"),
        "emails": value("emails"),
        "phones": value("phones"),
        "birthday": value("birthday"),
        "sources": entity.values.values().map(|v| v.source.clone()).collect::<HashSet<_>>(),
        "has_note": entity.note_ref.is_some(),
    })
}

/// The fields both records carry, compared. A field only one side has says nothing about
/// whether they are one person, so it is not compared and not shown to the model: on
/// 2026-09-25 the on-device model called 13 of 16 pairs "different company" or "different
/// city" when only one record had a company or a city.
pub const COMPARED: &[&str] = &["lives_in", "company", "birthday", "relation", "role"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evidence {
    /// Fields both carry with the same value (case-insensitive for text).
    pub same: Vec<String>,
    /// Fields both carry with different values.
    pub different: Vec<String>,
}

fn comparable(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.trim().to_lowercase()),
        other => Some(other.to_string().to_lowercase()),
    }
}

pub fn evidence(a: &Value, b: &Value) -> Evidence {
    let mut out = Evidence {
        same: vec![],
        different: vec![],
    };
    for key in COMPARED {
        if let (Some(x), Some(y)) = (comparable(&a[*key]), comparable(&b[*key])) {
            if x == y {
                out.same.push((*key).to_string());
            } else {
                out.different.push((*key).to_string());
            }
        }
    }
    out
}

/// A profile reduced to the name and the fields the other record also carries.
pub fn shared_only(profile: &Value, evidence: &Evidence) -> Value {
    let mut out = json!({ "name": profile["name"] });
    for key in evidence.same.iter().chain(&evidence.different) {
        out[key] = profile[key.as_str()].clone();
    }
    out
}

/// The model's advice on one pair.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    /// `None` when the model could not tell, or its answer could not be read.
    pub same: Option<bool>,
    pub why: String,
}

/// Reads the model's reply: a JSON object with `same` and `why`, possibly inside prose or a
/// code fence. Anything else is "could not tell", never a guess.
pub fn parse_verdict(reply: &str) -> Verdict {
    let parsed = reply
        .find('{')
        .and_then(|start| reply.rfind('}').map(|end| &reply[start..=end]))
        .and_then(|body| serde_json::from_str::<Value>(body).ok());
    match parsed {
        Some(v) => Verdict {
            same: v["same"].as_bool(),
            why: v["why"].as_str().unwrap_or("no reason given").to_string(),
        },
        None => Verdict {
            same: None,
            why: "the model's answer could not be read".into(),
        },
    }
}

/// Asks the on-device model whether two records are one person. Any failure is a verdict of
/// "could not tell" with the reason, so a model that is down never blocks the list.
pub fn judge(model_url: &str, a: &Value, b: &Value) -> Verdict {
    let client = match sjel_http::client(
        sjel_http::Purpose::new("entities-judge"),
        Duration::from_secs(30),
    ) {
        Ok(client) => client,
        Err(error) => {
            return Verdict {
                same: None,
                why: format!("model client: {error}"),
            }
        }
    };
    let prompt = format!(
        "Two contact records from one person's address book. Are they the same person? \
         Each record shows only the fields both records have. A first name alone matching a \
         full name is weak evidence; a different city, company or birthday is evidence \
         against. Answer with JSON only: \
         {{\"same\": true|false|null, \"why\": \"one short sentence\"}}. Use null when the \
         records do not say.\nRecord A: {a}\nRecord B: {b}"
    );
    let body = json!({
        "model": "apple-foundationmodel",
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0,
    });
    let reply: Result<Value, String> = client
        .post(format!(
            "{}/v1/chat/completions",
            model_url.trim_end_matches('/')
        ))
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| e.to_string());
    match reply {
        Ok(v) => parse_verdict(
            v["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default(),
        ),
        Err(error) => Verdict {
            same: None,
            why: format!("local model did not answer: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FieldValue;
    use std::collections::BTreeMap;

    fn person(id: &str, name: &str, values: &[(&str, Value)]) -> Entity {
        Entity {
            id: id.into(),
            kind: "person".into(),
            name: name.into(),
            note_ref: None,
            revision: 1,
            created_at: "0".into(),
            updated_at: "0".into(),
            values: values
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        FieldValue {
                            value: v.clone(),
                            source: "google".into(),
                            updated_at: "0".into(),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            facts: vec![],
        }
    }

    #[test]
    fn shared_contact_details_are_strong_and_names_rank_below() {
        let people = vec![
            person("1", "Ron", &[]),
            person(
                "2",
                "Ron Mustermann",
                &[("phones", json!(["+49 228 1234567"]))],
            ),
            person("3", "R. Mustermann", &[("phones", json!(["0228 1234567"]))]),
            person("4", "Anna Schmidt", &[]),
            person("5", "anna  schmidt", &[]),
            person("6", "Ronja", &[]),
        ];
        let found = candidates(&people, &HashSet::new());
        let got: Vec<(&str, &str, Strength)> = found
            .iter()
            .map(|c| (c.a.as_str(), c.b.as_str(), c.strength))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2", "3", Strength::Strong),
                ("4", "5", Strength::Name),
                ("1", "2", Strength::Partial)
            ],
            "Ronja is not Ron: a first name must match a whole word"
        );
        assert_eq!(found[0].reasons, vec!["same phone number".to_string()]);
    }

    #[test]
    fn a_pair_marked_distinct_is_not_proposed_again() {
        let people = vec![person("a", "Anna", &[]), person("b", "Anna", &[])];
        let distinct = HashSet::from([pair("b", "a")]);
        assert!(candidates(&people, &distinct).is_empty());
    }

    #[test]
    fn only_fields_both_carry_are_compared() {
        let a = json!({ "name": "Ron", "company": "Acme", "lives_in": "Bonn", "birthday": null });
        let b = json!({ "name": "Ron M", "company": "acme ", "lives_in": "Köln", "birthday": "1990-01-01", "role": "Dev" });
        let e = evidence(&a, &b);
        assert_eq!(e.same, vec!["company".to_string()]);
        assert_eq!(e.different, vec!["lives_in".to_string()]);
        assert_eq!(
            shared_only(&b, &e),
            json!({ "name": "Ron M", "company": "acme ", "lives_in": "Köln" })
        );
        assert_eq!(
            evidence(&json!({ "name": "Ron" }), &b),
            Evidence {
                same: vec![],
                different: vec![]
            }
        );
    }

    #[test]
    fn the_verdict_is_read_from_json_in_prose_or_not_at_all() {
        assert_eq!(
            parse_verdict("Sure. ```json\n{\"same\": false, \"why\": \"different cities\"}\n```"),
            Verdict {
                same: Some(false),
                why: "different cities".into()
            }
        );
        assert_eq!(
            parse_verdict("{\"same\": null, \"why\": \"no data\"}").same,
            None
        );
        assert_eq!(parse_verdict("I think so").same, None);
    }
}

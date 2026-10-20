//! Kinds, fields, values and dated facts, and the rules each obeys.
//!
//! PRD Q117 ruled the shape: a typed core per kind plus fields the operator declares, dated
//! facts with `valid_from`/`valid_to`, and a source and a data class on every value. A fully
//! generic attribute store was rejected because it checks nothing on write.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an entity can be. `self` is the operator: the personal context TELOS holds today.
pub const KINDS: &[&str] = &["person", "organisation", "place", "self"];

/// The value types a field can declare. Each has one check in [`check_value`].
pub const FIELD_TYPES: &[&str] = &[
    "text", "longtext", "bool", "date", "number", "enum", "emails", "phones", "url", "tags",
];

/// PRD §6.1's classes. C2 is a fact about a named person; C3 never leaves in any form.
pub const DATA_CLASSES: &[&str] = &["C0", "C1", "C2", "C3"];

/// Where a value came from. `operator` is Lars typing it.
pub const SOURCES: &[&str] = &[
    "operator",
    "obsidian",
    "google",
    "places-register",
    "import",
    "journal",
];

/// The dated facts a person can carry. Both place the entity somewhere for a span of days.
///
/// - `home_base`: where someone lives from `valid_from` on. A move is a new fact, so the old
///   home keeps its dates and "where did Ron live in 2025" stays answerable.
/// - `away`: somewhere else for a span, like "Lisbon 4–18 Oct". It wins over the home base
///   on the days it covers.
pub const FACT_PREDICATES: &[&str] = &["home_base", "away"];

/// One field an entity kind has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FieldDef {
    pub kind: String,
    /// `snake_case`, 1 to 40 characters. The key a value is stored under.
    pub key: String,
    pub label: String,
    pub field_type: String,
    /// The allowed values of an `enum` field, in display order. Empty for other types.
    #[serde(default)]
    pub options: Vec<String>,
    pub data_class: String,
    /// Built in by this capability, as opposed to declared by the operator.
    #[serde(default)]
    pub builtin: bool,
}

/// The fields every install has. PRD Q117 named them on 2026-09-25: a sleeping option to
/// ask for, and contact details. Home base and away periods are dated facts, not fields.
pub fn builtin_fields() -> Vec<FieldDef> {
    let person = |key: &str, label: &str, field_type: &str, options: &[&str]| FieldDef {
        kind: "person".into(),
        key: key.into(),
        label: label.into(),
        field_type: field_type.into(),
        options: options.iter().map(|o| o.to_string()).collect(),
        data_class: "C2".into(),
        builtin: true,
    };
    vec![
        person(
            "sleeping_option",
            "Sleeping option",
            "enum",
            &["none", "ask", "yes"],
        ),
        person("sleeping_note", "Sleeping note", "text", &[]),
        person("emails", "Emails", "emails", &[]),
        person("phones", "Phones", "phones", &[]),
        // Free prose about the person, markdown, several lines. Holds what the Atlas/People
        // notes carried as body and as the character, highlights, memories and experiences
        // keys until 2026-10-20, when the notes left the vault.
        person("notes", "Notes", "longtext", &[]),
        // The keys Atlas/People notes carry filled, measured 2026-09-25 across 89 notes, and
        // what Google Contacts carries.
        person("relation", "Relation", "text", &[]),
        person("company", "Company", "text", &[]),
        person("role", "Role", "text", &[]),
        person("birthday", "Birthday", "date", &[]),
        person("interests", "Interests", "tags", &[]),
        person("skills", "Skills", "tags", &[]),
        person("socials", "Socials", "tags", &[]),
        // Computed by capabilities/vault from Journal backlinks, never typed.
        person("last_contact", "Last contact", "date", &[]),
    ]
}

/// A field key: lower-case letter first, then letters, digits and `_`, 1 to 40 characters.
pub fn check_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty()
        && key.len() <= 40
        && key.as_bytes()[0].is_ascii_lowercase()
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "field key {key:?} must be snake_case: a lower-case letter, then letters, digits \
             or _, at most 40 characters"
        ))
    }
}

pub fn check_one_of(what: &str, value: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be one of: {} (got {value:?})",
            allowed.join(", ")
        ))
    }
}

/// Checks a field declaration before it is stored.
pub fn check_field(def: &FieldDef) -> Result<(), String> {
    check_one_of("kind", &def.kind, KINDS)?;
    check_key(&def.key)?;
    check_one_of("field_type", &def.field_type, FIELD_TYPES)?;
    check_one_of("data_class", &def.data_class, DATA_CLASSES)?;
    if def.label.trim().is_empty() {
        return Err("label must not be empty".into());
    }
    match (def.field_type.as_str(), def.options.is_empty()) {
        ("enum", true) => Err("an enum field needs at least one option".into()),
        ("enum", false) => Ok(()),
        (_, false) => Err("only an enum field takes options".into()),
        _ => Ok(()),
    }
}

/// `YYYY-MM-DD` by shape, with a month 01–12 and a day 01–31. Stored and compared as text.
pub fn is_day(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u8 = value[5..7].parse().unwrap_or(0);
    let day: u8 = value[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// Checks a value against its field and returns it normalised: trimmed text, a list with
/// empty entries removed. `null` is never passed here; it means "clear the field".
pub fn check_value(def: &FieldDef, value: &Value) -> Result<Value, String> {
    let wrong = |expected: &str| {
        Err(format!(
            "field {:?} is {}: expected {expected}",
            def.key, def.field_type
        ))
    };
    match def.field_type.as_str() {
        "text" | "longtext" | "url" => match value.as_str().map(str::trim) {
            Some(text) if !text.is_empty() => Ok(Value::String(text.into())),
            _ => wrong("a non-empty string"),
        },
        "bool" => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => wrong("true or false"),
        },
        "number" => match value {
            Value::Number(_) => Ok(value.clone()),
            _ => wrong("a number"),
        },
        "date" => match value.as_str() {
            Some(day) if is_day(day) => Ok(value.clone()),
            _ => wrong("a YYYY-MM-DD date"),
        },
        "enum" => match value.as_str() {
            Some(choice) if def.options.iter().any(|o| o == choice) => Ok(value.clone()),
            _ => wrong(&format!("one of {}", def.options.join(", "))),
        },
        "emails" | "phones" | "tags" => {
            let Some(items) = value.as_array() else {
                return wrong("a list of strings");
            };
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item.as_str().map(str::trim) else {
                    return wrong("a list of strings");
                };
                if text.is_empty() {
                    continue;
                }
                if def.field_type == "emails" && !text.contains('@') {
                    return Err(format!("{text:?} in {:?} is not an email address", def.key));
                }
                out.push(Value::String(text.into()));
            }
            Ok(Value::Array(out))
        }
        other => Err(format!("unknown field type {other:?}")),
    }
}

/// One stored value and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldValue {
    pub value: Value,
    pub source: String,
    pub updated_at: String,
}

/// A dated place fact: someone lives somewhere, or is somewhere else for a while.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fact {
    pub id: String,
    pub entity_id: String,
    pub predicate: String,
    /// What the operator typed, such as "Lisbon". The coordinate is resolved from it.
    pub place: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub note: Option<String>,
    pub source: String,
    pub created_at: String,
}

impl Fact {
    /// Whether the fact holds on `day`. An open start or end is open.
    pub fn covers(&self, day: &str) -> bool {
        self.valid_from.as_deref().is_none_or(|from| from <= day)
            && self.valid_to.as_deref().is_none_or(|to| to >= day)
    }
}

/// Where an entity is on `day`: an `away` fact covering it, else the home base that holds
/// on it. Among several, the one that started latest wins, because a newer fact is the
/// more recent statement. `None` when nothing places the entity that day.
pub fn located_on<'a>(facts: &'a [Fact], day: &str) -> Option<&'a Fact> {
    let latest = |predicate: &str| {
        facts
            .iter()
            .filter(|f| f.predicate == predicate && f.covers(day))
            .max_by(|a, b| a.valid_from.cmp(&b.valid_from))
    };
    latest("away").or_else(|| latest("home_base"))
}

/// One entity with its values and facts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub name: String,
    /// A vault path, such as `Atlas/People/Ron.md`, when a note exists. Prose stays there.
    pub note_ref: Option<String>,
    pub revision: u32,
    pub created_at: String,
    pub updated_at: String,
    pub values: BTreeMap<String, FieldValue>,
    pub facts: Vec<Fact>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(predicate: &str, place: &str, from: Option<&str>, to: Option<&str>) -> Fact {
        Fact {
            id: place.into(),
            entity_id: "e".into(),
            predicate: predicate.into(),
            place: place.into(),
            latitude: None,
            longitude: None,
            valid_from: from.map(str::to_string),
            valid_to: to.map(str::to_string),
            note: None,
            source: "operator".into(),
            created_at: "0".into(),
        }
    }

    #[test]
    fn away_wins_on_its_days_and_a_move_keeps_the_old_home() {
        let facts = vec![
            fact("home_base", "Köln", Some("2020-01-01"), Some("2024-06-30")),
            fact("home_base", "Bonn", Some("2024-07-01"), None),
            fact("away", "Lisbon", Some("2026-10-04"), Some("2026-10-18")),
        ];
        let at = |day| located_on(&facts, day).map(|f| f.place.as_str());
        assert_eq!(at("2026-10-10"), Some("Lisbon"));
        assert_eq!(at("2026-10-19"), Some("Bonn"));
        assert_eq!(at("2023-05-01"), Some("Köln"));
        assert_eq!(at("2019-01-01"), None);
    }

    #[test]
    fn an_undated_home_base_holds_every_day() {
        let facts = vec![fact("home_base", "Bonn", None, None)];
        assert_eq!(
            located_on(&facts, "2026-10-14").map(|f| f.place.as_str()),
            Some("Bonn")
        );
    }

    #[test]
    fn values_are_checked_against_their_field() {
        let fields = builtin_fields();
        let field = |key: &str| fields.iter().find(|f| f.key == key).unwrap();
        assert_eq!(
            check_value(field("sleeping_option"), &json!("ask")).unwrap(),
            json!("ask")
        );
        assert!(check_value(field("sleeping_option"), &json!("maybe")).is_err());
        assert_eq!(
            check_value(field("emails"), &json!([" ron@example.org ", ""])).unwrap(),
            json!(["ron@example.org"])
        );
        assert!(check_value(field("emails"), &json!(["not-an-address"])).is_err());
        assert!(check_value(field("sleeping_note"), &json!("  ")).is_err());
        assert_eq!(
            check_value(field("notes"), &json!("\n## Memories\n\nOne.\nTwo.\n")).unwrap(),
            json!("## Memories\n\nOne.\nTwo."),
            "a longtext keeps its inner line breaks"
        );
    }

    #[test]
    fn a_declared_field_is_checked_before_it_is_stored() {
        let mut def = FieldDef {
            kind: "person".into(),
            key: "climbing_grade".into(),
            label: "Climbing grade".into(),
            field_type: "text".into(),
            options: vec![],
            data_class: "C2".into(),
            builtin: false,
        };
        assert!(check_field(&def).is_ok());
        def.key = "Climbing Grade".into();
        assert!(check_field(&def).unwrap_err().contains("snake_case"));
        def.key = "grade".into();
        def.field_type = "enum".into();
        assert!(check_field(&def)
            .unwrap_err()
            .contains("at least one option"));
        def.kind = "pet".into();
        assert!(check_field(&def).unwrap_err().contains("kind"));
    }

    #[test]
    fn days_are_checked_by_shape_and_range() {
        assert!(is_day("2026-10-14"));
        for bad in ["14.10.2026", "2026-13-01", "2026-10-00", "2026-1-14", ""] {
            assert!(!is_day(bad), "{bad}");
        }
    }
}

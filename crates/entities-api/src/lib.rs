//! Typed API contract, wire models, and route constants for the Sjel entities capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-entities`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_ENTITIES: &str = "/api/entities";
pub const ROUTE_ENTITY_BY_ID: &str = "/api/entities/{id}";
pub const ROUTE_FIELDS: &str = "/api/fields";
pub const ROUTE_DUPLICATES: &str = "/api/duplicates";
pub const ROUTE_DUPLICATES_DISTINCT: &str = "/api/duplicates/distinct";
pub const ROUTE_MERGE: &str = "/api/entities/{id}/merge";
pub const ROUTE_LOCATED: &str = "/api/located";
pub const ROUTE_HOME: &str = "/api/home";
pub const ROUTE_RESOLVE: &str = "/api/resolve";
pub const ROUTE_SOURCES: &str = "/api/entities/{id}/sources";

// ============================================================================
// Core Vocabularies
// ============================================================================

pub const KINDS: &[&str] = &["person", "organisation", "place", "self"];

pub const FIELD_TYPES: &[&str] = &[
    "text", "longtext", "bool", "date", "number", "enum", "emails", "phones", "url", "tags",
];

pub const DATA_CLASSES: &[&str] = &["C0", "C1", "C2", "C3"];

pub const SOURCES: &[&str] = &[
    "operator",
    "obsidian",
    "google",
    "places-register",
    "import",
    "journal",
];

pub const FACT_PREDICATES: &[&str] = &["home_base", "away"];

// ============================================================================
// Models
// ============================================================================

/// One field an entity kind has (`GET /entities/api/fields`): built in or operator-declared.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct EntityField {
    #[serde(default)]
    pub kind: String,
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub field_type: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub data_class: String,
    #[serde(default)]
    pub builtin: bool,
}

pub type FieldDef = EntityField;

/// One stored value and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FieldValue {
    pub value: Value,
    pub source: String,
    pub updated_at: String,
}

/// One dated place fact: a home base, or an away period.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct EntityFact {
    pub id: String,
    #[serde(default)]
    pub predicate: String,
    #[serde(default)]
    pub place: String,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_to: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub source: String,
}

pub type Fact = EntityFact;

impl EntityFact {
    /// Whether the fact holds on `day`. An open start or end is open.
    pub fn covers(&self, day: &str) -> bool {
        self.valid_from.as_deref().is_none_or(|from| from <= day)
            && self.valid_to.as_deref().is_none_or(|to| to >= day)
    }
}

/// Where an entity is on `day`: an `away` fact covering it, else the home base that holds on it.
pub fn located_on<'a>(facts: &'a [EntityFact], day: &str) -> Option<&'a EntityFact> {
    let latest = |predicate: &str| {
        facts
            .iter()
            .filter(|f| f.predicate == predicate && f.covers(day))
            .max_by(|a, b| a.valid_from.cmp(&b.valid_from))
    };
    latest("away").or_else(|| latest("home_base"))
}

/// One entity record from `GET /entities/api/entities`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct EntityRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub note_ref: Option<String>,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    #[serde(default)]
    pub facts: Vec<EntityFact>,
}

pub type Entity = EntityRecord;

/// One side of a probable duplicate (`/entities/api/duplicates`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DuplicateSide {
    pub id: String,
    pub profile: Value,
}

/// A probable duplicate pair with the registry's default merge.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DuplicatePair {
    pub a: DuplicateSide,
    pub b: DuplicateSide,
    pub strength: String,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub keep: String,
    pub merged_name: String,
}

// ============================================================================
// Core Validation Logic
// ============================================================================

/// The fields every install has built in.
pub fn builtin_fields() -> Vec<EntityField> {
    let person = |key: &str, label: &str, field_type: &str, options: &[&str]| EntityField {
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
        person("notes", "Notes", "longtext", &[]),
        person("aliases", "Aliases", "tags", &[]),
        person("relation", "Relation", "text", &[]),
        person("company", "Company", "text", &[]),
        person("role", "Role", "text", &[]),
        person("birthday", "Birthday", "date", &[]),
        person("interests", "Interests", "tags", &[]),
        person("skills", "Skills", "tags", &[]),
        person("socials", "Socials", "tags", &[]),
        person("last_contact", "Last contact", "date", &[]),
    ]
}

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

pub fn check_field(def: &EntityField) -> Result<(), String> {
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

pub fn check_value(def: &EntityField, value: &Value) -> Result<Value, String> {
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

pub fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

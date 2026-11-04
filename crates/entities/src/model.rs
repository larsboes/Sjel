//! Kinds, fields, values and dated facts, and the rules each obeys.
//!
//! PRD Q117 ruled the shape: a typed core per kind plus fields the operator declares, dated
//! facts with `valid_from`/`valid_to`, and a source and a data class on every value. A fully
//! generic attribute store was rejected because it checks nothing on write.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

pub use entities_api::{
    builtin_fields, check_field, check_key, check_one_of, check_value, is_day, DATA_CLASSES,
    FACT_PREDICATES, FIELD_TYPES, FieldDef, KINDS, SOURCES,
};

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

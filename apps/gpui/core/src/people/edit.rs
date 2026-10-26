//! Writing a person from the app: a new person, an edit, a dated fact.
//!
//! The registry has had every route since it shipped; the page was read-only because nobody had
//! built the forms. Nothing here decides what a field means: the field list comes from
//! `GET /entities/api/fields`, and a value the registry would refuse is named by the form before
//! anything is sent, so a typing mistake is not reported as a service failure.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::shell::EntityField;

/// The two predicates `capabilities/entities` accepts, with the words the form shows.
pub const FACT_PREDICATES: [(&str, &str); 2] = [("home_base", "Home base"), ("away", "Away")];

/// A field being edited: its key, the label to show, and how the value is shaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditField {
    pub key: String,
    pub label: String,
    /// The registry's `field_type`: `text`, `emails`, `phones`, `tags`, `date`, `enum`, and so on.
    pub field_type: String,
    /// The allowed values of an `enum` field.
    pub options: Vec<String>,
    /// Who wrote what is in it now, when that is not the operator.
    pub source: Option<String>,
}

impl EditField {
    pub fn new(field: &EntityField, source: Option<String>) -> Self {
        Self {
            key: field.key.clone(),
            label: field.label.clone(),
            field_type: field.field_type.clone(),
            options: field.options.clone(),
            source,
        }
    }

    /// True for the fields a person is written with before they have any contact details.
    pub fn is_identity(&self) -> bool {
        matches!(self.key.as_str(), "alias" | "aliases")
    }
}

/// The fields to offer for a person: the registry's, with any base field it does not carry still
/// shown when the person has a value under that key.
pub fn edit_fields(declared: &[EntityField], held: &BTreeMap<String, String>) -> Vec<EditField> {
    let mut fields: Vec<EditField> = declared
        .iter()
        .filter(|field| field.key != "last_contact")
        .map(|field| EditField::new(field, None))
        .collect();
    for key in held.keys() {
        if !fields.iter().any(|field| &field.key == key) {
            fields.push(EditField {
                key: key.clone(),
                label: key.replace('_', " "),
                field_type: "text".into(),
                options: Vec::new(),
                source: None,
            });
        }
    }
    fields
}

/// A new person. Only a name is required; every other field is left to the registry's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewPerson {
    pub name: String,
    pub values: Vec<(String, String)>,
    /// The field types the values are shaped by, keyed as the registry keys its fields.
    pub types: BTreeMap<String, String>,
}

impl NewPerson {
    pub fn body(&self) -> Result<Value, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("A person needs a name.".to_owned());
        }
        let mut values = serde_json::Map::new();
        for (key, text) in &self.values {
            let field_type = self.types.get(key).map(String::as_str).unwrap_or("text");
            if let Some(value) = typed_as(key, field_type, text)? {
                values.insert(key.clone(), value);
            }
        }
        Ok(json!({ "kind": "person", "name": name, "values": values }))
    }
}

/// A change to one person. Every field the form shows is sent only when it differs from what the
/// registry answered with, so an untouched field is never written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PersonEdit {
    pub id: String,
    /// The name the form holds now.
    pub name: String,
    /// The name the registry holds, as the list showed it before the form opened.
    pub original_name: String,
    /// The revision the registry answered with, sent back so a lost update is a refusal.
    pub revision: u32,
    /// The values as the registry answered with them, flattened.
    pub before: BTreeMap<String, String>,
    /// What the form holds now, keyed the same way.
    pub after: BTreeMap<String, String>,
    /// The field types, so a list field is sent as a list.
    pub types: BTreeMap<String, String>,
}

impl PersonEdit {
    pub fn body(&self) -> Result<Value, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("A person needs a name.".to_owned());
        }
        let mut values = serde_json::Map::new();
        for (key, text) in &self.after {
            let before = self.before.get(key).map(String::as_str).unwrap_or("");
            if text.trim() == before.trim() {
                continue;
            }
            let field_type = self.types.get(key).map(String::as_str).unwrap_or("text");
            // An emptied field is cleared, not stored blank: the registry reads null that way.
            let value = if text.trim().is_empty() {
                Value::Null
            } else {
                typed_as(key, field_type, text)?.unwrap_or(Value::Null)
            };
            values.insert(key.clone(), value);
        }
        let mut body = json!({ "values": values, "expected_revision": self.revision });
        if name != self.original_name.trim() {
            body["name"] = json!(name);
        }
        Ok(body)
    }
}

/// A dated fact: someone lives somewhere, or is away for a while.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactDraft {
    pub predicate: String,
    pub place: String,
    pub from: String,
    pub to: String,
    pub note: String,
}

impl FactDraft {
    /// A home base has no end, and an away period needs both dates and an end that follows the
    /// start. Named here rather than by the registry so the field that is wrong is the one
    /// described.
    pub fn check(&self) -> Result<(), String> {
        if !FACT_PREDICATES
            .iter()
            .any(|(predicate, _)| *predicate == self.predicate)
        {
            return Err("Choose a home base or an away period.".to_owned());
        }
        if self.place.trim().is_empty() {
            return Err("A fact needs a place.".to_owned());
        }
        for day in [&self.from, &self.to] {
            let day = day.trim();
            if !day.is_empty() && !is_day(day) {
                return Err(format!("{day:?} is not a YYYY-MM-DD date."));
            }
        }
        if self.predicate == "away" {
            if self.from.trim().is_empty() || self.to.trim().is_empty() {
                return Err("An away period needs both a start and an end.".to_owned());
            }
            if self.to.trim() < self.from.trim() {
                return Err("The end of an away period has to follow its start.".to_owned());
            }
        }
        Ok(())
    }

    pub fn body(&self) -> Result<Value, String> {
        self.check()?;
        let optional = |text: &str| {
            let text = text.trim();
            (!text.is_empty()).then(|| json!(text))
        };
        Ok(json!({
            "predicate": self.predicate,
            "place": self.place.trim(),
            "valid_from": optional(&self.from),
            "valid_to": optional(&self.to),
            "note": optional(&self.note),
        }))
    }
}

/// A field the operator declares, so the next person can carry it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldDecl {
    pub key: String,
    pub label: String,
    /// One of the registry's field types.
    pub field_type: String,
    pub options: String,
}

impl FieldDecl {
    pub fn body(&self) -> Result<Value, String> {
        let key = self.key.trim();
        let label = self.label.trim();
        if !is_key(key) {
            return Err(
                "A field key is snake_case: a lower-case letter, then letters, digits or _."
                    .to_owned(),
            );
        }
        if label.is_empty() {
            return Err("A field needs a label.".to_owned());
        }
        let options: Vec<String> = self
            .options
            .split(',')
            .map(str::trim)
            .filter(|option| !option.is_empty())
            .map(str::to_owned)
            .collect();
        if self.field_type == "enum" && options.is_empty() {
            return Err("An enum field needs at least one option.".to_owned());
        }
        if self.field_type != "enum" && !options.is_empty() {
            return Err("Only an enum field takes options.".to_owned());
        }
        Ok(json!({
            "kind": "person",
            "key": key,
            "label": label,
            "field_type": self.field_type,
            "options": options,
            "data_class": "C2",
        }))
    }
}

/// A `YYYY-MM-DD` by shape, as the registry checks it.
pub fn is_day(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u8 = value[5..7].parse().unwrap_or(0);
    let day: u8 = value[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 40
        && key.as_bytes()[0].is_ascii_lowercase()
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The shape a typed field's text takes. A list field splits on commas, which is how it was shown;
/// a date is refused here when it cannot be stored. The registry checks the value again.
pub fn typed_as(key: &str, field_type: &str, text: &str) -> Result<Option<Value>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(match field_type {
        "emails" | "phones" | "tags" => {
            let items: Vec<Value> = text
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| json!(item))
                .collect();
            if items.is_empty() {
                return Ok(None);
            }
            Value::Array(items)
        }
        "date" => {
            if !is_day(text) {
                return Err(format!("{key:?}: {text:?} is not a YYYY-MM-DD date."));
            }
            json!(text)
        }
        "number" => {
            let number: f64 = text
                .parse()
                .map_err(|_| format!("{key:?}: {text:?} is not a number."))?;
            json!(number)
        }
        "bool" => json!(matches!(text, "true" | "yes" | "1")),
        _ => json!(text),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Vec<EntityField> {
        vec![
            EntityField {
                kind: "person".into(),
                key: "emails".into(),
                label: "Emails".into(),
                field_type: "emails".into(),
                options: Vec::new(),
                data_class: "C2".into(),
                builtin: true,
            },
            EntityField {
                kind: "person".into(),
                key: "company".into(),
                label: "Company".into(),
                field_type: "text".into(),
                options: Vec::new(),
                data_class: "C2".into(),
                builtin: true,
            },
            EntityField {
                kind: "person".into(),
                key: "last_contact".into(),
                label: "Last contact".into(),
                field_type: "date".into(),
                options: Vec::new(),
                data_class: "C2".into(),
                builtin: true,
            },
        ]
    }

    #[test]
    fn a_new_person_needs_a_name_and_sends_only_the_fields_that_hold_something() {
        let mut types = BTreeMap::new();
        types.insert("emails".to_string(), "emails".to_string());
        let draft = NewPerson {
            name: "  Ada Lovelace ".into(),
            values: vec![
                ("emails".into(), "ada@example.com".into()),
                ("company".into(), "   ".into()),
            ],
            types,
        };
        let body = draft.body().expect("a name and one value is enough");
        assert_eq!(body["name"], "Ada Lovelace");
        assert_eq!(body["kind"], "person");
        assert_eq!(body["values"]["emails"], json!(["ada@example.com"]));
        assert!(body["values"].get("company").is_none());

        let nameless = NewPerson::default();
        assert!(nameless.body().is_err());
    }

    /// The edit sends what changed, clears what was emptied, and carries the revision it read.
    #[test]
    fn an_edit_sends_only_the_fields_the_operator_touched() {
        let mut before = BTreeMap::new();
        before.insert("emails".to_string(), "ada@example.com".to_string());
        before.insert("company".to_string(), "Analytical Engine Inc".to_string());
        let mut after = before.clone();
        after.insert("company".to_string(), String::new());
        after.insert("aliases".to_string(), "Ada, A. A. L.".to_string());
        let mut types = BTreeMap::new();
        types.insert("aliases".to_string(), "tags".to_string());
        let edit = PersonEdit {
            id: "p1".into(),
            name: "Ada Lovelace".into(),
            original_name: "Ada Lovelace".into(),
            revision: 4,
            before,
            after,
            types,
        };
        let body = edit.body().expect("only a clear and one new list");
        assert_eq!(body["expected_revision"], 4);
        assert!(body.get("name").is_none(), "an unchanged name is not sent");
        assert_eq!(body["values"]["company"], Value::Null);
        assert_eq!(body["values"]["aliases"], json!(["Ada", "A. A. L."]));
        assert!(body["values"].get("emails").is_none());
    }

    #[test]
    fn a_renamed_person_sends_the_name() {
        let edit = PersonEdit {
            id: "p1".into(),
            name: "Ada King".into(),
            original_name: "Ada Lovelace".into(),
            revision: 2,
            ..Default::default()
        };
        assert_eq!(edit.body().unwrap()["name"], "Ada King");
    }

    #[test]
    fn a_home_base_needs_a_place_and_an_away_period_needs_both_dates_in_order() {
        let home = FactDraft {
            predicate: "home_base".into(),
            place: "Bonn".into(),
            ..Default::default()
        };
        let body = home.body().expect("a home base has no end");
        assert_eq!(body["place"], "Bonn");
        assert!(body["valid_to"].is_null());

        let away = FactDraft {
            predicate: "away".into(),
            place: "Lisbon".into(),
            from: "2026-11-02".into(),
            ..Default::default()
        };
        assert!(away.body().is_err(), "an away period needs an end");

        let backwards = FactDraft {
            from: "2026-11-05".into(),
            to: "2026-11-02".into(),
            ..away
        };
        assert!(backwards.body().is_err());

        let typo = FactDraft {
            predicate: "home_base".into(),
            place: "Bonn".into(),
            from: "2.11.2026".into(),
            ..Default::default()
        };
        assert!(typo.body().is_err(), "the registry stores the day as text");
    }

    #[test]
    fn a_declared_field_is_snake_case_and_an_enum_needs_options() {
        let good = FieldDecl {
            key: "favourite_drink".into(),
            label: "Favourite drink".into(),
            field_type: "text".into(),
            options: String::new(),
        };
        let body = good.body().expect("a lowercase key with underscores");
        assert_eq!(body["data_class"], "C2");

        let spaced = FieldDecl {
            key: "Favourite Drink".into(),
            ..good.clone()
        };
        assert!(spaced.body().is_err());

        let empty_enum = FieldDecl {
            field_type: "enum".into(),
            ..good.clone()
        };
        assert!(empty_enum.body().is_err());

        let options_on_text = FieldDecl {
            options: "one, two".into(),
            ..good
        };
        assert!(options_on_text.body().is_err());
    }

    /// The editor shows the registry's fields, minus the one the vault computes, plus any key the
    /// person already holds that the registry no longer declares.
    #[test]
    fn the_form_offers_the_declared_fields_and_any_the_person_still_holds() {
        let mut held = BTreeMap::new();
        held.insert("old_address".to_string(), "Bonn".to_string());
        let fields = edit_fields(&declared(), &held);
        let keys: Vec<&str> = fields.iter().map(|field| field.key.as_str()).collect();
        assert_eq!(keys, vec!["emails", "company", "old_address"]);
    }
}

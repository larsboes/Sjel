//! Core domain logic for the People / Contact Registry surface.
//!
//! Parses and organizes entity records of kind `person` into structured contact profiles,
//! initials, contact links, and custom properties.
//!
//! The field names are the registry's, not this module's: `crates/entities` writes the
//! Google-managed keys `emails` and `phones` (plural, and an array of strings), so a parser that
//! only looked for `email` and `phone` found neither and drew no contact row for anybody. Both
//! spellings are accepted here because the registry has written both: the values table is text, and
//! an import or an operator edit can have put either in it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::shell::{EntityFact, EntityRecord};

pub mod edit;
pub mod graph;

/// The value behind one key in `EntityRecord::values`.
///
/// Two shapes reach this: the registry's API serves `{ value, source, updated_at }` per key
/// (`crates/entities/src/model.rs`), while the Mac's mirrored copy holds the bare value. A
/// parser that knew only the second showed no email, phone, company or place for anybody on the
/// Mac, because every value there is an object and was skipped.
pub fn field_value(raw: &serde_json::Value) -> Option<&serde_json::Value> {
    match raw {
        serde_json::Value::Object(map) if map.contains_key("value") => map.get("value"),
        other => Some(other),
    }
}

/// Who wrote a value (`operator`, `google`, an import). `None` when the shape carries no source.
pub fn field_source(raw: &serde_json::Value) -> Option<&str> {
    raw.as_object()?.get("source")?.as_str()
}

/// A value as the operator reads it: a list joined with `, `, which is also how the editor takes
/// it back, because every list field this registry has is a list of short strings.
pub fn field_text(raw: &serde_json::Value) -> Option<String> {
    match field_value(raw)? {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(field_text)
                .filter(|text| !text.is_empty())
                .collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        serde_json::Value::Object(_) => None,
    }
}

/// A parsed person ready for contact card and list rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub note_ref: Option<String>,
    /// The revision the registry answered with, sent back on a write so a lost update is refused
    /// rather than silently overwriting a change made elsewhere.
    pub revision: u32,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub role: Option<String>,
    pub location: Option<String>,
    pub tags: Vec<String>,
    pub custom_fields: Vec<(String, String)>,
    /// Every value the registry holds, flattened for the editor, keyed as the registry keys it.
    pub values: BTreeMap<String, String>,
    /// Who wrote each value, so the editor can say when a field belongs to Google.
    pub sources: BTreeMap<String, String>,
    /// Dated place facts: home base, and away periods.
    pub facts: Vec<EntityFact>,
}

impl Person {
    /// Builds a Person from an EntityRecord.
    pub fn from_record(record: &EntityRecord) -> Self {
        let initials = extract_initials(&record.name);

        let mut email = None;
        let mut phone = None;
        let mut company = None;
        let mut role = None;
        let mut location = None;
        let mut tags = Vec::new();
        let mut custom_fields = Vec::new();
        let mut values = BTreeMap::new();
        let mut sources = BTreeMap::new();

        for (key, raw) in &record.values {
            if let Some(source) = field_source(raw) {
                sources.insert(key.clone(), source.to_owned());
            }
            let text = field_text(raw).filter(|text| !text.is_empty());
            if let Some(text) = &text {
                values.insert(key.clone(), text.clone());
            }
            if key == "tags" {
                let items: Vec<String> = field_value(raw)
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                tags.extend(items);
            }
            let Some(str_val) = text else { continue };

            match key.to_ascii_lowercase().as_str() {
                "email" | "emails" | "mail" => email = Some(str_val),
                "phone" | "phones" | "mobile" | "tel" => phone = Some(str_val),
                "company" | "org" | "organization" => company = Some(str_val),
                "role" | "title" | "position" => role = Some(str_val),
                "location" | "city" | "place" => location = Some(str_val),
                "tags" => {}
                key if NETWORK_ROWS.iter().any(|(k, _)| *k == key) => {}
                _ => custom_fields.push((key.clone(), str_val)),
            }
        }

        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            initials,
            note_ref: record.note_ref.clone(),
            revision: record.revision,
            email,
            phone,
            company,
            role,
            location,
            tags,
            custom_fields,
            values,
            sources,
            facts: record.facts.clone(),
        }
    }

    /// The text a field holds right now, or empty when the registry does not hold it.
    pub fn value(&self, key: &str) -> String {
        self.values.get(key).cloned().unwrap_or_default()
    }
    /// The day-valued or link fields that say how the operator knows this person, labelled, in
    /// display order. Absent fields are left out.
    pub fn network_rows(&self) -> Vec<(&'static str, String)> {
        NETWORK_ROWS
            .iter()
            .filter_map(|(key, label)| self.values.get(*key).map(|v| (*label, v.clone())))
            .collect()
    }

    pub fn follow_up_due(&self) -> Option<NaiveDate> {
        let last = self.values.get("last_contact")?.parse::<NaiveDate>().ok()?;
        let weeks = self
            .values
            .get(entities_api::FOLLOW_UP_WEEKS)?
            .parse::<u32>()
            .ok()?;
        (weeks > 0).then_some(())?;
        last.checked_add_days(Days::new(u64::from(weeks) * 7))
    }

    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        self.follow_up_due().is_some_and(|due| due < today)
    }

    /// Every source that wrote a value for this person.
    pub fn origins(&self) -> BTreeSet<&str> {
        self.sources.values().map(String::as_str).collect()
    }

    /// Whether this person belongs under `origin`. `Manual` means no import wrote any value.
    pub fn has_origin(&self, origin: Origin) -> bool {
        let origins = self.origins();
        match origin {
            Origin::All => true,
            Origin::Linkedin => origins.contains("linkedin"),
            Origin::Google => origins.contains("google"),
            Origin::Manual => !origins.contains("linkedin") && !origins.contains("google"),
        }
    }
}

/// Registry keys that `network_rows` shows, with their labels.
const NETWORK_ROWS: &[(&str, &str)] = &[
    ("linkedin", "LinkedIn"),
    ("connected_on", "Connected"),
    ("last_contact", "Last contact"),
    ("met_at", "How we met"),
    (entities_api::FOLLOW_UP_WEEKS, "Follow-up interval (weeks)"),
];

/// Which people the list shows, by the source that wrote their values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    All,
    Linkedin,
    Google,
    Manual,
}

impl Origin {
    pub const ALL: [Origin; 4] = [
        Origin::All,
        Origin::Linkedin,
        Origin::Google,
        Origin::Manual,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Origin::All => "All",
            Origin::Linkedin => "LinkedIn",
            Origin::Google => "Google",
            Origin::Manual => "Added by you",
        }
    }
}

/// How the list is ordered. The date orders put the most recent first and people without the
/// date last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Name,
    LastContact,
    Connected,
}

impl Order {
    pub fn label(self) -> &'static str {
        match self {
            Order::Name => "Name",
            Order::LastContact => "Last contact",
            Order::Connected => "Connected",
        }
    }

    /// The next order, for a button that cycles.
    pub fn next(self) -> Self {
        match self {
            Order::Name => Order::LastContact,
            Order::LastContact => Order::Connected,
            Order::Connected => Order::Name,
        }
    }

    /// The registry key this order sorts by, or `None` for the name.
    pub fn key(self) -> Option<&'static str> {
        match self {
            Order::Name => None,
            Order::LastContact => Some("last_contact"),
            Order::Connected => Some("connected_on"),
        }
    }
}

/// The list as shown: `filter_people`, narrowed to `origin`, in `order`.
pub fn view_people<'a>(
    people: &'a [Person],
    query: &str,
    origin: Origin,
    order: Order,
) -> Vec<&'a Person> {
    let mut shown: Vec<&Person> = filter_people(people, query)
        .into_iter()
        .filter(|p| p.has_origin(origin))
        .collect();
    let by_name = |a: &&Person, b: &&Person| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    match order.key() {
        None => shown.sort_by(by_name),
        Some(key) => shown.sort_by(|a, b| {
            // Days are `YYYY-MM-DD`, so text order is date order; `None` sorts after any day.
            let (x, y) = (a.values.get(key), b.values.get(key));
            match (x, y) {
                (Some(x), Some(y)) => y.cmp(x).then_with(|| by_name(a, b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => by_name(a, b),
            }
        }),
    }
    shown
}

/// Compute initials from a name (e.g. "Ada Lovelace" -> "AL", "Linus" -> "L").
pub fn extract_initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => "?".to_string(),
        [single] => single
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string()),
        [first, .., last] => {
            let f = first.chars().next().unwrap_or('?').to_uppercase();
            let l = last.chars().next().unwrap_or('?').to_uppercase();
            format!("{f}{l}")
        }
    }
}

/// Filters people by search query against name, aliases, company, role, location, or email.
pub fn filter_people<'a>(people: &'a [Person], query: &str) -> Vec<&'a Person> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return people.iter().collect();
    }
    people
        .iter()
        .filter(|p| {
            [
                Some(&p.name),
                p.values.get("aliases"),
                p.company.as_ref(),
                p.role.as_ref(),
                p.location.as_ref(),
                p.email.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|field| field.to_lowercase().contains(&q))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_searched_and_follow_up_becomes_overdue_after_its_due_day() {
        let mut record = EntityRecord {
            name: "Ada Lovelace".into(),
            ..Default::default()
        };
        record.values.insert(
            "aliases".into(),
            serde_json::json!({
                "value": ["Àda", "A. L."], "source": "operator"
            }),
        );
        record
            .values
            .insert("last_contact".into(), serde_json::json!("2026-12-24"));
        record
            .values
            .insert(entities_api::FOLLOW_UP_WEEKS.into(), serde_json::json!(2));
        let person = Person::from_record(&record);
        assert_eq!(
            filter_people(std::slice::from_ref(&person), " ÀDA "),
            vec![&person]
        );
        let due = NaiveDate::from_ymd_opt(2027, 1, 7).unwrap();
        assert_eq!(person.follow_up_due(), Some(due));
        assert!(!person.is_overdue(due));
        assert!(person.is_overdue(due.succ_opt().unwrap()));

        record.values.remove("last_contact");
        assert_eq!(Person::from_record(&record).follow_up_due(), None);
        record
            .values
            .insert("last_contact".into(), serde_json::json!("2026-02-30"));
        assert_eq!(Person::from_record(&record).follow_up_due(), None);
    }

    fn sourced(name: &str, values: &[(&str, &str, &str)]) -> Person {
        let values = values
            .iter()
            .map(|(k, v, source)| {
                (
                    k.to_string(),
                    serde_json::json!({ "value": v, "source": source, "updated_at": "2026-11-19" }),
                )
            })
            .collect();
        Person::from_record(&EntityRecord {
            id: name.into(),
            kind: "person".into(),
            name: name.into(),
            note_ref: None,
            revision: 1,
            values,
            facts: Vec::new(),
        })
    }

    #[test]
    fn origin_narrows_and_date_orders_put_recent_first() {
        let people = vec![
            sourced("Cy", &[("company", "Acme", "operator")]),
            sourced(
                "Ada",
                &[
                    ("company", "Acme", "linkedin"),
                    ("connected_on", "2024-03-15", "linkedin"),
                    ("last_contact", "2026-01-02", "linkedin"),
                ],
            ),
            sourced(
                "Bo",
                &[
                    ("connected_on", "2025-06-01", "linkedin"),
                    ("emails", "bo@example.org", "google"),
                ],
            ),
        ];
        let names = |shown: Vec<&Person>| shown.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(view_people(&people, "", Origin::All, Order::Name)),
            ["Ada", "Bo", "Cy"]
        );
        assert_eq!(
            names(view_people(&people, "", Origin::Linkedin, Order::Name)),
            ["Ada", "Bo"]
        );
        assert_eq!(
            names(view_people(&people, "", Origin::Google, Order::Name)),
            ["Bo"]
        );
        assert_eq!(
            names(view_people(&people, "", Origin::Manual, Order::Name)),
            ["Cy"]
        );
        assert_eq!(
            names(view_people(&people, "", Origin::All, Order::Connected)),
            ["Bo", "Ada", "Cy"]
        );
        assert_eq!(
            names(view_people(
                &people,
                "acme",
                Origin::All,
                Order::LastContact
            )),
            ["Ada", "Cy"]
        );
    }

    #[test]
    fn network_fields_are_labelled_rows_not_custom_fields() {
        let ada = sourced(
            "Ada",
            &[
                ("last_contact", "2026-01-02", "journal"),
                ("linkedin", "https://www.linkedin.com/in/ada", "linkedin"),
            ],
        );
        assert!(ada.custom_fields.is_empty());
        assert_eq!(
            ada.network_rows(),
            vec![
                ("LinkedIn", "https://www.linkedin.com/in/ada".to_string()),
                ("Last contact", "2026-01-02".to_string()),
            ]
        );
    }
    use std::collections::BTreeMap;

    /// The shape the registry actually writes: `emails` and `phones` as arrays, from
    /// `crates/entities/src/google.rs`. Before this was the key set, every person's email and
    /// phone were `None` and the contact card drew no row for either.
    #[test]
    fn the_registrys_own_field_names_reach_the_contact_card() {
        let mut values = BTreeMap::new();
        values.insert("emails".into(), serde_json::json!(["ada@example.com"]));
        values.insert("phones".into(), serde_json::json!(["+44 20 7946 0958"]));
        values.insert("company".into(), serde_json::json!("Analytical Engine Inc"));
        values.insert("birthday".into(), serde_json::json!("1815-12-10"));

        let record = EntityRecord {
            id: "p1".into(),
            kind: "person".into(),
            name: "Ada Lovelace".into(),
            note_ref: None,
            revision: 3,
            values,
            facts: Vec::new(),
        };

        let person = Person::from_record(&record);
        assert_eq!(person.email.as_deref(), Some("ada@example.com"));
        assert_eq!(person.phone.as_deref(), Some("+44 20 7946 0958"));
        assert_eq!(person.company.as_deref(), Some("Analytical Engine Inc"));
        // Named fields do not repeat as custom rows, and an unnamed one still gets one.
        assert_eq!(
            person.custom_fields,
            vec![("birthday".to_string(), "1815-12-10".to_string())]
        );
    }

    /// What the API serves, and what the Mac reads: each value is wrapped with its provenance.
    /// Read as a bare value, every one of these was an object and every field was lost.
    #[test]
    fn the_apis_wrapped_values_reach_the_card_and_the_editor() {
        let wrapped = |value: serde_json::Value, source: &str| serde_json::json!({ "value": value, "source": source, "updated_at": "2026-10-25" });
        let mut values = BTreeMap::new();
        values.insert(
            "emails".into(),
            wrapped(serde_json::json!(["ada@example.com"]), "google"),
        );
        values.insert(
            "company".into(),
            wrapped(serde_json::json!("Analytical Engine Inc"), "operator"),
        );
        values.insert(
            "aliases".into(),
            wrapped(serde_json::json!(["Ada", "A. A. L."]), "operator"),
        );
        let record = EntityRecord {
            id: "p1".into(),
            kind: "person".into(),
            name: "Ada Lovelace".into(),
            note_ref: None,
            revision: 7,
            values,
            facts: Vec::new(),
        };

        let person = Person::from_record(&record);
        assert_eq!(person.email.as_deref(), Some("ada@example.com"));
        assert_eq!(person.company.as_deref(), Some("Analytical Engine Inc"));
        assert_eq!(person.value("aliases"), "Ada, A. A. L.");
        assert_eq!(
            person.sources.get("emails").map(String::as_str),
            Some("google")
        );
        assert_eq!(person.revision, 7);
        // The editor takes a list back the way it showed it.
        assert_eq!(
            person.custom_fields,
            vec![("aliases".to_string(), "Ada, A. A. L.".to_string())]
        );
    }

    #[test]
    fn initials_extraction() {
        assert_eq!(extract_initials("Alan Turing"), "AT");
        assert_eq!(extract_initials("Grace Brewster Murray Hopper"), "GH");
        assert_eq!(extract_initials("Dennis"), "D");
        assert_eq!(extract_initials(""), "?");
    }

    #[test]
    fn person_parsing_and_filtering() {
        let mut values = BTreeMap::new();
        values.insert("email".into(), serde_json::json!("ada@example.com"));
        values.insert("company".into(), serde_json::json!("Analytical Engine Inc"));
        values.insert("role".into(), serde_json::json!("Chief Mathematician"));

        let record = EntityRecord {
            id: "p1".into(),
            kind: "person".into(),
            name: "Ada Lovelace".into(),
            note_ref: Some("Atlas/People/Ada.md".into()),
            revision: 1,
            values,
            facts: Vec::new(),
        };

        let person = Person::from_record(&record);
        assert_eq!(person.initials, "AL");
        assert_eq!(person.email.as_deref(), Some("ada@example.com"));
        assert_eq!(person.company.as_deref(), Some("Analytical Engine Inc"));

        let list = vec![person];
        assert_eq!(filter_people(&list, "lovelace").len(), 1);
        assert_eq!(filter_people(&list, "mathematician").len(), 1);
        assert_eq!(filter_people(&list, "unknown").len(), 0);
    }
}

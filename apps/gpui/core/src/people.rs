//! Core domain logic for the People / Contact Registry surface.
//!
//! Parses and organizes entity records of kind `person` into structured contact profiles,
//! initials, contact links, and custom properties.
//!
//! The field names are the registry's, not this module's: `capabilities/entities` writes the
//! Google-managed keys `emails` and `phones` (plural, and an array of strings), so a parser that
//! only looked for `email` and `phone` found neither and drew no contact row for anybody. Both
//! spellings are accepted here because the registry has written both: the values table is text, and
//! an import or an operator edit can have put either in it.

use serde::{Deserialize, Serialize};

use crate::shell::EntityRecord;

/// A parsed person ready for contact card and list rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub note_ref: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub role: Option<String>,
    pub location: Option<String>,
    pub tags: Vec<String>,
    pub custom_fields: Vec<(String, String)>,
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

        for (key, val) in &record.values {
            let str_val = match val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Array(arr) => {
                    if key == "tags" {
                        tags.extend(arr.iter().filter_map(|v| v.as_str()).map(str::to_string));
                    }
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
                _ => continue,
            };

            match key.to_ascii_lowercase().as_str() {
                "email" | "emails" | "mail" => email = Some(str_val),
                "phone" | "phones" | "mobile" | "tel" => phone = Some(str_val),
                "company" | "org" | "organization" => company = Some(str_val),
                "role" | "title" | "position" => role = Some(str_val),
                "location" | "city" | "place" => location = Some(str_val),
                "tags" => {}
                _ => custom_fields.push((key.clone(), str_val)),
            }
        }

        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            initials,
            note_ref: record.note_ref.clone(),
            email,
            phone,
            company,
            role,
            location,
            tags,
            custom_fields,
        }
    }
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

/// Filters people by search query against name, company, role, location, or email.
pub fn filter_people<'a>(people: &'a [Person], query: &str) -> Vec<&'a Person> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return people.iter().collect();
    }
    people
        .iter()
        .filter(|p| {
            p.name.to_ascii_lowercase().contains(&q)
                || p.company
                    .as_ref()
                    .is_some_and(|c| c.to_ascii_lowercase().contains(&q))
                || p.role
                    .as_ref()
                    .is_some_and(|r| r.to_ascii_lowercase().contains(&q))
                || p.location
                    .as_ref()
                    .is_some_and(|l| l.to_ascii_lowercase().contains(&q))
                || p.email
                    .as_ref()
                    .is_some_and(|e| e.to_ascii_lowercase().contains(&q))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The shape the registry actually writes: `emails` and `phones` as arrays, from
    /// `capabilities/entities/src/google.rs`. Before this was the key set, every person's email and
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

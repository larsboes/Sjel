//! Bringing records from another system into entities: Obsidian notes and Google Contacts
//! (PRD Q117: both are adapters, inbound).
//!
//! One rule decides every value: **a source may change a value only while that source owns
//! it.** A source owns a value it wrote itself, or a key that has no value yet. So a phone
//! number typed on the People page is never overwritten by Google, and a relation that came
//! from a note follows the note until someone edits it elsewhere.
//!
//! A record is matched to an entity by its external id. Failing that, by name: exactly one
//! person of that name who is not yet linked to the same system. Failing that, it creates
//! a person. A name matching two people creates a third rather than guessing.
//!
//! The decisions are pure functions over data (`value_changes`, `home_change`, `choose`),
//! tested without a store; `apply` only sequences them.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::model::{check_value, Fact, FieldDef, FieldValue};
use crate::places::{self, Resolved};
use crate::store::{EntitiesStore, NewFact, Patch, StoreError};

/// Where a record says a person lives. A coordinate from the source is used as given;
/// otherwise the place text is geocoded through places.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingPlace {
    pub place: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// One person as another system describes them.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub external_id: String,
    pub etag: Option<String>,
    pub name: String,
    pub note_ref: Option<String>,
    /// The values this source has for the keys it manages. A managed key missing here means
    /// the source has no value, and a value it owns is cleared.
    pub values: BTreeMap<String, Value>,
    pub home: Option<IncomingPlace>,
}

/// What one sync run did.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub struct SyncReport {
    pub records: usize,
    pub created: usize,
    pub linked_by_name: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub homes_set: usize,
    /// Records whose entity the operator deleted; skipped, never recreated.
    pub excluded: usize,
    /// "<record>: <key>: <reason>", for values the field registry refused.
    pub refused: Vec<String>,
}

/// The value writes a source may make: set what it has and owns, clear what it owned and no
/// longer has. `null` in the result means clear.
pub fn value_changes(
    current: &BTreeMap<String, FieldValue>,
    desired: &BTreeMap<String, Value>,
    managed: &[&str],
    source: &str,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for key in managed {
        let held = current.get(*key);
        if held.is_some_and(|v| v.source != source) {
            continue;
        }
        match (desired.get(*key), held) {
            (Some(want), Some(have)) if &have.value == want => {}
            (Some(want), _) => {
                out.insert((*key).to_string(), want.clone());
            }
            (None, Some(_)) => {
                out.insert((*key).to_string(), Value::Null);
            }
            (None, None) => {}
        }
    }
    out
}

/// The home-base facts to delete and the one to add, for the facts this source wrote. A
/// home base someone else wrote is never touched; one this source wrote with the same place
/// is kept as it is.
pub fn home_change(
    facts: &[Fact],
    desired: Option<&IncomingPlace>,
    source: &str,
) -> (Vec<String>, Option<IncomingPlace>) {
    let own: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.predicate == "home_base" && f.source == source)
        .collect();
    let Some(want) = desired else {
        return (own.iter().map(|f| f.id.clone()).collect(), None);
    };
    let same = |f: &Fact| f.place.trim().eq_ignore_ascii_case(want.place.trim());
    if let Some(keep) = own.iter().copied().find(|f| same(f)) {
        let delete = own
            .iter()
            .filter(|f| f.id != keep.id)
            .map(|f| f.id.clone())
            .collect();
        return (delete, None);
    }
    (
        own.iter().map(|f| f.id.clone()).collect(),
        Some(want.clone()),
    )
}

/// Which person a record belongs to: its linked entity, else the one unlinked person of the
/// same name, else none (create one).
pub enum Choice {
    Linked(String),
    ByName(String),
    Create,
}

pub fn choose(
    linked: Option<String>,
    name: &str,
    people: &[(String, String)],
    linked_to_system: &HashSet<String>,
) -> Choice {
    if let Some(id) = linked {
        return Choice::Linked(id);
    }
    let wanted = name.trim().to_lowercase();
    let matches: Vec<&String> = people
        .iter()
        .filter(|(id, person)| {
            person.trim().to_lowercase() == wanted && !linked_to_system.contains(id)
        })
        .map(|(id, _)| id)
        .collect();
    match matches.as_slice() {
        [one] => Choice::ByName((*one).clone()),
        _ => Choice::Create,
    }
}

/// Drops the values the field registry refuses, recording why, so one bad birthday does not
/// stop a whole contact.
fn checked(
    record: &str,
    values: BTreeMap<String, Value>,
    fields: &[FieldDef],
    refused: &mut Vec<String>,
) -> BTreeMap<String, Value> {
    values
        .into_iter()
        .filter(|(key, value)| {
            if value.is_null() {
                return true;
            }
            let Some(def) = fields.iter().find(|f| &f.key == key) else {
                refused.push(format!("{record}: {key}: no such person field"));
                return false;
            };
            match check_value(def, value) {
                Ok(_) => true,
                Err(reason) => {
                    refused.push(format!("{record}: {key}: {reason}"));
                    false
                }
            }
        })
        .collect()
}

/// Applies records from one system. `system` names the external-id namespace and is also
/// the source every written value and fact carries.
pub fn apply(
    store: &EntitiesStore,
    places_url: &str,
    system: &str,
    managed: &[&str],
    records: &[Incoming],
    dry_run: bool,
) -> Result<SyncReport, StoreError> {
    let fields = store.fields(Some("person"))?;
    let mut report = SyncReport {
        records: records.len(),
        ..SyncReport::default()
    };
    let mut linked_to_system = store.linked_entity_ids(system)?;
    for record in records {
        if store.is_excluded(system, &record.external_id)? {
            report.excluded += 1;
            continue;
        }
        let people: Vec<(String, String)> = store
            .list(Some("person"), None)?
            .into_iter()
            .map(|e| (e.id, e.name))
            .collect();
        let linked = store.external(system, &record.external_id)?;
        let entity = match choose(linked, &record.name, &people, &linked_to_system) {
            Choice::Linked(id) => store.get(&id)?,
            Choice::ByName(id) => {
                report.linked_by_name += 1;
                store.get(&id)?
            }
            Choice::Create => {
                report.created += 1;
                if dry_run {
                    continue;
                }
                Some(store.create(
                    "person",
                    &record.name,
                    record.note_ref.as_deref(),
                    &BTreeMap::new(),
                    system,
                )?)
            }
        };
        let Some(entity) = entity else {
            continue;
        };
        let changes = checked(
            &record.name,
            value_changes(&entity.values, &record.values, managed, system),
            &fields,
            &mut report.refused,
        );
        let (delete, add) = home_change(&entity.facts, record.home.as_ref(), system);
        let note_changes = record.note_ref.is_some() && entity.note_ref.is_none();
        if changes.is_empty() && delete.is_empty() && add.is_none() && !note_changes {
            report.unchanged += 1;
        } else {
            report.updated += 1;
        }
        if dry_run {
            continue;
        }
        if !changes.is_empty() || note_changes {
            store.patch(
                &entity.id,
                &Patch {
                    note_ref: note_changes.then(|| record.note_ref.clone()),
                    values: changes,
                    source: system.to_string(),
                    ..Patch::default()
                },
            )?;
        }
        for fact_id in delete {
            store.delete_fact(&entity.id, &fact_id)?;
        }
        if let Some(mut home) = add {
            if home.latitude.is_none() {
                if let Resolved::At {
                    latitude,
                    longitude,
                    ..
                } = places::resolve(places_url, &home.place)
                {
                    home.latitude = Some(latitude);
                    home.longitude = Some(longitude);
                }
            }
            store.add_fact(
                &entity.id,
                &NewFact {
                    predicate: "home_base".into(),
                    place: home.place,
                    latitude: home.latitude,
                    longitude: home.longitude,
                    source: system.to_string(),
                    ..NewFact::default()
                },
            )?;
            report.homes_set += 1;
        }
        store.link_external(
            system,
            &record.external_id,
            &entity.id,
            record.etag.as_deref(),
        )?;
        linked_to_system.insert(entity.id);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn held(value: Value, source: &str) -> FieldValue {
        FieldValue {
            value,
            source: source.into(),
            updated_at: "0".into(),
        }
    }

    #[test]
    fn a_source_changes_only_what_it_owns() {
        let current = BTreeMap::from([
            ("phones".to_string(), held(json!(["+49 1"]), "operator")),
            ("company".to_string(), held(json!("Old GmbH"), "google")),
            ("role".to_string(), held(json!("Dev"), "google")),
        ]);
        let desired = BTreeMap::from([
            ("phones".to_string(), json!(["+49 2"])),
            ("company".to_string(), json!("New GmbH")),
            ("emails".to_string(), json!(["r@example.org"])),
        ]);
        let changes = value_changes(
            &current,
            &desired,
            &["phones", "company", "role", "emails"],
            "google",
        );
        assert_eq!(
            changes,
            BTreeMap::from([
                ("company".to_string(), json!("New GmbH")),
                ("emails".to_string(), json!(["r@example.org"])),
                ("role".to_string(), Value::Null),
            ]),
            "the operator's phone stays; the company follows Google; a role Google dropped is cleared"
        );
    }

    fn home(id: &str, place: &str, source: &str) -> Fact {
        Fact {
            id: id.into(),
            entity_id: "e".into(),
            predicate: "home_base".into(),
            place: place.into(),
            latitude: None,
            longitude: None,
            valid_from: None,
            valid_to: None,
            note: None,
            source: source.into(),
            created_at: "0".into(),
        }
    }

    #[test]
    fn a_synced_home_is_replaced_and_someone_elses_is_left() {
        let facts = vec![
            home("mine", "Köln", "google"),
            home("theirs", "Bonn", "operator"),
        ];
        let bonn = IncomingPlace {
            place: "Berlin".into(),
            latitude: None,
            longitude: None,
        };
        let (delete, add) = home_change(&facts, Some(&bonn), "google");
        assert_eq!(delete, vec!["mine".to_string()]);
        assert_eq!(add, Some(bonn));

        let same = IncomingPlace {
            place: "köln".into(),
            latitude: None,
            longitude: None,
        };
        assert_eq!(home_change(&facts, Some(&same), "google"), (vec![], None));
        assert_eq!(
            home_change(&facts, None, "google"),
            (vec!["mine".to_string()], None)
        );
    }

    #[test]
    fn a_name_matches_only_one_unlinked_person() {
        let people = vec![
            ("a".to_string(), "Ron".to_string()),
            ("b".to_string(), "Anna".to_string()),
            ("c".to_string(), "Anna".to_string()),
        ];
        let none = HashSet::new();
        assert!(matches!(choose(None, " ron ", &people, &none), Choice::ByName(id) if id == "a"));
        assert!(
            matches!(choose(None, "Anna", &people, &none), Choice::Create),
            "two Annas: no guess"
        );
        let linked = HashSet::from(["a".to_string()]);
        assert!(matches!(
            choose(None, "Ron", &people, &linked),
            Choice::Create
        ));
        assert!(
            matches!(choose(Some("z".into()), "Ron", &people, &none), Choice::Linked(id) if id == "z")
        );
    }
}

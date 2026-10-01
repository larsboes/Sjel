//! The Obsidian adapter, inbound: `Atlas/People` notes as `capabilities/vault` reads them
//! (GET /api/people), mapped to sync records.
//!
//! Only structured keys cross: the ones vault's `PROFILE_SCALARS` and `PROFILE_LISTS` name,
//! `home`/`host`/`host_note`, and the `last_contact` vault computes from Journal backlinks.
//! Prose stays in the note (PRD Q117); the entity links to it through `note_ref`.
//!
//! `last_contact` here is vault's computed value, never the key stored on the note, so the key
//! `vault fields --apply` writes does not flow back through this import.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::sync::{Incoming, IncomingPlace};

/// The person fields this adapter writes. A value one of these holds from another source
/// is left alone (sync.rs, `value_changes`).
pub const MANAGED: &[&str] = &[
    "relation",
    "company",
    "role",
    "birthday",
    "emails",
    "interests",
    "skills",
    "socials",
    "last_contact",
    "sleeping_option",
    "sleeping_note",
];

/// `[[Target|Alias]]` → `Alias`, `[[Target]]` → `Target`. A note writes links where the
/// entity wants text.
pub fn unlink(text: &str) -> String {
    let t = text.trim();
    match t.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        Some(inner) => inner.rsplit('|').next().unwrap_or(inner).trim().to_string(),
        None => t.to_string(),
    }
}

/// `"50.73, 7.10"` → the pair.
fn coordinate(text: &str) -> Option<(f64, f64)> {
    let (lat, lon) = text.trim().trim_matches(['[', ']']).split_once(',')?;
    Some((lat.trim().parse().ok()?, lon.trim().parse().ok()?))
}

/// One vault person (an element of `facts`) as a sync record.
pub fn record(person: &Value) -> Option<Incoming> {
    let id = person["id"].as_str()?;
    let name = person["name"].as_str()?;
    let profile = &person["profile"];
    let text = |key: &str| profile[key].as_str().map(unlink).filter(|t| !t.is_empty());
    let list = |key: &str| {
        profile[key].as_array().map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(unlink)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
        })
    };
    let mut values = BTreeMap::new();
    for key in ["relation", "company", "role", "birthday"] {
        if let Some(v) = text(key) {
            values.insert(key.to_string(), json!(v));
        }
    }
    if let Some(email) = text("email") {
        values.insert("emails".into(), json!([email]));
    }
    for key in ["interests", "skills", "socials"] {
        if let Some(items) = list(key).filter(|i| !i.is_empty()) {
            values.insert(key.to_string(), json!(items));
        }
    }
    if let Some(day) = person["last_contact"].as_str() {
        values.insert("last_contact".into(), json!(day));
    }
    if person["host"].as_bool() == Some(true) {
        values.insert("sleeping_option".into(), json!("ask"));
    }
    if let Some(note) = person["host_note"].as_str() {
        values.insert("sleeping_note".into(), json!(note));
    }
    let home_text = person["home"].as_str().map(str::to_string);
    let home = match (
        home_text,
        profile["coordinates"].as_str().and_then(coordinate),
    ) {
        (place, Some((lat, lon))) => Some(IncomingPlace {
            place: place.unwrap_or_else(|| format!("{lat:.3}, {lon:.3}")),
            latitude: Some(lat),
            longitude: Some(lon),
        }),
        (Some(place), None) => Some(IncomingPlace {
            place,
            latitude: None,
            longitude: None,
        }),
        (None, None) => None,
    };
    Some(Incoming {
        external_id: id.to_string(),
        etag: None,
        name: name.to_string(),
        note_ref: Some(id.to_string()),
        values,
        home,
    })
}

/// Reads every person note through vault.
pub fn fetch(vault_url: &str) -> Result<Vec<Incoming>, String> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("entities-obsidian"),
        Duration::from_secs(60),
    )
    .map_err(|e| e.to_string())?;
    let url = format!("{}/api/people", vault_url.trim_end_matches('/'));
    let request = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url);
    let body: Value = request
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("vault /api/people: {e}"))?;
    Ok(body["facts"]
        .as_array()
        .map(|a| a.iter().filter_map(record).collect())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_maps_to_values_and_a_home() {
        let person = json!({
            "id": "Atlas/People/Ron.md",
            "name": "Ron",
            "last_contact": "2026-09-20",
            "home": "Bonn",
            "host": true,
            "host_note": null,
            "profile": {
                "relation": "[[Colleague]]",
                "company": "[[Acme GmbH|Acme]]",
                "email": "ron@example.org",
                "interests": ["bouldering", "[[Jazz]]"],
                "coordinates": "50.73, 7.10"
            }
        });
        let r = record(&person).unwrap();
        assert_eq!(r.note_ref.as_deref(), Some("Atlas/People/Ron.md"));
        assert_eq!(r.values["relation"], json!("Colleague"));
        assert_eq!(r.values["company"], json!("Acme"));
        assert_eq!(r.values["emails"], json!(["ron@example.org"]));
        assert_eq!(r.values["interests"], json!(["bouldering", "Jazz"]));
        assert_eq!(r.values["last_contact"], json!("2026-09-20"));
        assert_eq!(r.values["sleeping_option"], json!("ask"));
        let home = r.home.unwrap();
        assert_eq!((home.place.as_str(), home.latitude), ("Bonn", Some(50.73)));
    }

    #[test]
    fn a_bare_note_maps_to_a_name_and_nothing_else() {
        let r = record(&json!({ "id": "Atlas/People/X.md", "name": "X", "profile": {} })).unwrap();
        assert!(r.values.is_empty());
        assert!(r.home.is_none());
    }
}

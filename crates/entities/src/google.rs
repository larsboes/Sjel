//! The Google Contacts adapter, inbound, through the People API (PRD Q117).
//!
//! Read only. The token carries `contacts.readonly` and nothing else, and nothing here
//! writes to Google: sending a person's time frames or sleeping option to Google would move
//! C2 into a cloud service (PRD §6.1), so the sync runs one way.
//!
//! **Credentials** are the keychain grant `google-entities`, connected in the desktop app
//! (Settings › Accounts), or the overlay's `config/entities.env` until the grant moves there
//! (`libs/sjel-credentials/src/google.rs`). A separate grant from comms', so Gmail's grant does
//! not widen and this one cannot read mail.
//! No value is ever logged, including in a failed-refresh body.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use crate::sync::{Incoming, IncomingPlace};

/// The person fields this adapter writes.
pub const MANAGED: &[&str] = &["emails", "phones", "birthday", "company", "role"];

const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,birthdays,organizations,addresses";

pub fn env_path() -> PathBuf {
    sjel_config::overlay_config("entities.env").unwrap_or_else(|| PathBuf::from("entities.env"))
}

/// Every connection, all pages.
/// A client and a fresh access token from the overlay's credentials.
fn authorised() -> Result<(reqwest::blocking::Client, String), String> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("entities-google"),
        Duration::from_secs(30),
    )
    .map_err(|e| e.to_string())?;
    let token = sjel_credentials::google::access_token(GRANT_ID, &env_path()).map_err(|e| {
        format!("{e}. Reconnect Google Contacts in the desktop app, Settings › Accounts")
    })?;
    Ok((client, token))
}

/// The credential ID of this capability's Contacts grant, as `service.toml` declares it.
pub const GRANT_ID: &str = "google-entities";

/// One contact as Google holds it now, by its resource name (`people/c…`).
pub fn fetch_one(resource_name: &str) -> Result<Value, String> {
    if !resource_name.starts_with("people/") || resource_name.contains("..") {
        return Err(format!(
            "{resource_name:?} is not a People API resource name"
        ));
    }
    let (client, token) = authorised()?;
    let response = sjel_http::send(
        || {
            client
                .get(format!("https://people.googleapis.com/v1/{resource_name}"))
                .bearer_auth(&token)
                .query(&[("personFields", PERSON_FIELDS)])
        },
        // A read.
        sjel_http::Idempotent::Yes,
    )
    .map_err(|e| format!("people.get: {e}"))?;
    let status = response.status();
    let body: Value = response.json().map_err(|e| format!("people.get: {e}"))?;
    if !status.is_success() {
        let message = body["error"]["message"].as_str().unwrap_or("no message");
        return Err(format!("people.get answered {status}: {message}"));
    }
    Ok(body)
}

pub fn fetch() -> Result<Vec<Value>, String> {
    let (client, token) = authorised()?;
    let mut people = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut query = vec![
            ("personFields", PERSON_FIELDS.to_string()),
            ("pageSize", "1000".to_string()),
        ];
        if let Some(p) = &page {
            query.push(("pageToken", p.clone()));
        }
        let response = sjel_http::send(
            || {
                client
                    .get("https://people.googleapis.com/v1/people/me/connections")
                    .bearer_auth(&token)
                    .query(&query)
            },
            // A read.
            sjel_http::Idempotent::Yes,
        )
        .map_err(|e| format!("people.connections: {e}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|e| format!("people.connections: {e}"))?;
        if !status.is_success() {
            let message = body["error"]["message"].as_str().unwrap_or("no message");
            return Err(format!("people.connections answered {status}: {message}"));
        }
        people.extend(body["connections"].as_array().cloned().unwrap_or_default());
        match body["nextPageToken"].as_str() {
            Some(next) => page = Some(next.to_string()),
            None => return Ok(people),
        }
    }
}

/// A People API date as `YYYY-MM-DD`, or `None` without a year: a birthday with no year is
/// not a date the field can hold.
fn day(date: &Value) -> Option<String> {
    Some(format!(
        "{:04}-{:02}-{:02}",
        date["year"].as_u64().filter(|y| *y > 0)?,
        date["month"].as_u64()?,
        date["day"].as_u64()?
    ))
}

fn strings(items: &Value, key: &str) -> Vec<String> {
    items
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| i[key].as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// One People API person as a sync record. A contact with no name is skipped: a name is
/// what the entity is found by.
pub fn record(person: &Value) -> Option<Incoming> {
    let external_id = person["resourceName"].as_str()?.to_string();
    let name = person["names"][0]["displayName"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    let mut values = BTreeMap::new();
    let emails = strings(&person["emailAddresses"], "value");
    if !emails.is_empty() {
        values.insert("emails".into(), json!(emails));
    }
    let phones = strings(&person["phoneNumbers"], "value");
    if !phones.is_empty() {
        values.insert("phones".into(), json!(phones));
    }
    if let Some(birthday) = person["birthdays"]
        .as_array()
        .and_then(|b| b.iter().find_map(|b| day(&b["date"])))
    {
        values.insert("birthday".into(), json!(birthday));
    }
    let org = &person["organizations"][0];
    if let Some(company) = org["name"].as_str().filter(|s| !s.trim().is_empty()) {
        values.insert("company".into(), json!(company.trim()));
    }
    if let Some(role) = org["title"].as_str().filter(|s| !s.trim().is_empty()) {
        values.insert("role".into(), json!(role.trim()));
    }
    // A home address first, else the first address; only its city (and country) is kept,
    // because a city is what "who is around" matches and a street is more than it needs.
    let addresses = person["addresses"].as_array().cloned().unwrap_or_default();
    let address = addresses
        .iter()
        .find(|a| a["type"].as_str() == Some("home"))
        .or_else(|| addresses.first());
    let home = address.and_then(|a| {
        let city = a["city"]
            .as_str()
            .map(str::trim)
            .filter(|c| !c.is_empty())?;
        let place = match a["country"]
            .as_str()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(country) => format!("{city}, {country}"),
            None => city.to_string(),
        };
        Some(IncomingPlace {
            place,
            latitude: None,
            longitude: None,
        })
    });
    Some(Incoming {
        external_id,
        etag: person["etag"].as_str().map(str::to_string),
        name: name.to_string(),
        note_ref: None,
        values,
        home,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_maps_to_values_and_a_city() {
        let person = json!({
            "resourceName": "people/c123",
            "etag": "%EgU",
            "names": [{ "displayName": "Ron Example" }],
            "emailAddresses": [{ "value": "ron@example.org" }, { "value": " " }],
            "phoneNumbers": [{ "value": "+49 228 0000" }],
            "birthdays": [{ "date": { "month": 3, "day": 12 } }, { "date": { "year": 1995, "month": 3, "day": 12 } }],
            "organizations": [{ "name": "Acme", "title": "Engineer" }],
            "addresses": [
                { "type": "work", "city": "Köln" },
                { "type": "home", "city": "Bonn", "country": "Germany", "streetAddress": "Hauptstr. 1" }
            ]
        });
        let r = record(&person).unwrap();
        assert_eq!(r.external_id, "people/c123");
        assert_eq!(r.values["emails"], json!(["ron@example.org"]));
        assert_eq!(
            r.values["birthday"],
            json!("1995-03-12"),
            "the dated birthday, not the yearless one"
        );
        assert_eq!(r.values["company"], json!("Acme"));
        assert_eq!(r.values["role"], json!("Engineer"));
        assert_eq!(
            r.home.unwrap().place,
            "Bonn, Germany",
            "home over work, city not street"
        );
    }

    #[test]
    fn a_nameless_contact_is_skipped() {
        assert!(record(&json!({ "resourceName": "people/c1", "names": [] })).is_none());
    }
}

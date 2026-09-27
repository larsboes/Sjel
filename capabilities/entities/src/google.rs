//! The Google Contacts adapter, inbound, through the People API (PRD Q117).
//!
//! Read only. The token carries `contacts.readonly` and nothing else, and nothing here
//! writes to Google: sending a person's time frames or sleeping option to Google would move
//! C2 into a cloud service (PRD §6.1), so the sync runs one way.
//!
//! **Credentials** come from the overlay's `config/entities.env`: `GOOGLE_CLIENT_ID`,
//! `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`, the same three keys and file shape
//! `capabilities/comms` uses. A separate token from comms', so Gmail's grant does not widen
//! and this one cannot read mail. Mint it with
//! `bun capabilities/comms/auth/get-refresh-token.ts --env entities.env --scope contacts.readonly`.
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

/// The three keys, or an error naming the missing one, the file, and the step that writes it.
pub fn credentials(body: &str, path: &str) -> Result<(String, String, String), String> {
    let get = |key: &str| {
        body.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                format!(
                    "{key} is missing from {path}. Enable the People API, then run: bun \
                     capabilities/comms/auth/get-refresh-token.ts --env entities.env \
                     --scope contacts.readonly"
                )
            })
    };
    Ok((
        get("GOOGLE_CLIENT_ID")?,
        get("GOOGLE_CLIENT_SECRET")?,
        get("GOOGLE_REFRESH_TOKEN")?,
    ))
}

fn access_token(
    client: &reqwest::blocking::Client,
    id: &str,
    secret: &str,
    refresh: &str,
) -> Result<String, String> {
    let response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", id),
            ("client_secret", secret),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|e| format!("token refresh: {e}"))?;
    let status = response.status();
    let body: Value = response.json().unwrap_or(Value::Null);
    if !status.is_success() {
        // The error code only: Google puts token material in some of these bodies.
        let code = body["error"].as_str().unwrap_or("unknown");
        return Err(format!(
            "token refresh answered {status} ({code}); re-mint the refresh token"
        ));
    }
    body["access_token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "token refresh returned no access_token".into())
}

/// Every connection, all pages.
/// A client and a fresh access token from the overlay's credentials.
fn authorised() -> Result<(reqwest::blocking::Client, String), String> {
    let path = env_path();
    let body = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let (id, secret, refresh) = credentials(&body, &path.display().to_string())?;
    let client = sjel_http::client(
        sjel_http::Purpose::new("entities-google"),
        Duration::from_secs(30),
    )
    .map_err(|e| e.to_string())?;
    let token = access_token(&client, &id, &secret, &refresh)?;
    Ok((client, token))
}

/// One contact as Google holds it now, by its resource name (`people/c…`).
pub fn fetch_one(resource_name: &str) -> Result<Value, String> {
    if !resource_name.starts_with("people/") || resource_name.contains("..") {
        return Err(format!(
            "{resource_name:?} is not a People API resource name"
        ));
    }
    let (client, token) = authorised()?;
    let response = client
        .get(format!("https://people.googleapis.com/v1/{resource_name}"))
        .bearer_auth(&token)
        .query(&[("personFields", PERSON_FIELDS)])
        .send()
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
        let response = client
            .get("https://people.googleapis.com/v1/people/me/connections")
            .bearer_auth(&token)
            .query(&query)
            .send()
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

    #[test]
    fn a_missing_key_names_itself_and_the_fix() {
        let error = credentials(
            "GOOGLE_CLIENT_ID=x\nGOOGLE_CLIENT_SECRET=y\n",
            "entities.env",
        )
        .unwrap_err();
        assert!(error.contains("GOOGLE_REFRESH_TOKEN") && error.contains("contacts.readonly"));
        assert!(credentials(
            "GOOGLE_CLIENT_ID=x\nGOOGLE_CLIENT_SECRET=y\nGOOGLE_REFRESH_TOKEN=z",
            "f"
        )
        .is_ok());
    }
}

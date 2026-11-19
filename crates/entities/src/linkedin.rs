//! The LinkedIn adapter, inbound, from `Connections.csv` in a "Get a copy of your data" export.
//!
//! The export lists first-degree connections only: name, profile URL, the email a connection
//! chose to share, current company and position, and the day the connection was made. It holds
//! nothing about a connection's own connections. `messages.csv` adds the day of the latest
//! message with each linked person; only participant URLs and dates are read, never content.
//! Files are read locally; nothing is sent to LinkedIn or any other service.

use std::collections::BTreeMap;

use serde_json::json;

use crate::model::EntityEdge;
use crate::store::{EntitiesStore, Patch, StoreError};
use crate::sync::Incoming;

/// The source of every value this adapter writes, and its external-ID namespace.
pub const SOURCE: &str = "linkedin";

/// The person fields this adapter writes.
pub const MANAGED: &[&str] = &["emails", "company", "role", "linkedin", "connected_on"];

/// The header cell that starts the table. The rows above it are LinkedIn's notes.
const FIRST_HEADER: &str = "First Name";

/// What a `Connections.csv` held.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub records: Vec<Incoming>,
    /// Rows without a name or a profile URL: nothing to find or link them by.
    pub skipped: usize,
}

/// Parses the text of a `Connections.csv`.
///
/// # Errors
/// The text has no header row with the columns the adapter reads.
pub fn parse(text: &str) -> Result<Parsed, String> {
    let rows = split_rows(text);
    let start = rows
        .iter()
        .position(|row| row.first().is_some_and(|c| c.trim() == FIRST_HEADER))
        .ok_or("no 'First Name' header row: is this a LinkedIn Connections.csv?")?;
    let header: Vec<&str> = rows[start].iter().map(|c| c.trim()).collect();
    let column = |name: &str| header.iter().position(|h| *h == name);
    let (Some(first), Some(last), Some(url)) =
        (column("First Name"), column("Last Name"), column("URL"))
    else {
        return Err("the header lacks First Name, Last Name or URL".into());
    };
    let email = column("Email Address");
    let company = column("Company");
    let position = column("Position");
    let connected = column("Connected On");

    let mut parsed = Parsed::default();
    for row in &rows[start + 1..] {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let cell = |i: Option<usize>| {
            i.and_then(|i| row.get(i))
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
        };
        let name = [cell(Some(first)), cell(Some(last))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let Some(profile) = cell(Some(url)).map(profile_key) else {
            parsed.skipped += 1;
            continue;
        };
        if name.is_empty() {
            parsed.skipped += 1;
            continue;
        }
        let mut values = BTreeMap::new();
        values.insert("linkedin".into(), json!(profile));
        if let Some(e) = cell(email) {
            values.insert("emails".into(), json!([e]));
        }
        if let Some(c) = cell(company) {
            values.insert("company".into(), json!(c));
        }
        if let Some(p) = cell(position) {
            values.insert("role".into(), json!(p));
        }
        if let Some(day) = cell(connected).and_then(day) {
            values.insert("connected_on".into(), json!(day));
        }
        parsed.records.push(Incoming {
            external_id: profile,
            etag: None,
            name,
            note_ref: None,
            values,
            home: None,
        });
    }
    Ok(parsed)
}

/// The latest message day per participant profile, from the text of `messages.csv`.
///
/// # Errors
/// The text lacks the participant or date columns.
pub fn last_messages(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut latest: BTreeMap<String, String> = BTreeMap::new();
    for conversation in conversations(text)? {
        for profile in conversation.profiles {
            let held = latest.entry(profile).or_default();
            if *held < conversation.day {
                held.clone_from(&conversation.day);
            }
        }
    }
    Ok(latest)
}

/// One conversation: everyone who wrote or received a message in it, and its latest day.
#[derive(Debug, Default, PartialEq)]
pub struct Conversation {
    pub profiles: std::collections::BTreeSet<String>,
    pub day: String,
}

/// The conversations in the text of `messages.csv`, by conversation ID. Messages without a
/// date are left out.
///
/// # Errors
/// The text lacks the conversation, participant or date columns.
pub fn conversations(text: &str) -> Result<Vec<Conversation>, String> {
    let rows = split_rows(text);
    let Some((header, body)) = rows.split_first() else {
        return Ok(Vec::new());
    };
    let column = |name: &str| header.iter().position(|h| h.trim() == name);
    let (Some(id), Some(sender), Some(recipients), Some(date)) = (
        column("CONVERSATION ID"),
        column("SENDER PROFILE URL"),
        column("RECIPIENT PROFILE URLS"),
        column("DATE"),
    ) else {
        return Err(
            "no CONVERSATION ID, SENDER PROFILE URL, RECIPIENT PROFILE URLS or DATE \
                    column: is this a LinkedIn messages.csv?"
                .into(),
        );
    };
    let mut by_id: BTreeMap<String, Conversation> = BTreeMap::new();
    for row in body {
        let Some(day) = row
            .get(date)
            .and_then(|d| d.trim().get(..10))
            .filter(|d| entities_api::is_day(d))
        else {
            continue;
        };
        let key = row.get(id).map_or("", |c| c.trim()).to_string();
        let conversation = by_id.entry(key).or_default();
        if conversation.day.as_str() < day {
            conversation.day = day.to_string();
        }
        let urls = row.get(sender).into_iter().chain(row.get(recipients));
        for url in urls.flat_map(|cell| cell.split([',', ' '])) {
            let url = url.trim();
            if !url.is_empty() {
                conversation.profiles.insert(profile_key(url));
            }
        }
    }
    Ok(by_id.into_values().collect())
}

/// What a conversation-edge run did.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct EdgeReport {
    /// Conversations with at least two linked people besides the operator.
    pub groups: usize,
    pub edges: usize,
}

/// Links every two people who shared a conversation, with its latest day. The operator's own
/// profile links to no person, so a one-to-one conversation adds no edge.
pub fn apply_conversations(
    store: &EntitiesStore,
    conversations: &[Conversation],
    dry_run: bool,
) -> Result<EdgeReport, StoreError> {
    let mut report = EdgeReport::default();
    for conversation in conversations {
        let mut people = Vec::new();
        for profile in &conversation.profiles {
            if let Some(id) = store.external(SOURCE, profile)? {
                if store.get(&id)?.is_some_and(|e| e.kind == "person") {
                    people.push(id);
                }
            }
        }
        if people.len() < 2 {
            continue;
        }
        report.groups += 1;
        for (i, a) in people.iter().enumerate() {
            for b in &people[i + 1..] {
                report.edges += 1;
                if !dry_run {
                    store.add_edge(&EntityEdge {
                        a: a.clone(),
                        b: b.clone(),
                        kind: "conversation".into(),
                        source: SOURCE.into(),
                        at: Some(conversation.day.clone()),
                        note: None,
                    })?;
                }
            }
        }
    }
    Ok(report)
}

/// What a `last_contact` run did.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct ContactReport {
    pub profiles: usize,
    pub linked: usize,
    pub updated: usize,
}

/// Writes `last_contact` for each person linked to a profile, where the message day is later
/// than the stored one.
pub fn apply_last_contact(
    store: &EntitiesStore,
    latest: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<ContactReport, StoreError> {
    let mut report = ContactReport {
        profiles: latest.len(),
        ..ContactReport::default()
    };
    for (profile, day) in latest {
        let Some(id) = store.external(SOURCE, profile)? else {
            continue;
        };
        let Some(person) = store.get(&id)?.filter(|e| e.kind == "person") else {
            continue;
        };
        report.linked += 1;
        let stored = person
            .values
            .get("last_contact")
            .and_then(|v| v.value.as_str());
        if stored.is_some_and(|s| s >= day.as_str()) {
            continue;
        }
        report.updated += 1;
        if !dry_run {
            store.patch(
                &person.id,
                &Patch {
                    values: BTreeMap::from([("last_contact".to_string(), json!(day))]),
                    source: SOURCE.into(),
                    ..Patch::default()
                },
            )?;
        }
    }
    Ok(report)
}

/// A profile URL as one key: no query, no trailing slash, lower case, `https`.
fn profile_key(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let url = url.trim_end_matches('/').to_lowercase();
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    }
}

/// `15 Mar 2024` as `2024-03-15`, or `None` for any other shape.
fn day(text: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let mut parts = text.split_whitespace();
    let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let d: u32 = d.parse().ok().filter(|d| (1..=31).contains(d))?;
    let m = MONTHS
        .iter()
        .position(|name| m.to_lowercase().starts_with(name))?
        + 1;
    let y: u32 = y.parse().ok().filter(|y| (1900..=9999).contains(y))?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Splits CSV text into rows of cells, per RFC 4180: quoted fields, `""` for one quote, and
/// newlines inside quotes.
fn split_rows(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' => {}
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = "\u{feff}Notes:\r\n\
\"When exporting your connection data, you may notice that some of the email addresses are missing.\"\r\n\
\r\n\
First Name,Last Name,URL,Email Address,Company,Position,Connected On\r\n\
Ada,Example,https://www.linkedin.com/in/ada-example/,ada@example.org,\"Acme, Inc.\",Engineer,15 Mar 2024\r\n\
Bo,,http://www.linkedin.com/in/Bo-Test?trk=x,,,,1 Sep 2019\r\n\
,,https://www.linkedin.com/in/nameless,,,,\r\n\
Cy,Nourl,,,,,\r\n";

    #[test]
    fn a_connection_becomes_a_record_keyed_by_its_profile() {
        let parsed = parse(EXPORT).unwrap();
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.skipped, 2);
        let ada = &parsed.records[0];
        assert_eq!(ada.name, "Ada Example");
        assert_eq!(ada.external_id, "https://www.linkedin.com/in/ada-example");
        assert_eq!(ada.values["emails"], json!(["ada@example.org"]));
        assert_eq!(ada.values["company"], json!("Acme, Inc."));
        assert_eq!(ada.values["role"], json!("Engineer"));
        assert_eq!(ada.values["connected_on"], json!("2024-03-15"));
    }

    #[test]
    fn a_missing_value_is_absent_not_empty() {
        let bo = &parse(EXPORT).unwrap().records[1];
        assert_eq!(bo.name, "Bo");
        assert_eq!(bo.external_id, "https://www.linkedin.com/in/bo-test");
        assert!(!bo.values.contains_key("emails"));
        assert!(!bo.values.contains_key("company"));
        assert_eq!(bo.values["connected_on"], json!("2019-09-01"));
    }

    #[test]
    fn the_latest_message_day_counts_for_every_participant() {
        let text = "CONVERSATION ID,FROM,SENDER PROFILE URL,TO,RECIPIENT PROFILE URLS,DATE,CONTENT\n\
c1,Me,https://www.linkedin.com/in/me,Ada,https://www.linkedin.com/in/Ada-Example/,2026-03-01 10:00:00 UTC,hi\n\
c1,Ada,https://www.linkedin.com/in/ada-example,Me,https://www.linkedin.com/in/me,2026-02-01 09:00:00 UTC,\"a, b\"\n\
c2,Bo,https://www.linkedin.com/in/bo,\"Me, Cy\",\"https://www.linkedin.com/in/me,https://www.linkedin.com/in/cy\",2025-12-24 08:00:00 UTC,x\n\
c3,Bo,https://www.linkedin.com/in/bo,Me,https://www.linkedin.com/in/me,undated,x\n";
        let latest = last_messages(text).unwrap();
        assert_eq!(
            latest["https://www.linkedin.com/in/ada-example"],
            "2026-03-01"
        );
        assert_eq!(latest["https://www.linkedin.com/in/bo"], "2025-12-24");
        assert_eq!(latest["https://www.linkedin.com/in/cy"], "2025-12-24");
        assert_eq!(latest["https://www.linkedin.com/in/me"], "2026-03-01");
        assert!(last_messages("a,b\n1,2\n").is_err());
    }

    #[test]
    fn a_conversation_collects_every_participant_and_its_latest_day() {
        let text = "CONVERSATION ID,SENDER PROFILE URL,RECIPIENT PROFILE URLS,DATE\n\
g,https://www.linkedin.com/in/me,\"https://www.linkedin.com/in/ada,https://www.linkedin.com/in/bo\",2026-01-01 10:00:00 UTC\n\
g,https://www.linkedin.com/in/bo,\"https://www.linkedin.com/in/me,https://www.linkedin.com/in/ada\",2026-02-01 10:00:00 UTC\n\
o,https://www.linkedin.com/in/cy,https://www.linkedin.com/in/me,2025-05-05 10:00:00 UTC\n";
        let found = conversations(text).unwrap();
        assert_eq!(found.len(), 2);
        let group = found.iter().find(|c| c.profiles.len() == 3).unwrap();
        assert_eq!(group.day, "2026-02-01");
        assert!(group.profiles.contains("https://www.linkedin.com/in/bo"));
    }

    #[test]
    fn a_file_without_the_header_is_refused() {
        assert!(parse("Name,Email\nAda,ada@example.org\n").is_err());
    }

    #[test]
    fn only_the_export_date_shape_is_read() {
        assert_eq!(day("1 Sep 2019").as_deref(), Some("2019-09-01"));
        assert_eq!(day("2019-09-01"), None);
        assert_eq!(day("32 Sep 2019"), None);
    }

    #[test]
    fn every_managed_key_is_a_built_in_person_field() {
        let fields = entities_api::builtin_fields();
        for key in MANAGED {
            assert!(
                fields.iter().any(|f| f.kind == "person" && f.key == *key),
                "{key} is not a built-in person field"
            );
        }
    }
}

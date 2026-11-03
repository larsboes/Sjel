//! `last_contact`, computed from the Journal in `projects`.
//!
//! A dated Journal entry that links a person's name counts as contact on that day. Until
//! 2026-10-20 `capabilities/vault` computed this from the vault's `Journal/` and the
//! `Atlas/People` notes (`vault/src/people.rs`), and the Obsidian adapter carried it here. Both
//! folders left the vault that day: the Journal for the `projects` store, the people for this
//! capability. The rule is the same: the entry's date is the first ten characters of its file
//! name, a link counts once per entry, and links are parsed by `markdown_root::wikilink_targets`,
//! the parser the vault's link checker uses.
//!
//! The value only moves forward. A later day typed by the operator is never replaced by an
//! earlier one from the Journal.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::store::{EntitiesStore, Patch, StoreError};

/// The top folder of Journal records in `projects`.
pub const JOURNAL: &str = "Journal";
/// The source every value written here carries.
pub const SOURCE: &str = "journal";

/// `YYYY-MM-DD` from the start of a file name, or `None` for an undated entry such as a review.
fn entry_date(path: &str) -> Option<String> {
    let base = path.rsplit('/').next()?;
    let day: String = base.chars().take(10).collect();
    let b = day.as_bytes();
    let shaped = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && day
            .chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
    shaped.then_some(day)
}

/// The latest dated entry that links each name, keyed by the lowercased link basename.
pub fn last_contacts(records: &[(String, String)]) -> BTreeMap<String, String> {
    let prefix = format!("{JOURNAL}/");
    let mut latest: BTreeMap<String, String> = BTreeMap::new();
    for (path, content) in records {
        if !path.starts_with(&prefix) {
            continue;
        }
        let Some(day) = entry_date(path) else {
            continue;
        };
        for (target, _) in markdown_root::wikilink_targets(content, 0) {
            let name = target
                .rsplit('/')
                .next()
                .unwrap_or(&target)
                .trim_end_matches(".md")
                .to_lowercase();
            let slot = latest.entry(name).or_default();
            if day > *slot {
                *slot = day.clone();
            }
        }
    }
    latest
}

/// Every Journal record, as `(path, content)`, through the `projects` HTTP surface.
pub fn fetch(projects_url: &str) -> Result<Vec<(String, String)>, String> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("entities-journal"),
        Duration::from_secs(60),
    )
    .map_err(|e| e.to_string())?;
    let url = format!(
        "{}/api/project?name={JOURNAL}",
        projects_url.trim_end_matches('/')
    );
    let body: Value = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("projects /api/project: {e}"))?;
    Ok(body["records"]
        .as_array()
        .ok_or("projects /api/project: no records array")?
        .iter()
        .filter_map(|r| Some((r["path"].as_str()?.into(), r["content"].as_str()?.into())))
        .collect())
}

/// What one run did.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct Report {
    pub people: usize,
    pub with_contact: usize,
    pub updated: usize,
}

/// Write each person's `last_contact` where the Journal knows a later day than the stored one.
pub fn apply(
    store: &EntitiesStore,
    latest: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<Report, StoreError> {
    let people = store.list(Some("person"), None)?;
    let mut report = Report {
        people: people.len(),
        ..Report::default()
    };
    for person in people {
        let Some(day) = latest.get(&person.name.to_lowercase()) else {
            continue;
        };
        report.with_contact += 1;
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
                    name: None,
                    note_ref: None,
                    values: BTreeMap::from([("last_contact".to_string(), json!(day))]),
                    source: SOURCE.into(),
                    expected_revision: None,
                },
            )?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, text: &str) -> (String, String) {
        (path.to_string(), text.to_string())
    }

    #[test]
    fn the_latest_dated_entry_wins_and_undated_ones_do_not_count() {
        let latest = last_contacts(&[
            entry(
                "Journal/01. Daily Notes/2026-01-05.md",
                "coffee with [[Erika]]",
            ),
            entry(
                "Journal/01. Daily Notes/2026-03-01 Sunday.md",
                "hike, [[Atlas/People/Erika.md|E]]",
            ),
            entry("Journal/02. Weekly Reviews/Week 9.md", "[[Erika]] again"),
            entry("Atlas/Reflections/Talk.md", "[[Ron]]"),
        ]);
        assert_eq!(latest.get("erika").map(String::as_str), Some("2026-03-01"));
        assert!(!latest.contains_key("ron"), "only Journal entries count");
    }
}

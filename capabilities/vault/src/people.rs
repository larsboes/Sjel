//! What the Journal already knows about each person, computed rather than typed.
//!
//! PRD D2: `last_contact`, `met_at` and `mention_count` sit on `Atlas/People` notes. All three
//! are computable from `Journal/` backlinks, and this file computes them and reports the drift
//! against what the notes carry.
//!
//! **It reads and never writes.** A note-borne hash is impossible on a frontmatter key, so a
//! writer here could not tell its own value from one a human corrected. `fields.rs` is the
//! writer, for `last_contact` and `met_at` only, and it gets around that by moving each key in
//! one declared direction and recording a conflict when the note is ahead of the Journal.
//!
//! **`contact_frequency` is not here** (D1), and that is a different kind of absence: how often
//! you want to see someone is a judgement nobody can derive from a backlink count. §8.1's band
//! 540 either gets it by hand or the band goes.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::note::Note;

/// Where the People notes live. Named once, because the CLI reads the whole vault and
/// `vault-server` reads exactly this folder and `JOURNAL` — two spellings of the same folder is
/// how the two front ends would start answering differently.
pub const FOLDER: &str = "Atlas/People";

/// Where the evidence lives. All three computed fields are backlinks from here.
pub const JOURNAL: &str = "Journal";

/// One person, as the Journal describes them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PersonFacts {
    pub id: String,
    pub name: String,
    /// How many `Journal/` notes link this person. Notes, not occurrences: two mentions in one
    /// entry are one day on which they came up.
    pub mention_count: usize,
    /// The newest journal date that links them, `YYYY-MM-DD`.
    pub last_contact: Option<String>,
    /// The oldest journal date that links them. Read as "first appears in the record" and not
    /// as "the day we met". `fields.rs` writes it only earlier, never later.
    pub met_at: Option<String>,
    /// What the note itself carries today, for each of the three keys, where it carries it.
    pub stored: BTreeMap<String, String>,
    /// The keys whose stored value disagrees with the computed one.
    pub disagrees: Vec<String>,
    /// The note's `home` key: the city the person lives in, as typed. PRD Q116 (2026-09-25):
    /// who someone is stays in the note, and Axon reads it.
    pub home: Option<String>,
    /// `host: yes` in the note: Lars could ask to stay with this person at their place.
    pub host: bool,
    /// The note's `host_note`, as typed: "sofa, ask a week ahead".
    pub host_note: Option<String>,
    /// The structured keys a person note carries, as typed, for `capabilities/entities`'
    /// Obsidian import (PRD Q117). Only [`PROFILE_SCALARS`] and [`PROFILE_LISTS`], and only
    /// when filled. Prose keys (character, memories, highlights) stay in the note by rule.
    pub profile: BTreeMap<String, serde_json::Value>,
}

/// Single-value keys the import reads. Measured 2026-09-25 across 89 notes: relation 26,
/// company 12, birthday 3, coordinates 2, role 2, email 1 filled.
pub const PROFILE_SCALARS: &[&str] = &[
    "relation",
    "company",
    "role",
    "birthday",
    "email",
    "coordinates",
];

/// List keys the import reads: interests 8, skills 3, socials 1 filled on the same day.
pub const PROFILE_LISTS: &[&str] = &["interests", "skills", "socials"];

/// The items of a YAML list key in raw frontmatter: `key: [a, b]` or `key:` followed by
/// `- a` lines. `note.fields` keeps scalars only, so lists are read here.
fn raw_list(raw: &str, key: &str) -> Vec<String> {
    let clean = |item: &str| {
        item.trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .trim()
            .to_string()
    };
    let mut lines = raw.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) else {
            continue;
        };
        let rest = rest.trim();
        if let Some(inline) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            return inline
                .split(',')
                .map(clean)
                .filter(|i| !i.is_empty())
                .collect();
        }
        let mut items = Vec::new();
        for next in lines.by_ref() {
            let Some(item) = next.trim_start().strip_prefix("- ") else {
                break;
            };
            let item = clean(item);
            if !item.is_empty() {
                items.push(item);
            }
        }
        return items;
    }
    Vec::new()
}

fn profile_of(note: &Note) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for key in PROFILE_SCALARS {
        if let Some(value) = scalar(note, key) {
            out.insert((*key).to_string(), serde_json::Value::String(value));
        }
    }
    if let Some(raw) = note.raw_frontmatter.as_deref() {
        for key in PROFILE_LISTS {
            let items = raw_list(raw, key);
            if !items.is_empty() {
                out.insert((*key).to_string(), serde_json::json!(items));
            }
        }
    }
    out
}

/// A frontmatter scalar without quotes or surrounding space, or `None` when empty.
fn scalar(note: &Note, key: &str) -> Option<String> {
    let value = note
        .field(key)?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim();
    (!value.is_empty() && value != "[]").then(|| value.to_string())
}

/// `yes`, `true` or `y`, in any case. Anything else, including absence, is no.
fn yes(value: Option<String>) -> bool {
    value.is_some_and(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "y"))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PeopleReport {
    pub people: usize,
    /// People with at least one journal backlink.
    pub with_mentions: usize,
    /// Notes carrying at least one of the three keys.
    pub carrying_any: usize,
    /// Notes where a carried key disagrees with what the Journal says.
    pub disagreeing: usize,
    pub facts: Vec<PersonFacts>,
}

/// The date a journal note is about, from its id: `Journal/2026-09-07.md` and
/// `Journal/2026/2026-09-07 Monday.md` both answer `2026-09-07`.
///
/// From the id and never from a frontmatter field: the filename is what the operator controls
/// and what survives a plugin, and a journal note whose name is not a date is not a day.
pub(crate) fn journal_date(id: &str) -> Option<String> {
    let base = id.rsplit('/').next()?;
    let candidate: String = base.chars().take(10).collect();
    let bytes = candidate.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits_ok = candidate
        .chars()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
    digits_ok.then_some(candidate)
}

/// Every `[[target]]` in a note, basename-folded, so a path-form link and a bare one count once.
pub(crate) fn linked_basenames(note: &Note) -> Vec<String> {
    crate::graph::targets_for_test(&note.text, note.body_start)
        .into_iter()
        .map(|target| {
            target
                .rsplit('/')
                .next()
                .unwrap_or(&target)
                .trim_end_matches(".md")
                .to_lowercase()
        })
        .collect()
}

pub fn report(notes: &[Note]) -> PeopleReport {
    let prefix = format!("{FOLDER}/");
    let people: Vec<&Note> = notes
        .iter()
        .filter(|note| note.id.starts_with(&prefix))
        .collect();

    // basename -> (mentions, earliest, latest)
    let mut seen: BTreeMap<String, (usize, Option<String>, Option<String>)> = BTreeMap::new();
    for note in notes.iter().filter(|note| note.folder == JOURNAL) {
        let Some(date) = journal_date(&note.id) else {
            continue;
        };
        let mut once: Vec<String> = linked_basenames(note);
        once.sort();
        once.dedup();
        for name in once {
            let entry = seen.entry(name).or_insert((0, None, None));
            entry.0 += 1;
            if entry.1.as_ref().is_none_or(|first| date < *first) {
                entry.1 = Some(date.clone());
            }
            if entry.2.as_ref().is_none_or(|last| date > *last) {
                entry.2 = Some(date.clone());
            }
        }
    }

    let mut out = PeopleReport {
        people: people.len(),
        ..PeopleReport::default()
    };

    for note in people {
        let key = note.basename.to_lowercase();
        let (mention_count, met_at, last_contact) =
            seen.get(&key).cloned().unwrap_or((0, None, None));

        let mut stored = BTreeMap::new();
        for field in ["last_contact", "met_at", "mention_count"] {
            if let Some(value) = note.field(field).map(str::trim).filter(|v| !v.is_empty()) {
                stored.insert(field.to_string(), value.to_string());
            }
        }

        let mut disagrees = Vec::new();
        if let Some(value) = stored.get("mention_count") {
            if value.trim() != mention_count.to_string() {
                disagrees.push("mention_count".to_string());
            }
        }
        for (field, computed) in [("last_contact", &last_contact), ("met_at", &met_at)] {
            if let (Some(value), Some(computed)) = (stored.get(field), computed.as_ref()) {
                // Compared on the first ten characters: a stored value may carry a time or a
                // wikilink around the date, and the disagreement worth reporting is the day.
                let stored_day: String = value
                    .chars()
                    .filter(|c| !"[]".contains(*c))
                    .take(10)
                    .collect();
                if &stored_day != computed {
                    disagrees.push(field.to_string());
                }
            }
        }

        if mention_count > 0 {
            out.with_mentions += 1;
        }
        if !stored.is_empty() {
            out.carrying_any += 1;
        }
        if !disagrees.is_empty() {
            out.disagreeing += 1;
        }

        out.facts.push(PersonFacts {
            id: note.id.clone(),
            name: note.basename.clone(),
            mention_count,
            last_contact,
            met_at,
            stored,
            disagrees,
            home: scalar(note, "home"),
            host: yes(scalar(note, "host")),
            host_note: scalar(note, "host_note"),
            profile: profile_of(note),
        });
    }

    out.facts.sort_by(|a, b| {
        b.mention_count
            .cmp(&a.mention_count)
            .then(a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, text: &str) -> Note {
        Note {
            id: id.to_string(),
            path: std::path::PathBuf::from(id),
            basename: id
                .rsplit('/')
                .next()
                .unwrap_or(id)
                .trim_end_matches(".md")
                .to_string(),
            folder: id.split('/').next().unwrap_or("").to_string(),
            fields: Default::default(),
            body_start: 0,
            raw_frontmatter: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn the_three_fields_come_out_of_the_journal() {
        let notes = vec![
            note("Atlas/People/Erika.md", ""),
            note("Journal/2026-01-05.md", "coffee with [[Erika]]"),
            note(
                "Journal/2026-03-09 Monday.md",
                "[[Erika]] again, and [[Erika]] twice",
            ),
        ];
        let report = report(&notes);
        let erika = &report.facts[0];
        // Two notes, not three mentions: a day on which someone came up is one day.
        assert_eq!(erika.mention_count, 2);
        assert_eq!(erika.met_at.as_deref(), Some("2026-01-05"));
        assert_eq!(erika.last_contact.as_deref(), Some("2026-03-09"));
    }

    #[test]
    fn home_and_hosting_are_read_as_typed() {
        let mut host = note("Atlas/People/Erika.md", "");
        host.fields.insert("home".into(), "\"Bonn\"".into());
        host.fields.insert("host".into(), "Yes".into());
        host.fields
            .insert("host_note".into(), "sofa, ask a week ahead".into());
        let mut other = note("Atlas/People/Max.md", "");
        other.fields.insert("host".into(), "maybe".into());
        other.fields.insert("home".into(), " ".into());
        let report = report(&[host, other]);
        let erika = report.facts.iter().find(|f| f.name == "Erika").unwrap();
        assert_eq!(erika.home.as_deref(), Some("Bonn"));
        assert!(erika.host);
        assert_eq!(erika.host_note.as_deref(), Some("sofa, ask a week ahead"));
        let max = report.facts.iter().find(|f| f.name == "Max").unwrap();
        assert_eq!(max.home, None);
        assert!(!max.host, "only yes, true or y is a yes");
    }

    #[test]
    fn list_keys_are_read_from_raw_frontmatter_in_both_forms() {
        let raw = "relation: colleague\ninterests:\n  - bouldering\n  - \"jazz\"\nskills: [rust, go]\nsocials:\nnext: x";
        assert_eq!(raw_list(raw, "interests"), vec!["bouldering", "jazz"]);
        assert_eq!(raw_list(raw, "skills"), vec!["rust", "go"]);
        assert!(raw_list(raw, "socials").is_empty());
        assert!(raw_list(raw, "missing").is_empty());

        let mut ron = note("Atlas/People/Ron.md", "");
        ron.fields.insert("relation".into(), "colleague".into());
        ron.fields.insert("company".into(), " ".into());
        ron.raw_frontmatter = Some(raw.into());
        let profile = profile_of(&ron);
        assert_eq!(profile["relation"], serde_json::json!("colleague"));
        assert!(
            !profile.contains_key("company"),
            "an empty key is not a value"
        );
        assert_eq!(
            profile["interests"],
            serde_json::json!(["bouldering", "jazz"])
        );
    }

    #[test]
    fn a_journal_note_that_is_not_a_date_is_not_a_day() {
        let notes = vec![
            note("Atlas/People/Erika.md", ""),
            note("Journal/Notes on journalling.md", "[[Erika]]"),
        ];
        assert_eq!(report(&notes).facts[0].mention_count, 0);
    }

    #[test]
    fn a_stored_value_that_disagrees_is_named() {
        let mut person = note("Atlas/People/Erika.md", "");
        person.fields.insert("mention_count".into(), "40".into());
        person
            .fields
            .insert("last_contact".into(), "2020-01-01".into());
        let notes = vec![person, note("Journal/2026-01-05.md", "[[Erika]]")];
        let report = report(&notes);
        assert_eq!(report.disagreeing, 1);
        assert_eq!(
            report.facts[0].disagrees,
            vec!["mention_count", "last_contact"]
        );
    }

    #[test]
    fn a_person_the_journal_never_names_reads_as_zero_and_not_as_missing() {
        // Zero is the measurement here, and it is the one a `contact_frequency` band would act
        // on. An absent value would be indistinguishable from a producer that did not run.
        let notes = vec![note("Atlas/People/Erika.md", "")];
        let facts = &report(&notes).facts[0];
        assert_eq!((facts.mention_count, facts.last_contact.clone()), (0, None));
    }
}

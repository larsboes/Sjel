//! `trips gear import` — the overlay's item notes, read as PROPOSALS.
//!
//! ## It proposes by default and writes when asked
//!
//! PRD Q58 rules that gear and furniture share one table, `interior_item`. Until
//! 2026-09-07 that table's `kind` was `CHECK (kind IN ('piece','slot'))` with no
//! column for any of the seven fields below, so an imported row was
//! indistinguishable from a sofa and `--apply` was refused by naming what it was
//! waiting for. B51 shipped the columns and widened the CHECK
//! (`capabilities/interior/src/store.rs`), so the refusal is lifted: without a
//! flag this still only reads and proposes, and `--apply` writes.
//!
//! ## Idempotent by construction
//!
//! A proposal's id is derived from the note's file stem, so the same notes
//! produce the same proposal set on every run, on every machine. When the
//! columns exist, `--apply` will POST each proposal to interior's own
//! `POST /api/items`, which answers 409 for an id that is already there and is
//! therefore idempotent for free — never by SQL, which is Q73's write-path rule
//! and the direction `capabilities/calendar` already takes into trips.
//!
//! ## Nothing from the overlay lives in this file
//!
//! Only key names and shapes. The values — brands, models, prices, what is in
//! the operator's wardrobe — are read at run time and printed to the operator's
//! own terminal. The tests below use synthetic notes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// The seven fields that distinguish a gear row from furniture, and the columns
/// `interior_item` has carried since B51. Ordered as the notes carry them.
pub const REQUIRED_INTERIOR_COLUMNS: &[&str] = &[
    "category",
    "weight_g",
    "packable",
    "waterproof",
    "quick_dry",
    "pack_location",
    "trip_types",
];

/// What a run without `--apply` says, so "nothing happened" is never mistaken for
/// "nothing could happen". The refusal this replaces stood from 2026-09-05 to
/// 2026-09-07 and named the seven columns it was waiting for; they exist now.
pub const NOTHING_WRITTEN: &str = "\
Nothing was written. Re-run with --apply to POST each proposal to interior's own \
POST /api/items, which answers 409 for an id that is already there — so a second run \
changes nothing.";

/// One note, reduced to what a pack list would need from it.
///
/// Every attribute is optional: a note that does not carry a field is reported
/// without it rather than with a zero, because "not weighed" and "weighs
/// nothing" are different facts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GearProposal {
    /// Stable, derived from the note's file stem — the same identity rule
    /// `places::stable_id` uses, so a re-run lands on the row it made before.
    pub id: String,
    pub label: String,
    pub category: Option<String>,
    pub weight_g: Option<i64>,
    pub packable: Option<bool>,
    pub waterproof: Option<bool>,
    pub quick_dry: Option<bool>,
    pub pack_location: Option<String>,
    pub trip_types: Vec<String>,
    /// `owned` or `wanted`, from the note's `state:` key.
    ///
    /// Defaults to `owned`, and the default is a claim worth stating: a note in the
    /// operator's items directory was written about a thing that exists. `wanted` is the
    /// exception and has to be said. Interior requires the field either way — a row with no
    /// state joins to nothing and shows up in no list.
    pub state: String,
    /// The fields of [`REQUIRED_INTERIOR_COLUMNS`] this note does not carry.
    pub incomplete: Vec<String>,
}

impl GearProposal {
    /// The body of interior's `POST /api/items`, minus the `state` and `note` its handler
    /// adds around this.
    ///
    /// Written out field by field rather than serialising `Self`: `incomplete` and `state`
    /// are this file's own bookkeeping and have no column, and interior's `Item` accepts
    /// unknown keys silently. A payload built by coincidence of field names is one rename
    /// away from writing nothing and reporting 201.
    pub fn item_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "kind": "gear",
            "label": self.label,
            "category": self.category,
            "weight_g": self.weight_g,
            "packable": self.packable,
            "waterproof": self.waterproof,
            "quick_dry": self.quick_dry,
            "pack_location": self.pack_location,
            "trip_types": self.trip_types,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub scanned: usize,
    pub proposals: Vec<GearProposal>,
    /// Notes that carry none of the seven fields — a note about something that
    /// is not gear, not a defect.
    pub skipped: usize,
    /// The distinct `trip_types` values the notes actually use, which is where
    /// a pack list's `template_key` vocabulary comes from. There is no template
    /// table: the filter keys already exist as data.
    pub trip_type_vocabulary: Vec<String>,
}

/// Read every top-level `*.md` note under `directory`.
///
/// Top level only, and by design: the overlay's items directory carries an
/// `Assets` subdirectory of images and a nested notes folder, and recursing
/// would propose an attachment as a piece of gear.
pub fn scan(directory: &Path) -> Result<ImportReport, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    let mut report = ImportReport::default();
    let mut vocabulary: BTreeMap<String, ()> = BTreeMap::new();
    let mut notes: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    notes.sort();

    for path in notes {
        report.scanned += 1;
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let front = frontmatter(&body);
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let Some(proposal) = proposal_from(&stem, &front) else {
            report.skipped += 1;
            continue;
        };
        for trip_type in &proposal.trip_types {
            vocabulary.insert(trip_type.clone(), ());
        }
        report.proposals.push(proposal);
    }
    report.trip_type_vocabulary = vocabulary.into_keys().collect();
    Ok(report)
}

/// One proposal, or `None` when the note carries none of the seven fields.
pub fn proposal_from(stem: &str, front: &BTreeMap<String, String>) -> Option<GearProposal> {
    if REQUIRED_INTERIOR_COLUMNS
        .iter()
        .all(|column| !front.contains_key(*column))
    {
        return None;
    }
    let incomplete = REQUIRED_INTERIOR_COLUMNS
        .iter()
        .filter(|column| !front.contains_key(**column))
        .map(|column| (*column).to_string())
        .collect();
    Some(GearProposal {
        id: proposal_id(stem),
        label: stem.to_string(),
        category: front.get("category").cloned(),
        weight_g: front.get("weight_g").and_then(|value| value.parse().ok()),
        packable: front.get("packable").map(|value| is_true(value)),
        waterproof: front.get("waterproof").map(|value| is_true(value)),
        quick_dry: front.get("quick_dry").map(|value| is_true(value)),
        pack_location: front.get("pack_location").cloned(),
        trip_types: front
            .get("trip_types")
            .map(|value| list_values(value))
            .unwrap_or_default(),
        state: match front.get("state").map(|value| value.trim().to_lowercase()) {
            Some(value) if value == "wanted" => "wanted".to_string(),
            _ => "owned".to_string(),
        },
        incomplete,
    })
}

/// `gear:<slug>`. Deterministic, so two runs propose the same ids.
pub fn proposal_id(stem: &str) -> String {
    let mut slug = String::with_capacity(stem.len());
    let mut last_dash = true;
    for character in stem.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    format!("gear:{}", slug.trim_end_matches('-'))
}

/// The `key: value` pairs of a note's YAML front matter.
///
/// A hand-written reader rather than a YAML dependency: the shape here is one
/// flat block of scalars and inline or dashed lists, and a parser for that is
/// smaller than the argument for adding a crate to a workspace that has none.
/// Anything it cannot read is absent, which the `incomplete` list reports.
pub fn frontmatter(body: &str) -> BTreeMap<String, String> {
    let mut front = BTreeMap::new();
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return front;
    }
    let mut pending_list: Option<(String, Vec<String>)> = None;
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim() == "---" {
            break;
        }
        if let Some(item) = trimmed.trim_start().strip_prefix("- ") {
            if let Some((_, values)) = pending_list.as_mut() {
                values.push(unquote(item));
            }
            continue;
        }
        if let Some((key, values)) = pending_list.take() {
            front.insert(key, values.join(", "));
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if key.starts_with(char::is_whitespace) || key.trim().is_empty() {
            continue;
        }
        let key = key.trim().to_string();
        let value = value.trim();
        if value.is_empty() {
            pending_list = Some((key, Vec::new()));
        } else {
            front.insert(key, unquote(value));
        }
    }
    if let Some((key, values)) = pending_list {
        front.insert(key, values.join(", "));
    }
    front
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string()
}

fn is_true(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "yes")
}

fn list_values(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic notes. The overlay's own values never enter this repository.
    fn write_notes(directory: &Path) {
        std::fs::create_dir_all(directory.join("Assets")).unwrap();
        std::fs::write(directory.join("Assets").join("photo.md"), "not gear").unwrap();
        std::fs::write(
            directory.join("Test Jacket.md"),
            "---\n\
             category: outerwear\n\
             weight_g: 320\n\
             packable: true\n\
             waterproof: true\n\
             quick_dry: false\n\
             pack_location: main compartment\n\
             trip_types:\n\
             \x20 - hiking\n\
             \x20 - conference\n\
             summary: a jacket\n\
             ---\n\nBody text.\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("Test Socks.md"),
            "---\ncategory: base\nweight_g: 40\ntrip_types: [hiking, europe]\n---\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("A Meeting Note.md"),
            "---\ntitle: not an item\n---\n",
        )
        .unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("trips-gear-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Two runs over the same notes propose exactly the same ids, which is what
    /// makes the import safe to run again.
    #[test]
    fn the_same_notes_propose_the_same_ids_every_run() {
        let directory = temp_dir("idempotent");
        write_notes(&directory);
        let first = scan(&directory).expect("the directory is readable");
        let second = scan(&directory).expect("the directory is readable");
        assert_eq!(first.proposals, second.proposals);
        assert_eq!(
            first.proposals.iter().map(|p| &p.id).collect::<Vec<_>>(),
            vec!["gear:test-jacket", "gear:test-socks"]
        );
    }

    /// A note that is not about an item is skipped, and the subdirectory of
    /// attachments is never read.
    #[test]
    fn a_note_that_is_not_gear_is_skipped_and_assets_are_not_read() {
        let directory = temp_dir("skip");
        write_notes(&directory);
        let report = scan(&directory).expect("the directory is readable");
        assert_eq!(report.scanned, 3, "only top-level notes are scanned");
        assert_eq!(report.skipped, 1);
        assert_eq!(report.proposals.len(), 2);
    }

    /// A note missing four of the seven fields is proposed with the gaps named,
    /// not with zeros.
    #[test]
    fn a_partial_note_names_the_fields_it_does_not_carry() {
        let directory = temp_dir("partial");
        write_notes(&directory);
        let report = scan(&directory).expect("the directory is readable");
        let socks = report
            .proposals
            .iter()
            .find(|p| p.id == "gear:test-socks")
            .expect("the second note is proposed");
        assert_eq!(socks.weight_g, Some(40));
        assert_eq!(socks.packable, None, "an absent flag is not false");
        assert_eq!(
            socks.incomplete,
            vec!["packable", "waterproof", "quick_dry", "pack_location"]
        );
    }

    /// The vocabulary a `template_key` draws on comes from the notes, which is
    /// why there is no template table.
    #[test]
    fn the_trip_type_vocabulary_comes_from_the_notes() {
        let directory = temp_dir("vocab");
        write_notes(&directory);
        let report = scan(&directory).expect("the directory is readable");
        assert_eq!(
            report.trip_type_vocabulary,
            vec!["conference", "europe", "hiking"]
        );
    }

    #[test]
    fn both_list_forms_read_the_same() {
        let dashed = frontmatter("---\ntrip_types:\n  - a\n  - b\nnext: x\n---\n");
        assert_eq!(dashed.get("trip_types").map(String::as_str), Some("a, b"));
        assert_eq!(dashed.get("next").map(String::as_str), Some("x"));
        let inline = frontmatter("---\ntrip_types: [a, b]\n---\n");
        assert_eq!(
            list_values(inline.get("trip_types").unwrap()),
            vec!["a", "b"]
        );
    }

    /// The payload carries every one of the seven fields, under interior's own column names.
    ///
    /// This test replaces the one that asserted the refusal named all seven columns. It holds
    /// the same contract from the other end: the refusal existed because the columns did not,
    /// and now the risk is the opposite one — a field renamed here and silently dropped by
    /// interior, which accepts unknown keys without complaint.
    #[test]
    fn the_payload_carries_all_seven_fields_and_says_it_is_gear() {
        let proposal = proposal_from(
            "Zelt",
            &BTreeMap::from([
                ("category".to_string(), "schlafen".to_string()),
                ("weight_g".to_string(), "1850".to_string()),
                ("packable".to_string(), "true".to_string()),
                ("waterproof".to_string(), "true".to_string()),
                ("quick_dry".to_string(), "false".to_string()),
                ("pack_location".to_string(), "rucksack".to_string()),
                ("trip_types".to_string(), "[hiking]".to_string()),
            ]),
        )
        .expect("a note with all seven fields is a proposal");
        let payload = proposal.item_payload();
        assert_eq!(payload["kind"], "gear");
        assert_eq!(payload["id"], "gear:zelt");
        for column in REQUIRED_INTERIOR_COLUMNS {
            assert!(
                !payload[column].is_null(),
                "{column} is missing from the payload"
            );
        }
    }

    /// `state` defaults to owned, and `wanted` has to be said.
    #[test]
    fn a_note_is_owned_unless_it_says_otherwise() {
        let owned = proposal_from(
            "Zelt",
            &BTreeMap::from([("weight_g".to_string(), "1850".to_string())]),
        )
        .unwrap();
        assert_eq!(owned.state, "owned");
        let wanted = proposal_from(
            "Zelt",
            &BTreeMap::from([
                ("weight_g".to_string(), "1850".to_string()),
                ("state".to_string(), " Wanted ".to_string()),
            ]),
        )
        .unwrap();
        assert_eq!(wanted.state, "wanted");
    }
}

//! Vault classification reports use the shared classifier and expose refused declarations.

use std::collections::BTreeMap;

use content_item::DataClass;
use serde::Serialize;

use crate::note::Note;

/// The frontmatter key Q9a gives the operator.
pub const CLASS_KEY: &str = "class";

#[derive(Debug, Serialize)]
pub struct Classified {
    pub id: String,
    pub class: String,
    pub label: &'static str,
    pub method: String,
    pub rationale: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    /// Notes per class, `c0`..`c3`, in the vocabulary's own order.
    pub counts: BTreeMap<String, usize>,
    /// Notes whose frontmatter set the class, whatever it set it to.
    pub overridden: Vec<Classified>,
    /// Invalid declarations and attempts to lower the folder floor.
    pub refused: Vec<Classified>,
    pub notes: Vec<Classified>,
}

/// The deployment's configured folder floors (`content_item::VAULT_FLOORS_CONFIG`).
///
/// # Errors
/// The file exists and does not parse; a classifier must not run on floors it could not read.
pub fn deployment_floors() -> Result<content_item::VaultFloors, String> {
    match sjel_config::overlay_config(content_item::VAULT_FLOORS_CONFIG) {
        Some(path) => content_item::VaultFloors::from_file(&path),
        None => Ok(content_item::VaultFloors::default()),
    }
}

fn classify(floors: &content_item::VaultFloors, note: &Note) -> Classified {
    let class = DataClass::classify_vault_note_with(floors, &note.id, note.field(CLASS_KEY));
    Classified {
        id: note.id.clone(),
        class: class.value,
        label: class.label,
        method: class.method,
        rationale: class.rationale,
    }
}

pub fn report(floors: &content_item::VaultFloors, notes: &[Note]) -> Report {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut overridden = Vec::new();
    let mut refused = Vec::new();
    let mut all = Vec::with_capacity(notes.len());

    for note in notes {
        let row = classify(floors, note);
        *counts.entry(row.class.clone()).or_insert(0) += 1;

        let declared = note
            .field(CLASS_KEY)
            .map(str::trim)
            .is_some_and(|d| !d.is_empty());
        if declared {
            if row.method == content_item::METHOD_HUMAN {
                overridden.push(classify(floors, note));
            } else {
                refused.push(classify(floors, note));
            }
        }
        all.push(row);
    }

    for class in content_item::DATA_CLASSES {
        counts.entry(class.to_string()).or_insert(0);
    }

    Report {
        counts,
        overridden,
        refused,
        notes: all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn note(id: &str, declared: Option<&str>) -> Note {
        let mut fields = HashMap::new();
        if let Some(d) = declared {
            fields.insert(CLASS_KEY.to_string(), d.to_string());
        }
        Note {
            id: id.to_string(),
            path: PathBuf::from(id),
            basename: id.rsplit('/').next().unwrap_or(id).to_string(),
            folder: id.split('/').next().unwrap_or("").to_string(),
            fields,
            body_start: 0,
            raw_frontmatter: None,
            text: String::new(),
        }
    }

    #[test]
    fn every_class_gets_a_row_even_at_zero() {
        let rep = report(
            &content_item::VaultFloors::default(),
            &[note("Journal/a.md", None)],
        );
        for class in content_item::DATA_CLASSES {
            assert!(rep.counts.contains_key(class), "{class} has no row");
        }
        assert_eq!(rep.counts["c1"], 1);
        assert_eq!(rep.counts["c0"], 0);
    }

    #[test]
    fn an_override_and_a_refusal_are_counted_apart() {
        let rep = report(
            &content_item::VaultFloors::default(),
            &[
                note("Atlas/People/a.md", None),
                note("Atlas/People/b.md", Some("c3")),
                note("Atlas/People/c.md", Some("c22")),
                note("Journal/d.md", Some("   ")),
                note("Atlas/People/e.md", Some("c1")),
            ],
        );
        assert_eq!(rep.overridden.len(), 1);
        assert_eq!(rep.overridden[0].id, "Atlas/People/b.md");
        assert_eq!(rep.refused.len(), 2);
        assert_eq!(rep.refused[0].id, "Atlas/People/c.md");
        assert_eq!(rep.refused[1].id, "Atlas/People/e.md");
        assert_eq!(rep.counts["c2"], 3);
        assert_eq!(rep.counts["c1"], 1);
        assert_eq!(rep.counts["c3"], 1);
        assert_eq!(rep.notes.len(), 5);
    }

    #[test]
    fn the_report_carries_the_reason_and_not_just_the_class() {
        let rep = report(
            &content_item::VaultFloors::default(),
            &[note("Atlas/Finance/Depot.md", None)],
        );
        assert_eq!(rep.notes[0].class, "c2");
        assert_eq!(rep.notes[0].label, "Others");
        assert!(
            rep.notes[0].rationale.contains("Atlas/Finance"),
            "a class with no stated reason cannot be audited: {}",
            rep.notes[0].rationale
        );
    }
}

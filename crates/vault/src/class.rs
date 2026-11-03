//! What class each note is, and why (PRD Q9a, answered 2026-08-23).
//!
//! The rule is the PRD's: the folder sets the default, a note's frontmatter
//! `class:` key overrides it in either direction. Neither half lives here.
//! `content_item::DataClass::classify_vault_note` holds both, beside the mail
//! rules and the C0–C3 vocabulary they share, because a second place that
//! decides what `c2` means is the one outcome §6.1 forbids. This module is the
//! walk: it hands each note its id and its declared key and counts the answers.
//!
//! ## Why the report names the overrides and the refusals separately
//!
//! A classifier that only prints totals cannot be audited. Two numbers decide
//! whether the folder defaults are doing any work: how many notes overrode
//! them, and how many tried to and misspelled the class. The first is the
//! escape hatch being used as designed. The second is a note whose author
//! believed they had set a class and had not — the failure this whole section
//! exists to make visible rather than silent.

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
    /// Notes that declared a class outside the vocabulary. The folder default
    /// answered for them; each one is a note nobody has actually classified.
    pub refused: Vec<Classified>,
    pub notes: Vec<Classified>,
}

fn classify(note: &Note) -> Classified {
    let class = DataClass::classify_vault_note(&note.id, note.field(CLASS_KEY));
    Classified {
        id: note.id.clone(),
        class: class.value,
        label: class.label,
        method: class.method,
        rationale: class.rationale,
    }
}

pub fn report(notes: &[Note]) -> Report {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut overridden = Vec::new();
    let mut refused = Vec::new();
    let mut all = Vec::with_capacity(notes.len());

    for note in notes {
        let row = classify(note);
        *counts.entry(row.class.clone()).or_insert(0) += 1;

        // A declared key that produced a deterministic answer is a key the
        // vocabulary refused: `classify_vault_note` only reaches `human` when
        // the literal was valid.
        let declared = note
            .field(CLASS_KEY)
            .map(str::trim)
            .is_some_and(|d| !d.is_empty());
        if declared {
            if row.method == content_item::METHOD_HUMAN {
                overridden.push(classify(note));
            } else {
                refused.push(classify(note));
            }
        }
        all.push(row);
    }

    // Every class present in the vocabulary gets a row, including the ones with
    // no notes. A missing key and a zero read the same to a human and do not
    // read the same to whoever is checking that `c3` is empty on purpose.
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
        let rep = report(&[note("Journal/a.md", None)]);
        for class in content_item::DATA_CLASSES {
            assert!(rep.counts.contains_key(class), "{class} has no row");
        }
        assert_eq!(rep.counts["c1"], 1);
        assert_eq!(rep.counts["c0"], 0);
    }

    #[test]
    fn an_override_and_a_refusal_are_counted_apart() {
        let rep = report(&[
            note("Atlas/People/a.md", None),        // folder default
            note("Atlas/People/b.md", Some("c1")),  // honoured override
            note("Atlas/People/c.md", Some("c22")), // refused literal
            note("Journal/d.md", Some("   ")),      // an empty key is not a declaration
        ]);
        assert_eq!(rep.overridden.len(), 1);
        assert_eq!(rep.overridden[0].id, "Atlas/People/b.md");
        assert_eq!(rep.refused.len(), 1);
        assert_eq!(rep.refused[0].id, "Atlas/People/c.md");
        // The refused note is still classified, by its folder, and counted.
        assert_eq!(rep.counts["c2"], 2);
        assert_eq!(rep.counts["c1"], 2);
        assert_eq!(rep.notes.len(), 4);
    }

    #[test]
    fn the_report_carries_the_reason_and_not_just_the_class() {
        let rep = report(&[note("Atlas/Finance/Depot.md", None)]);
        assert_eq!(rep.notes[0].class, "c2");
        assert_eq!(rep.notes[0].label, "Others");
        assert!(
            rep.notes[0].rationale.contains("Atlas/Finance"),
            "a class with no stated reason cannot be audited: {}",
            rep.notes[0].rationale
        );
    }
}

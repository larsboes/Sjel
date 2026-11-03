//! Frontmatter conformance, reported per folder and per dialect.
//!
//! ## Coverage is counted on non-empty values
//!
//! A key present with nothing after it is a template that was never filled in.
//! Counting it as coverage is how a vault reports 80% `summary` while a fifth
//! of those summaries are blank, and it is why this report carries `present`
//! and `filled` separately rather than one number that hides the difference.
//!
//! ## Dialect drift is the finding, not a footnote
//!
//! The same field is written two ways across this vault: `knowledge: reference`
//! on some notes and `knowledge: "reference"` on others; `maturity: evergreen`
//! against `maturity: 🌲`. Both parse to the same value, so a parsed map cannot
//! see the split — which is exactly why every Base in the vault carries a
//! hand-written compatibility shim for it. The raw block is scanned instead, so
//! the drift is a number someone can decide to fix.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::note::Note;

/// The fields this vault's own contract says a note carries.
const TRACKED: &[&str] = &[
    "type",
    "maturity",
    "summary",
    "categories",
    "related",
    "sources",
    "status",
    "aliases",
];

/// Fields whose written form matters, not only their value.
const DIALECT_WATCH: &[&str] = &["knowledge", "maturity", "scope", "type"];

#[derive(Debug, Serialize)]
pub struct FieldCoverage {
    pub field: String,
    pub present: usize,
    pub filled: usize,
}

#[derive(Debug, Serialize)]
pub struct FolderReport {
    pub folder: String,
    pub notes: usize,
    pub no_frontmatter: usize,
    pub fields: Vec<FieldCoverage>,
}

#[derive(Debug, Serialize)]
pub struct DialectReport {
    pub field: String,
    /// Written value as it appears on disk, quotes and all, to occurrence count.
    pub forms: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct LintReport {
    pub notes: usize,
    pub no_frontmatter: usize,
    pub folders: Vec<FolderReport>,
    pub dialects: Vec<DialectReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

/// The written form of `key` in a raw frontmatter block, if the key is there.
///
/// Line-anchored on purpose: a `sources:` line naming a URL that contains
/// `type:` would otherwise be read as a `type` declaration.
pub(crate) fn raw_value<'a>(block: &'a str, k: &str) -> Option<&'a str> {
    block.lines().find_map(|line| {
        let rest = line.strip_prefix(k)?.strip_prefix(':')?;
        // `knowledge:` must not match a line beginning `knowledge_base:`.
        Some(rest.trim())
    })
}

pub fn report(notes: &[Note], problems: Vec<String>) -> LintReport {
    let mut by_folder: BTreeMap<String, Vec<&Note>> = BTreeMap::new();
    for n in notes {
        by_folder
            .entry(if n.folder.is_empty() {
                "(root)".to_string()
            } else {
                n.folder.clone()
            })
            .or_default()
            .push(n);
    }

    let folders = by_folder
        .into_iter()
        .map(|(folder, group)| FolderReport {
            folder,
            notes: group.len(),
            no_frontmatter: group.iter().filter(|n| n.raw_frontmatter.is_none()).count(),
            fields: TRACKED
                .iter()
                .map(|f| FieldCoverage {
                    field: (*f).to_string(),
                    present: group.iter().filter(|n| n.fields.contains_key(*f)).count(),
                    filled: group.iter().filter(|n| n.has(f)).count(),
                })
                .collect(),
        })
        .collect();

    let dialects = DIALECT_WATCH
        .iter()
        .map(|field| {
            let mut forms: BTreeMap<String, usize> = BTreeMap::new();
            for n in notes {
                let Some(block) = &n.raw_frontmatter else {
                    continue;
                };
                if let Some(v) = raw_value(block, field) {
                    if !v.is_empty() {
                        *forms.entry(v.to_string()).or_insert(0) += 1;
                    }
                }
            }
            DialectReport {
                field: (*field).to_string(),
                forms,
            }
        })
        .filter(|d| !d.forms.is_empty())
        .collect();

    LintReport {
        notes: notes.len(),
        no_frontmatter: notes.iter().filter(|n| n.raw_frontmatter.is_none()).count(),
        folders,
        dialects,
        problems,
    }
}

/// Notes carrying a given frontmatter key at all, regardless of value.
///
/// The archive move selects on exactly this: 996 notes carry a `knowledge:`
/// key and no note outside `Knowledge/` does, which makes the vault's own
/// self-label the selector rather than a content classifier nobody can audit.
pub fn carrying<'a>(notes: &'a [Note], key: &str) -> Vec<&'a Note> {
    notes
        .iter()
        .filter(|n| {
            n.raw_frontmatter
                .as_deref()
                .and_then(|b| raw_value(b, key))
                .is_some()
        })
        .collect()
}

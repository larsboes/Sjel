//! The vault's Bases, checked against the vault they query.
//!
//! PRD **D5**: the Bases have been unverified in Obsidian since the `maturity:` → `status:`
//! rewrite, and the row's own remedy is "two minutes of looking" because **a CLI cannot confirm
//! that a Base renders**. That is still true and this module does not claim otherwise.
//!
//! What a CLI can confirm is everything a Base says about the vault before rendering enters it.
//! A Base is a query: it names folders, and it names frontmatter keys it will draw as columns.
//! Both are checkable, and both are wrong in this vault today. Measured 2026-09-08: 28 Bases,
//! 37 folder references, **11 of them across 11 Bases naming one of 10 folders that hold no
//! note** — `TELOS/Focus`, `Projects/Soma/Domains`, `Atlas/Finance/Purchases` — and **106
//! declared columns that no note in scope carries.** A Base whose scope is empty renders an
//! empty table, which looks exactly like a Base that renders correctly over nothing.
//!
//! ## Why a candidate is offered and never applied
//!
//! A `.base` file lives in the vault. §5.5 is one-way — Axon reads the vault and does not write
//! to it — so this verb names the folder that a moved one most likely became and stops there.
//! The rule for "most likely" is deliberately narrow: a folder somewhere in the vault whose last
//! path segment is the same, and which holds at least one note.
//!
//! A matching name is still not a destination, so each candidate is weighed by the only evidence
//! available without opening Obsidian: how many of the Base's own declared columns the notes in
//! that folder actually carry. Measured on this vault 2026-09-08, four references have exactly
//! one candidate and only three of the four survive that: `Atlas/Focus` fills 3 of Focus.base's
//! 3 columns, `Atlas/Reflections` 4 of 5, `Projects/Axon/Knowledge-Base/Domains` 4 of 4 — and
//! `Projects/Archive/Ledger/Notability/Investments`, the lone candidate for
//! `Atlas/Finance/Investments`, fills **0 of 8**. It is an archived Notability import that
//! shares a word with a folder that no longer exists.
//!
//! ## Why every unresolved folder carries a verdict
//!
//! "MISSING" is one word for six different repairs, and a list of them is a list, not a work
//! queue. So each unresolved reference is diagnosed from evidence — what is on disk, what
//! holds a note, and which folders elsewhere end in the same segment — and reported as one of
//! [`Missing`]'s variants. Two of them are the useful ends of the range: `Moved` names a single
//! destination and is a proposal a human can apply; `NeverCreated` says the parent is fine and
//! the leaf never appeared, which is a decision to make rather than a rename to undo.
//!
//! `Spread` is the shape a single path cannot express. `Projects/Tasks` is the example: several
//! folders named `Tasks`, every one of them somewhere under `Projects/`.
//! **Q104 (2026-09-09) rules that `Projects/Tasks/` is never created and a task lives under the
//! project that owns it**, so the repair is a filter that matches the shape, not a path that
//! names the folder. The verdict is structural and not a special case — any missing folder whose
//! namesakes all sit under its own parent reads the same way.
//!
//! This verb does not read the vault's git history, so `NeverCreated` states what was measured:
//! the folder is not on disk and nothing in the vault carries its name. It is not a claim about
//! what was ever committed.
//!
//! ## Why the declared properties are counted too
//!
//! A folder that resolves is not a Base that works. The `maturity:` → `status:` rewrite is
//! exactly the failure that leaves the folder intact and the columns empty, and a column drawn
//! over a key no note carries is the visible half of D5. So each Base's declared properties are
//! counted against the notes actually in its scope, and a zero is reported as a zero rather than
//! as an absence.
//!
//! ## Parsing
//!
//! `.base` is YAML, and this reads it with a line scan for the same reason `note::resolve_root`
//! reads one key out of `knowledge.toml` with one: two constructs are needed — the folder
//! literal inside `file.inFolder("…")` and the key list under `properties:` — and a YAML parser
//! would be a dependency to audit for two constructs. The scan is deliberately conservative:
//! a comment line is skipped, so a folder named in prose is not mistaken for a query.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use crate::note::Note;

/// A folder that could be where a missing one went, and the evidence for it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Candidate {
    pub folder: String,
    pub notes: usize,
    /// How many of this Base's declared columns at least one note in that folder carries. The
    /// number that tells a destination apart from a folder that shares a word with one.
    pub columns_carried: usize,
    pub columns_total: usize,
}

/// Why a folder reference selects nothing, decided from evidence rather than guessed.
///
/// One verdict per unresolved reference, so a report of many of them is a work queue rather
/// than a list. See the module doc for what each one is a repair for, and for why
/// `NeverCreated` is a statement about the disk and not about history.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Missing {
    /// The folder is on disk and holds no markdown note. The notes left; the folder did not.
    Empty,
    /// The folder is now one folder per child of its parent. Every namesake sits under the same
    /// parent, which is a shape no single path can name.
    Spread { under: String, folders: usize },
    /// One folder elsewhere ends in the same segment and carries columns this Base draws.
    Moved {
        to: String,
        columns_carried: usize,
        columns_total: usize,
    },
    /// Several folders end in the same segment and carry this Base's columns. Choosing one
    /// would be inventing a ruling.
    Ambiguous { folders: usize },
    /// Not on disk, and its parent holds notes. Whatever this Base expected to find was never
    /// put there.
    NeverCreated {
        parent: String,
        parent_notes: usize,
        /// Folders elsewhere with the same final segment that carry none of this Base's
        /// columns. A shared word, not a destination.
        namesakes: usize,
    },
    /// Not on disk, and the parent holds no note either — the branch above it went too.
    BranchGone {
        /// The longest prefix of this path that still holds a note, if any.
        deepest: Option<String>,
        namesakes: usize,
    },
}

impl Missing {
    /// One line: the verdict, then the evidence that produced it.
    pub fn explain(&self) -> String {
        match self {
            Missing::Empty => "empty — on disk and holds no note".to_string(),
            Missing::Spread { under, folders } => format!(
                "spread — {folders} folders under `{under}/` end in this name; a single path \
                 cannot reach them"
            ),
            Missing::Moved {
                to,
                columns_carried,
                columns_total,
            } => format!(
                "moved — `{to}` carries {columns_carried} of this Base's {columns_total} columns"
            ),
            Missing::Ambiguous { folders } => format!(
                "moved, destination unclear — {folders} folders share the name and carry this \
                 Base's columns"
            ),
            Missing::NeverCreated {
                parent,
                parent_notes,
                namesakes,
            } => format!(
                "never created — not on disk, and {} holds {parent_notes} {} at or below it{}",
                describe_parent(parent),
                if *parent_notes == 1 { "note" } else { "notes" },
                namesake_note(*namesakes),
            ),
            Missing::BranchGone { deepest, namesakes } => match deepest {
                Some(folder) => format!(
                    "gone with its parent — the deepest folder on this path holding a note is \
                     `{folder}`{}",
                    namesake_note(*namesakes)
                ),
                None => format!(
                    "gone — no folder on this path holds a note{}",
                    namesake_note(*namesakes)
                ),
            },
        }
    }
}

fn describe_parent(parent: &str) -> String {
    if parent.is_empty() {
        "the vault root".to_string()
    } else {
        format!("`{parent}`")
    }
}

fn namesake_note(namesakes: usize) -> String {
    match namesakes {
        0 => String::new(),
        1 => "; 1 folder shares the name and carries none of this Base's columns".to_string(),
        n => format!("; {n} folders share the name and carry none of this Base's columns"),
    }
}

/// One `file.inFolder("…")` reference, and whether the vault answers it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderRef {
    pub folder: String,
    /// `!file.inFolder(…)` — an exclusion. A missing exclusion folder excludes nothing, which
    /// is harmless, so it is reported and not counted as unresolved.
    pub excluded: bool,
    /// Notes at or below the folder. `inFolder` is recursive in Obsidian, so this is too.
    pub notes: usize,
    /// Folders in the vault that end in the same segment and hold at least one note, best
    /// evidence first. Empty when the reference resolved; one entry is a proposal, several are
    /// a question.
    pub candidates: Vec<Candidate>,
    /// Why the reference selects nothing. `None` when it resolved.
    pub missing: Option<Missing>,
}

impl FolderRef {
    pub fn resolved(&self) -> bool {
        self.notes > 0
    }
}

/// One frontmatter key a Base declares as a column, and how many notes in scope carry it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldRef {
    pub field: String,
    pub carried: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseReport {
    /// Vault-relative id of the `.base` file.
    pub id: String,
    pub views: usize,
    pub folders: Vec<FolderRef>,
    /// Notes the Base's positive folder references select, deduplicated. `None` when the Base
    /// names no folder at all — `Tasks.base` selects on a tag and a `type:` key — because a
    /// field count against the whole vault would answer a question nobody asked.
    pub scope: Option<usize>,
    pub fields: Vec<FieldRef>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BasesReport {
    pub bases: usize,
    pub folder_refs: usize,
    /// Positive references naming a folder that holds no note. The number `--strict` fails on.
    pub unresolved_folders: usize,
    /// Declared columns that no note in scope carries. The `maturity:` → `status:` shape.
    pub empty_fields: usize,
    pub items: Vec<BaseReport>,
}

/// Every `file.inFolder("…")` in a `.base`, with the `!` in front of it if there was one.
///
/// Deduplicated by folder, because `Map.base` names `Atlas/Events` seven times across a filter,
/// two formulas and four views, and seven rows saying the same thing about one folder is a
/// report a reader stops reading.
fn folder_refs(text: &str) -> Vec<(String, bool)> {
    const CALL: &str = "file.inFolder(\"";
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find(CALL) {
            let head = &rest[..at];
            let excluded = head.trim_end().ends_with('!');
            let tail = &rest[at + CALL.len()..];
            if let Some(end) = tail.find('"') {
                let folder = tail[..end].trim_end_matches('/').to_string();
                if !folder.is_empty() {
                    // A folder named positively anywhere is a folder the Base reads, even if
                    // another line excludes it.
                    out.entry(folder)
                        .and_modify(|e| *e = *e && excluded)
                        .or_insert(excluded);
                }
                rest = &tail[end + 1..];
            } else {
                break;
            }
        }
    }
    out.into_iter().collect()
}

/// The keys under the top-level `properties:` block, in file order.
///
/// `file.*` and `formula.*` are dropped: they are computed by Obsidian and carry nothing a note
/// could fail to have, which is the only thing this count is for. `note.*` is the third
/// namespace and the opposite case — it is Obsidian's explicit spelling of a frontmatter key,
/// so `note.color` is the key `color` and the prefix comes off. `Map.base` is the only Base in
/// this vault that spells it out.
///
/// What that costs today is smaller than it first read, and the smaller number is the honest
/// one: both of `Map.base`'s `note.` keys are carried — `color` by 11 of the 111 notes in its
/// scope, `icon` by 12 — so dropping them with `file.` and `formula.` would have hidden two
/// columns that work, not two empty ones. `declared columns empty` is 106 either way, measured
/// both ways on the operator's vault 2026-09-08. The rule earns its place because the next
/// `note.` key a Base declares has no reason to be carried, not because it moved that total.
fn declared_fields(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("properties:") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if !line.trim().is_empty() && indent == 0 {
            break; // the next top-level key ends the block
        }
        if indent != 2 {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(field) = trimmed.strip_suffix(':') else {
            continue;
        };
        let field = field.trim_matches(['"', '\'']);
        if field.starts_with("file.") || field.starts_with("formula.") {
            continue;
        }
        let field = field.strip_prefix("note.").unwrap_or(field);
        if field.is_empty() {
            continue;
        }
        out.push(field.to_string());
    }
    out
}

/// Top-level entries under `views:`. Counted only so a reader can see a Base is not empty.
fn view_count(text: &str) -> usize {
    let mut inside = false;
    let mut n = 0;
    for line in text.lines() {
        if line.starts_with("views:") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if !line.trim().is_empty() && indent == 0 {
            break;
        }
        if indent == 2 && line.trim_start().starts_with("- ") {
            n += 1;
        }
    }
    n
}

/// Every directory under the vault root, vault-relative, slash-separated.
///
/// Read from the disk rather than derived from note ids, because that is the whole of the
/// difference between "the folder is gone" and "the folder is there and its notes are not" —
/// and the second one renders in Obsidian as a folder a reader can click. Dot-directories are
/// skipped for the reason `markdown_files_recursive` skips them: `.trash` and `.obsidian` are
/// not the vault.
///
/// A directory that cannot be listed, or an entry in it whose type cannot be read, is an error
/// rather than an omission. Skipping it would report a folder that exists as one that never
/// did, which is the one wrong answer this whole verdict exists to avoid.
pub fn folders_on_disk(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("{}: unreadable: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: unreadable entry: {e}", dir.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| format!("{}: unreadable: {e}", entry.path().display()))?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if let Ok(rel) = path.strip_prefix(root) {
                out.insert(
                    rel.to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/"),
                );
            }
            stack.push(path);
        }
    }
    Ok(out)
}

/// The longest prefix of `folder` that holds at least one note.
fn deepest_present(folder: &str, holding: &BTreeSet<String>) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').collect();
    parts.pop();
    while !parts.is_empty() {
        let candidate = parts.join("/");
        if holding.contains(&candidate) {
            return Some(candidate);
        }
        parts.pop();
    }
    None
}

/// Which of the six repairs an unresolved reference needs.
///
/// The order is the order of the evidence's strength. What is on disk beats what shares a name,
/// and what shares a name beats what is absent — so a folder that is there and empty never reads
/// as one that was never made.
fn diagnose(
    folder: &str,
    candidates: &[Candidate],
    dirs: &BTreeSet<String>,
    holding: &BTreeSet<String>,
    notes: &[Note],
) -> Missing {
    if dirs.contains(folder) {
        return Missing::Empty;
    }
    let parent = folder.rsplit_once('/').map(|(p, _)| p).unwrap_or("");

    // Spread: every namesake sits under this folder's own parent. `Projects/Tasks` against
    // `Projects/**/Tasks` is the shape, and it is the one a path cannot express.
    if !parent.is_empty() && candidates.len() > 1 {
        let under = format!("{parent}/");
        if candidates.iter().all(|c| c.folder.starts_with(&under)) {
            return Missing::Spread {
                under: parent.to_string(),
                folders: candidates.len(),
            };
        }
    }

    // A Base that declares no column offers no column evidence, so every namesake counts.
    // Filtering on `columns_carried > 0` there would discard the only candidate on a
    // technicality.
    let weighed = candidates.first().is_some_and(|c| c.columns_total > 0);
    let strong: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| !weighed || c.columns_carried > 0)
        .collect();
    match strong.len() {
        1 => {
            return Missing::Moved {
                to: strong[0].folder.clone(),
                columns_carried: strong[0].columns_carried,
                columns_total: strong[0].columns_total,
            }
        }
        n if n > 1 => return Missing::Ambiguous { folders: n },
        _ => {}
    }

    let namesakes = candidates.len();
    if parent.is_empty() || holding.contains(parent) {
        let prefix = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}/")
        };
        return Missing::NeverCreated {
            parent: parent.to_string(),
            parent_notes: notes.iter().filter(|n| n.id.starts_with(&prefix)).count(),
            namesakes,
        };
    }
    Missing::BranchGone {
        deepest: deepest_present(folder, holding),
        namesakes,
    }
}

/// Every folder in the vault that holds at least one note, at or below it.
fn folders_holding_notes(notes: &[Note]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for note in notes {
        let Some((dir, _)) = note.id.rsplit_once('/') else {
            continue;
        };
        let mut parts: Vec<&str> = dir.split('/').collect();
        while !parts.is_empty() {
            out.insert(parts.join("/"));
            parts.pop();
        }
    }
    out
}

/// The report.
///
/// `bases` are `(id, text)` pairs — the `.base` files as the caller read them. `dirs` is every
/// directory under the vault root, from [`folders_on_disk`]. Both are passed in rather than read
/// here so the walk stays in one place and a test can hand over a vault that never touched a
/// disk.
pub fn report(notes: &[Note], bases: &[(String, String)], dirs: &BTreeSet<String>) -> BasesReport {
    let all_folders = folders_holding_notes(notes);
    let mut out = BasesReport {
        bases: bases.len(),
        ..BasesReport::default()
    };

    for (id, text) in bases {
        // The declared columns come first because a candidate is weighed against them.
        let declared = declared_fields(text);
        let mut folders = Vec::new();
        let mut scope_ids: BTreeSet<&str> = BTreeSet::new();
        let mut has_positive = false;

        for (folder, excluded) in folder_refs(text) {
            let prefix = format!("{folder}/");
            let inside: Vec<&Note> = notes
                .iter()
                .filter(|note| note.id.starts_with(&prefix))
                .collect();
            let candidates = if inside.is_empty() {
                let tail = folder.rsplit('/').next().unwrap_or(&folder);
                let mut hits: Vec<Candidate> = all_folders
                    .iter()
                    .filter(|f| f.rsplit('/').next() == Some(tail) && *f != &folder)
                    .map(|f| {
                        let under: Vec<&Note> = notes
                            .iter()
                            .filter(|note| note.id.starts_with(&format!("{f}/")))
                            .collect();
                        Candidate {
                            folder: f.clone(),
                            notes: under.len(),
                            columns_carried: declared
                                .iter()
                                .filter(|field| under.iter().any(|note| note.has(field)))
                                .count(),
                            columns_total: declared.len(),
                        }
                    })
                    .collect();
                hits.sort_by(|a, b| {
                    b.columns_carried
                        .cmp(&a.columns_carried)
                        .then(a.folder.cmp(&b.folder))
                });
                hits
            } else {
                Vec::new()
            };
            if !excluded {
                has_positive = true;
                scope_ids.extend(inside.iter().map(|note| note.id.as_str()));
                if inside.is_empty() {
                    out.unresolved_folders += 1;
                }
            }
            out.folder_refs += 1;
            // Diagnosed for an exclusion too. A `!file.inFolder(…)` naming a folder that is not
            // there is harmless, but it is still a stale line in the Base, and the verdict is
            // what tells the operator whether to repoint it or delete it.
            let missing = inside
                .is_empty()
                .then(|| diagnose(&folder, &candidates, dirs, &all_folders, notes));
            folders.push(FolderRef {
                folder,
                excluded,
                notes: inside.len(),
                candidates,
                missing,
            });
        }

        let scope = has_positive.then_some(scope_ids.len());
        let fields = declared
            .into_iter()
            .map(|field| {
                let carried = if has_positive {
                    notes
                        .iter()
                        .filter(|note| scope_ids.contains(note.id.as_str()) && note.has(&field))
                        .count()
                } else {
                    0
                };
                if has_positive && carried == 0 {
                    out.empty_fields += 1;
                }
                FieldRef { field, carried }
            })
            .collect();

        out.items.push(BaseReport {
            id: id.clone(),
            views: view_count(text),
            folders,
            scope,
            fields,
        });
    }

    out.items.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, fields: &[(&str, &str)]) -> Note {
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
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body_start: 0,
            raw_frontmatter: None,
            text: String::new(),
        }
    }

    /// A vault whose folders are absent from the disk as well as from the note index. That is
    /// the usual shape of an unresolved reference, so it is the default; `Missing::Empty` is the one verdict that needs a directory listing and its own
    /// test builds one.
    fn report_off_disk(notes: &[Note], bases: &[(String, String)]) -> BasesReport {
        report(notes, bases, &BTreeSet::new())
    }

    const MOVED: &str = "filters:\n  and:\n    - file.inFolder(\"TELOS/Focus\")\n\
                         properties:\n  file.name:\n    displayName: Focus Area\n  \
                         current_focus:\n    displayName: Active\nviews:\n  - type: cards\n";

    /// The shape D5 is about: the Base is intact, the folder moved, and the Base now selects
    /// nothing. One candidate is a proposal a human can act on.
    #[test]
    fn a_base_whose_folder_moved_is_unresolved_and_carries_one_candidate() {
        let notes = vec![note("Atlas/Focus/Polymath.md", &[("current_focus", "AI")])];
        let rep = report_off_disk(&notes, &[("Focus.base".into(), MOVED.into())]);
        assert_eq!(rep.unresolved_folders, 1);
        assert_eq!(
            rep.items[0].folders[0].candidates,
            vec![Candidate {
                folder: "Atlas/Focus".into(),
                notes: 1,
                columns_carried: 1,
                columns_total: 1,
            }]
        );
        assert_eq!(rep.items[0].scope, Some(0));
        assert_eq!(rep.items[0].views, 1);
    }

    /// The check that turned four single-candidate references into three proposals on the real
    /// vault. A folder that shares a final segment and carries none of the Base's columns is a
    /// name collision — `Projects/Archive/Ledger/Notability/Investments` against
    /// `Atlas/Finance/Investments` — and it is ranked below the one that does.
    #[test]
    fn a_candidate_that_carries_none_of_the_columns_ranks_below_one_that_does() {
        let notes = vec![
            note("Archive/Old/Focus/Scan.md", &[]),
            note("Atlas/Focus/Polymath.md", &[("current_focus", "AI")]),
        ];
        let rep = report_off_disk(&notes, &[("Focus.base".into(), MOVED.into())]);
        let candidates = &rep.items[0].folders[0].candidates;
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            (candidates[0].folder.as_str(), candidates[0].columns_carried),
            ("Atlas/Focus", 1)
        );
        assert_eq!(
            (candidates[1].folder.as_str(), candidates[1].columns_carried),
            ("Archive/Old/Focus", 0)
        );
    }

    /// The control. Same Base, pointed at the folder that exists: nothing unresolved, no
    /// candidate offered, and the column it declares is counted as carried. Without this the
    /// test above would pass against a checker that calls everything broken.
    #[test]
    fn the_same_base_pointed_at_the_folder_that_exists_resolves_clean() {
        let notes = vec![note("Atlas/Focus/Polymath.md", &[("current_focus", "AI")])];
        let text = MOVED.replace("TELOS/Focus", "Atlas/Focus");
        let rep = report_off_disk(&notes, &[("Focus.base".into(), text)]);
        assert_eq!((rep.unresolved_folders, rep.empty_fields), (0, 0));
        assert!(rep.items[0].folders[0].candidates.is_empty());
        assert_eq!(rep.items[0].scope, Some(1));
        assert_eq!(
            rep.items[0].fields,
            vec![FieldRef {
                field: "current_focus".into(),
                carried: 1,
            }]
        );
    }

    /// The other half of D5, and the half a resolving folder hides: the `maturity:` → `status:`
    /// rewrite leaves the folder intact and the column empty.
    #[test]
    fn a_column_no_note_carries_is_reported_as_empty_even_though_the_folder_resolves() {
        let notes = vec![note("Knowledge/Rust.md", &[("status", "evergreen")])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Knowledge\")\n\
                    properties:\n  maturity:\n    displayName: Maturity\n  status:\n    displayName: Status\n";
        let rep = report_off_disk(&notes, &[("Knowledge.base".into(), text.into())]);
        assert_eq!((rep.unresolved_folders, rep.empty_fields), (0, 1));
        assert_eq!(
            rep.items[0].fields,
            vec![
                FieldRef {
                    field: "maturity".into(),
                    carried: 0,
                },
                FieldRef {
                    field: "status".into(),
                    carried: 1,
                },
            ]
        );
    }

    /// Several candidates are not a proposal, and Q104 says why: `Projects/Tasks` is never
    /// created, so its namesakes are one `Tasks/` folder per project. A verb that picked one of
    /// them would be inventing a ruling; `Spread` names the shape instead.
    #[test]
    fn a_folder_whose_namesakes_all_sit_under_its_parent_reads_as_spread() {
        let notes = vec![
            note("Projects/Axon/Tasks/Ship it.md", &[]),
            note("Projects/Home-Lab/Tasks/Buy a drive.md", &[]),
        ];
        let text = "filters:\n  and:\n    - file.inFolder(\"Projects/Tasks\")\n";
        let rep = report_off_disk(&notes, &[("Tasks.base".into(), text.into())]);
        let folder = &rep.items[0].folders[0];
        let folders: Vec<&str> = folder
            .candidates
            .iter()
            .map(|c| c.folder.as_str())
            .collect();
        assert_eq!(
            folders,
            vec!["Projects/Axon/Tasks", "Projects/Home-Lab/Tasks"]
        );
        assert_eq!(
            folder.missing,
            Some(Missing::Spread {
                under: "Projects".into(),
                folders: 2,
            })
        );
        assert!(folder
            .missing
            .as_ref()
            .unwrap()
            .explain()
            .contains("spread"));
    }

    /// The lower edge of `Spread`. One namesake under the same parent is not a shape, it is an
    /// address — so it is proposed as a move a human can apply, which is the more useful of the
    /// two answers. Without this the rule could be "any namesake under the parent" and nothing
    /// would notice.
    #[test]
    fn a_single_namesake_under_the_parent_is_a_move_rather_than_a_shape() {
        let notes = vec![note("Projects/Axon/Tasks/Ship it.md", &[("done", "false")])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Projects/Tasks\")\n\
                    properties:\n  done:\n    displayName: Done\n";
        let rep = report_off_disk(&notes, &[("Tasks.base".into(), text.into())]);
        assert_eq!(
            rep.items[0].folders[0].missing,
            Some(Missing::Moved {
                to: "Projects/Axon/Tasks".into(),
                columns_carried: 1,
                columns_total: 1,
            })
        );
    }

    /// The control for `Spread`, and the reason it is not just "more than one candidate". Move
    /// one namesake out from under `Projects/` and the same two folders stop being
    /// one-per-project: the answer becomes "moved, destination unclear", a different repair.
    #[test]
    fn namesakes_under_different_parents_are_ambiguous_rather_than_spread() {
        let notes = vec![
            note("Projects/Axon/Tasks/Ship it.md", &[]),
            note("Atlas/Tasks/Buy a drive.md", &[]),
        ];
        let text = "filters:\n  and:\n    - file.inFolder(\"Projects/Tasks\")\n";
        let rep = report_off_disk(&notes, &[("Tasks.base".into(), text.into())]);
        assert_eq!(
            rep.items[0].folders[0].missing,
            Some(Missing::Ambiguous { folders: 2 })
        );
    }

    /// The verdict that tells a repair from a decision. `Atlas/Places` has no namesake anywhere
    /// and its parent holds notes, so nothing moved — the folder the Base draws was never put
    /// there. The sentence carries the evidence a reader would otherwise have to go and check.
    #[test]
    fn a_folder_with_no_namesake_whose_parent_holds_notes_was_never_created() {
        let notes = vec![note("Atlas/People/Erika.md", &[("city", "Berlin")])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Atlas/Places\")\n";
        let rep = report_off_disk(&notes, &[("Places.base".into(), text.into())]);
        let missing = rep.items[0].folders[0].missing.clone().unwrap();
        assert_eq!(
            missing,
            Missing::NeverCreated {
                parent: "Atlas".into(),
                parent_notes: 1,
                namesakes: 0,
            }
        );
        assert_eq!(
            missing.explain(),
            "never created — not on disk, and `Atlas` holds 1 note at or below it"
        );
    }

    /// The planted bad input for the verdict above: a directory listing that says the folder is
    /// there. Same notes, same Base, same zero rows — and the verdict has to change, because
    /// "make the folder" and "put notes back in it" are not the same job. An instrument that
    /// answered `never_created` to both would be reading the note index and calling it the disk.
    #[test]
    fn a_folder_that_is_on_disk_and_holds_no_note_is_empty_not_never_created() {
        let notes = vec![note("Atlas/People/Erika.md", &[("city", "Berlin")])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Atlas/Places\")\n";
        let dirs: BTreeSet<String> = ["Atlas".to_string(), "Atlas/Places".to_string()]
            .into_iter()
            .collect();
        let rep = report(&notes, &[("Places.base".into(), text.into())], &dirs);
        assert_eq!(rep.unresolved_folders, 1);
        assert_eq!(rep.items[0].folders[0].missing, Some(Missing::Empty));
    }

    /// A namesake that carries none of the Base's columns is not a destination, so it does not
    /// earn `Moved`. The verdict falls through to the absence it actually is, and says how many
    /// folders shared the word — which is why an archived import such as
    /// `Projects/Archive/Ledger/Investments` is not proposed for `Atlas/Finance/Investments`.
    #[test]
    fn a_namesake_carrying_no_column_does_not_earn_a_moved_verdict() {
        let notes = vec![
            note(
                "Atlas/Finance/Subscriptions/Netflix.md",
                &[("amount", "12")],
            ),
            note("Projects/Archive/Ledger/Investments/Scan.md", &[]),
        ];
        let text = "filters:\n  and:\n    - file.inFolder(\"Atlas/Finance/Investments\")\n\
                    properties:\n  asset_class:\n    displayName: Asset Class\n";
        let rep = report_off_disk(&notes, &[("Investments.base".into(), text.into())]);
        let missing = rep.items[0].folders[0].missing.clone().unwrap();
        assert_eq!(
            missing,
            Missing::NeverCreated {
                parent: "Atlas/Finance".into(),
                parent_notes: 1,
                namesakes: 1,
            }
        );
        assert!(missing
            .explain()
            .ends_with("1 folder shares the name and carries none of this Base's columns"));
    }

    /// The branch above the folder went too. `TELOS/Focus` would read this way if nothing in
    /// the vault were named `Focus` any more, and the deepest surviving folder on the path is
    /// the only address a reader can start from.
    #[test]
    fn a_folder_whose_parent_holds_no_note_reads_as_gone_with_its_parent() {
        let notes = vec![note("Atlas/People/Erika.md", &[])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Atlas/Old/Archive/Deep\")\n";
        let rep = report_off_disk(&notes, &[("Old.base".into(), text.into())]);
        assert_eq!(
            rep.items[0].folders[0].missing,
            Some(Missing::BranchGone {
                deepest: Some("Atlas".into()),
                namesakes: 0,
            })
        );
    }

    /// A Base that declares no column has no column evidence, so the single namesake is still
    /// the proposal. Weighing candidates on a number that cannot be anything but zero would
    /// throw away the only answer available.
    #[test]
    fn a_base_with_no_declared_column_still_gets_its_one_candidate_proposed() {
        let notes = vec![note("Atlas/Focus/Polymath.md", &[])];
        let text = "filters:\n  and:\n    - file.inFolder(\"TELOS/Focus\")\n";
        let rep = report_off_disk(&notes, &[("Focus.base".into(), text.into())]);
        assert_eq!(
            rep.items[0].folders[0].missing,
            Some(Missing::Moved {
                to: "Atlas/Focus".into(),
                columns_carried: 0,
                columns_total: 0,
            })
        );
    }

    /// A reference that resolves carries no verdict at all. Without this the verdict could be a
    /// field that is always populated, which says nothing.
    #[test]
    fn a_resolved_reference_carries_no_verdict() {
        let notes = vec![note("Knowledge/Rust.md", &[])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Knowledge\")\n";
        let rep = report_off_disk(&notes, &[("Knowledge.base".into(), text.into())]);
        assert_eq!(rep.items[0].folders[0].missing, None);
    }

    /// An exclusion that names a folder which is not there excludes nothing, so it is not a
    /// defect. Counting it would put `!file.inFolder("Resources/Templates")` on the failure list
    /// of every Base that carries it.
    #[test]
    fn a_negated_reference_is_not_counted_as_unresolved() {
        let notes = vec![note("Projects/Axon/Axon.md", &[])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Projects\")\n    \
                    - '!file.inFolder(\"Resources/Templates\")'\n";
        let rep = report_off_disk(&notes, &[("Projects.base".into(), text.into())]);
        assert_eq!(rep.unresolved_folders, 0);
        assert!(rep.items[0].folders[1].excluded);
        assert_eq!(rep.items[0].scope, Some(1));
    }

    /// A Base that selects on a tag and a `type:` key has no folder scope, and a field count
    /// taken against the whole vault would be a number nobody asked for.
    #[test]
    fn a_base_with_no_folder_reports_no_scope_rather_than_the_whole_vault() {
        let notes = vec![note("Projects/Axon/Tasks/Ship it.md", &[("done", "false")])];
        let text = "filters:\n  and:\n    - type == \"task\"\nproperties:\n  done:\n    displayName: Done\n";
        let rep = report_off_disk(&notes, &[("Tasks.base".into(), text.into())]);
        assert_eq!((rep.items[0].scope, rep.empty_fields), (None, 0));
        assert_eq!(rep.items[0].fields[0].carried, 0);
    }

    /// The three property namespaces, told apart. `file.` and `formula.` are Obsidian's and no
    /// note can fail to carry them; `note.` is a frontmatter key with its namespace spelled out,
    /// and treating it as Obsidian's would hide the column rather than check it.
    #[test]
    fn the_note_namespace_is_a_frontmatter_key_and_the_other_two_are_not() {
        let notes = vec![note("Atlas/People/Erika.md", &[("color", "blue")])];
        let text = "filters:\n  and:\n    - file.inFolder(\"Atlas/People\")\nproperties:\n  \
                    file.name:\n    displayName: Name\n  formula.age:\n    displayName: Age\n  \
                    note.color:\n    displayName: Color\n  note.icon:\n    displayName: Icon\n";
        let rep = report_off_disk(&notes, &[("Map.base".into(), text.into())]);
        assert_eq!(
            rep.items[0].fields,
            vec![
                FieldRef {
                    field: "color".into(),
                    carried: 1,
                },
                FieldRef {
                    field: "icon".into(),
                    carried: 0,
                },
            ]
        );
        assert_eq!(rep.empty_fields, 1);
    }

    /// A folder named in a comment is prose, not a query.
    #[test]
    fn a_folder_named_in_a_comment_is_not_a_reference() {
        let notes = vec![note("Knowledge/Rust.md", &[])];
        let text = "# copied from file.inFolder(\"TELOS/Gone\")\nfilters:\n  and:\n    - file.inFolder(\"Knowledge\")\n";
        let rep = report_off_disk(&notes, &[("Knowledge.base".into(), text.into())]);
        assert_eq!((rep.folder_refs, rep.unresolved_folders), (1, 0));
    }
}

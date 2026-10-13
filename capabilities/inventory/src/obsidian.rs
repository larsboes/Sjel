//! A slot's way into the vault, and the one block Axon owns there.
//!
//! A slot is an open decision with target sizes, and decisions live in the vault. On the first
//! run a note is seeded; its prose sits outside the markers and belongs to the person from then
//! on. Every later run touches only the marked region, which holds machine numbers and never a
//! sentence a person wrote. Pieces stay rows and get no note.

use std::collections::BTreeMap;
use std::path::PathBuf;

use markdown_root::{region, MarkdownRoot, RegionOutcome, RegionSpec, RootError};

use crate::store::{Item, Kind, State};

/// The marker owner. Stable forever: a change makes every region already written foreign.
pub const REGION_OWNER: &str = "interior";

/// Raised when the rendered block changes shape.
pub const REGION_VERSION: u32 = 1;

/// Vault-relative and fixed, so two hosts cannot write into two folders.
pub const DIR: &str = "Atlas/Interior";

#[derive(Debug, Default)]
pub struct Report {
    /// Notes created on this run.
    pub seeded: Vec<String>,
    /// Notes whose region was rewritten.
    pub written: Vec<String>,
    /// Notes whose region was already current; nothing written.
    pub unchanged: Vec<String>,
    /// Notes whose region a person edited; nothing written.
    pub conflicts: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("vault root: {0}")]
    Root(#[from] RootError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("region in {path}: {source}")]
    Region {
        path: PathBuf,
        #[source]
        source: region::RegionError,
    },
}

/// A slot's file name, from the label because a person opens the note. A slash would invent a
/// folder, so it becomes a dash.
pub fn file_name(item: &Item) -> String {
    let mut out = String::with_capacity(item.label.len());
    for ch in item.label.chars() {
        match ch {
            '/' | '\\' | ':' => out.push('-'),
            '#' | '^' | '[' | ']' | '|' => out.push(' '),
            c => out.push(c),
        }
    }
    let trimmed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        item.id.clone()
    } else {
        trimmed
    }
}

fn line(out: &mut String, label: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("> **{label}:** {value}\n"));
    }
}

fn dimensions(item: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(w) = item.width {
        parts.push(format!("W {w}"));
    }
    if let Some(d) = item.depth {
        parts.push(format!("D {d}"));
    }
    if let Some(h) = item.height {
        parts.push(format!("H {h}"));
    }
    if let Some(h) = item.height_min {
        parts.push(format!("H min {h}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} cm", parts.join(" × "))
    }
}

fn euro(cents: i64) -> String {
    format!("{},{:02} €", cents / 100, (cents % 100).abs())
}

fn price(item: &Item) -> String {
    match (item.price_cents, item.cost_min_cents, item.cost_max_cents) {
        (Some(p), _, _) => euro(p),
        (None, Some(min), Some(max)) if min != max => format!("{} to {}", euro(min), euro(max)),
        (None, Some(min), _) => format!("from {}", euro(min)),
        (None, None, Some(max)) => format!("up to {}", euro(max)),
        _ => String::new(),
    }
}

/// The block Axon owns: numbers only, never a sentence a person wrote.
pub fn render_block(item: &Item, state: Option<State>) -> String {
    let mut out = String::new();
    out.push_str("> [!info] Derived by Axon. Do not edit inside this block\n");

    let text = |v: &Option<String>| v.as_deref().filter(|s| !s.trim().is_empty()).map(str::to_string);
    line(&mut out, "Target size", &dimensions(item));
    line(&mut out, "Price", &price(item));
    if let Some(p) = text(&item.priority) {
        line(&mut out, "Priority", &p);
    }
    if let Some(s) = state {
        line(&mut out, "State", s.as_str());
    }
    if let Some(zone) = item.clearance_zone {
        line(&mut out, "Clearance zone", &format!("{zone} cm"));
    }
    if let Some(block) = item.clearance_block {
        line(&mut out, "Clearance block", &format!("{block} cm"));
    }
    if let Some(c) = item.count.filter(|n| *n != 1) {
        line(&mut out, "Count", &c.to_string());
    }
    if !item.replaces.is_empty() {
        line(&mut out, "Replaces", &item.replaces.join(", "));
    }
    if !item.variants.is_empty() {
        line(&mut out, "Variants", &item.variants.join(", "));
    }
    if let Some(b) = text(&item.based_on) {
        line(&mut out, "Based on", &b);
    }
    if let Some(l) = text(&item.link) {
        line(&mut out, "Link", &l);
    }
    if let Some(s) = text(&item.source) {
        line(&mut out, "Measured from", &s);
    }
    if let Some(m) = text(&item.measured_at) {
        line(&mut out, "Measured on", &m);
    }
    // In the region, not the prose: a caveat a person can delete by accident is none.
    if !item.estimated.is_empty() {
        line(&mut out, "Estimated", &item.estimated.join(", "));
    }

    out.push_str(&format!(
        "\n> `{}` · from the inventory table, not maintained by hand.\n",
        item.id
    ));
    out
}

/// The one-time seed of a note whose prose belongs to the person. The frontmatter keys and values
/// are kept as earlier notes have them, so vault queries keep matching.
fn seed(item: &Item) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("type: interior-slot\n");
    out.push_str(&format!("axon_interior_id: \"{}\"\n", item.id));
    out.push_str(&format!("summary: \"{}\"\n", item.label.replace('"', "'")));
    out.push_str("status: offen\n");
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", item.label));

    let mut section = |heading: &str, text: Option<&String>| {
        if let Some(t) = text.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            out.push_str(&format!("## {heading}\n\n{t}\n\n"));
        }
    };
    section("Target", item.target.as_ref());
    section("Reason", item.reason.as_ref());
    section("Notes", item.notes.as_ref());
    section("Open question", item.open_question.as_ref());

    out.push_str(
        "> [!tip] The text from here on is yours\n\
         > This note was created once from the inventory table. Everything outside the Axon\n\
         > block below is yours to change; it is never overwritten.\n\n",
    );
    // The region is appended by `region::apply`, which also writes the hash conflict detection
    // relies on.
    out
}

/// Bring every slot into the vault: seed once, then maintain only the region.
pub fn write_all(
    root: &MarkdownRoot,
    catalogue: &BTreeMap<String, (Item, Option<State>)>,
) -> Result<Report, Error> {
    let mut report = Report::default();
    let spec = RegionSpec::new(REGION_OWNER, REGION_VERSION);

    let dir = root.path().join(DIR);
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;

    for (item, state) in catalogue.values() {
        if item.kind != Kind::Slot {
            continue;
        }

        let relative = format!("{DIR}/{}.md", file_name(item));
        let path = root.locate(&relative)?;
        let name = relative.clone();

        let existed = path.exists();
        if !existed {
            std::fs::write(&path, seed(item)).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            report.seeded.push(name.clone());
        }

        let original = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let (updated, outcome) = region::apply(&original, &spec, &render_block(item, *state))
            .map_err(|source| Error::Region {
                path: path.clone(),
                source,
            })?;

        match outcome {
            RegionOutcome::Created | RegionOutcome::Updated => {
                std::fs::write(&path, updated).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
                // A note seeded on this run counts as `seeded` only.
                if existed {
                    report.written.push(name);
                }
            }
            RegionOutcome::Unchanged => {
                if existed {
                    report.unchanged.push(name);
                }
            }
            RegionOutcome::Conflict { .. } => report.conflicts.push(name),
        }
    }

    Ok(report)
}

/// This machine's vault root, if one is declared; `None` means no vault and no writes.
/// `inventory.json` first, then `interior.json`, where existing machines already declare it.
pub fn vault_root() -> Option<PathBuf> {
    for name in [
        "SJEL_INVENTORY_OBSIDIAN_ROOT",
        "SJEL_INTERIOR_OBSIDIAN_ROOT",
    ] {
        if let Ok(p) = sjel_config::env_var(name) {
            if !p.trim().is_empty() {
                return Some(sjel_config::expand_tilde(&p));
            }
        }
    }
    for file in ["inventory.json", "interior.json"] {
        let Some(path) = sjel_config::overlay_config(file) else {
            continue;
        };
        let Some(text) = std::fs::read_to_string(path).ok() else {
            continue;
        };
        let Some(value) = serde_json::from_str::<serde_json::Value>(&text).ok() else {
            continue;
        };
        if let Some(root) = value
            .get("obsidian")
            .and_then(|o| o.get("root"))
            .and_then(|r| r.as_str())
        {
            return Some(sjel_config::expand_tilde(root));
        }
    }
    None
}

/// `write_all` against the declared root, or `None` when none is declared.
pub fn writeback(
    catalogue: &BTreeMap<String, (Item, Option<State>)>,
) -> Option<Result<Report, Error>> {
    let root_path = vault_root()?;
    Some(
        MarkdownRoot::declare(root_path)
            .map_err(Error::Root)
            .and_then(|root| write_all(&root, catalogue)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: &str, label: &str) -> Item {
        Item {
            id: id.into(),
            kind: Kind::Slot,
            label: label.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_slash_in_the_label_is_no_folder() {
        let it = slot("vorhang", "Vorhang / Verdunkelung Terrassentuer");
        let name = file_name(&it);
        assert!(!name.contains('/'), "got: {name}");
        assert_eq!(name, "Vorhang - Verdunkelung Terrassentuer");
    }

    /// A `.md` without a stem would be invisible in Obsidian.
    #[test]
    fn an_empty_label_falls_back_to_the_id() {
        let it = slot("kleiderschrank", "   ");
        assert_eq!(file_name(&it), "kleiderschrank");
    }

    /// No human sentence is copied into the region; otherwise a later run would have to decide
    /// between two versions of it.
    #[test]
    fn the_region_reproduces_no_human_sentence() {
        let mut it = slot("chair", "Second chair");
        it.reason = Some("A second chair is a coat stand with legs.".into());
        it.target = Some("Foldable, under 40 cm deep.".into());
        it.notes = Some("Only after the table decision.".into());
        it.open_question = Some("Foldable or not at all?".into());
        it.width = Some(45);

        let block = render_block(&it, Some(State::Wanted));
        for sentence in [
            "coat stand with legs",
            "Foldable, under 40",
            "Only after the table",
            "Foldable or not at all",
        ] {
            assert!(!block.contains(sentence), "region carries prose: {sentence}");
        }
        assert!(block.contains("W 45 cm"), "got: {block}");
        assert!(block.contains("wanted"), "got: {block}");
    }

    #[test]
    fn the_seed_carries_the_prose_and_no_marker() {
        let mut it = slot("chair", "Second chair");
        it.reason = Some("A second chair is a coat stand with legs.".into());
        let text = seed(&it);
        assert!(text.contains("coat stand with legs"), "got: {text}");
        assert!(!text.contains("axon:begin"), "the seed renders no region");
        assert!(text.contains("axon_interior_id: \"chair\""), "got: {text}");
    }

    #[test]
    fn a_piece_gets_no_note() {
        let dir = std::env::temp_dir().join(format!("axon-interior-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut catalogue: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        let mut piece = slot("tisch_bestand", "Tisch vorhanden");
        piece.kind = Kind::Piece;
        catalogue.insert("tisch_bestand".into(), (piece, Some(State::Owned)));
        catalogue.insert(
            "vorhang".into(),
            (slot("vorhang", "Vorhang Terrassentuer"), Some(State::Wanted)),
        );

        let report = write_all(&root, &catalogue).unwrap();
        assert_eq!(report.seeded, vec!["Atlas/Interior/Vorhang Terrassentuer.md"]);
        assert!(!dir.join(DIR).join("Tisch vorhanden.md").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The vault is a git repository; a no-op write would be a commit that says nothing.
    #[test]
    fn a_second_run_without_change_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("axon-interior-2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut catalogue: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        catalogue.insert(
            "vorhang".into(),
            (slot("vorhang", "Vorhang Terrassentuer"), Some(State::Wanted)),
        );

        let first = write_all(&root, &catalogue).unwrap();
        assert_eq!(first.seeded.len(), 1);
        let second = write_all(&root, &catalogue).unwrap();
        assert!(second.seeded.is_empty(), "got: {:?}", second.seeded);
        assert!(second.written.is_empty(), "got: {:?}", second.written);
        assert_eq!(second.unchanged.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn human_prose_and_an_edited_region_survive() {
        let dir = std::env::temp_dir().join(format!("axon-interior-3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();

        let mut catalogue: BTreeMap<String, (Item, Option<State>)> = BTreeMap::new();
        catalogue.insert(
            "vorhang".into(),
            (slot("vorhang", "Vorhang Terrassentuer"), Some(State::Wanted)),
        );
        write_all(&root, &catalogue).unwrap();

        let path = dir.join(DIR).join("Vorhang Terrassentuer.md");
        let with_prose = std::fs::read_to_string(&path)
            .unwrap()
            .replace(
                "# Vorhang Terrassentuer",
                "# Vorhang Terrassentuer\n\nMy own sentence.",
            )
            .replace("wanted", "changed by hand");
        std::fs::write(&path, &with_prose).unwrap();

        let report = write_all(&root, &catalogue).unwrap();
        assert_eq!(report.conflicts, vec!["Atlas/Interior/Vorhang Terrassentuer.md"]);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, with_prose, "nothing was written");
        assert!(after.contains("My own sentence."));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

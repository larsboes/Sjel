//! Every saved feed item as one markdown note in `Resources/Sources/`.
//!
//! PRD Q49 (2026-08-27) ruled one generic bridge rather than N exporters, and named
//! the order: **feed library first**, into `Resources/Sources/`. Q31 (2026-08-23) had
//! already ruled the pattern and `libs/markdown-root/src/projection.rs` owns the
//! mechanism, so only the *shape* is here — what a saved link looks like as a note,
//! and which name it takes.
//!
//! ## Why this folder is different from every projection before it
//!
//! `Resources/Sjel/Trips/` and `Resources/Sjel/Subscriptions/` are machine rooms: a
//! human never edits there, so the only question a write has to answer is "is this
//! file mine". `Resources/Sources/` is a human-facing folder. Q49's own words are that
//! this bridge "finally executes the Sources consolidation" — the seven source homes
//! become one, `Clippings/` merges in, and `Atlas/Media`'s V3 survivors move across.
//! So a projection here lands beside notes a human wrote and will keep writing.
//!
//! Three guards follow from that, and none of them is optional:
//!
//! 1. **A file that is not comms' is never written and never deleted.** The mechanism's
//!    `NotOurs` covers a human's note; the owner in the header covers another
//!    capability's projection. Both are reported, not silently skipped.
//! 2. **A refused name is not worked around.** When a human note already holds the
//!    name, the item gets no projection at all rather than a near-miss file beside it.
//!    That is Q31's promotion, and a second note about one source is exactly what the
//!    consolidation is removing.
//! 3. **The sweep only ever considers `Resources/Sources/*.md` files carrying comms'
//!    header.** Everything else in the folder is somebody's writing.
//!
//! ## Which frontmatter keys exist, and why not more
//!
//! Vault rule V2: *a key must name its reader before it may be written.* The readers
//! that exist today, measured rather than assumed:
//!
//! - `Resources/Bases/Media.base` renders `summary`, `format`, `status`, `author`,
//!   `rating`, `started_at`, `finished_at`, `related`, `source`, `tags` and `file.name`.
//!   It is the Source note's Base — PRD §5.6 lists the Source template as serving
//!   "`Resources/Sources/` — replaces Media" — and its four table views all filter on
//!   `status`, which is why `status` is emitted and not omitted as unknowable.
//! - The ten `Clippings/` notes, the population that merges into this folder, carry
//!   `title`, `source`, `author`, `published`, `created`, `description`, `tags`. They
//!   are where `created` comes from: the vault already names the day a source arrived,
//!   so this does not invent `saved_at`.
//! - `Resources/Templates/Source Template.md` declares `type` and `url`.
//!
//! Everything the store knows that has no reader here stays in the store. `data_class`,
//! `content_status`, `stream`, `summary_attempts` and the provenance columns are read
//! by comms and by nothing in the vault, and a served field with no reader is a
//! contract nothing checks.
//!
//! `summary` is the one reader-having key deliberately left empty: the body carries the
//! summary verbatim, live summaries run 484–1,293 characters, and a paragraph of that
//! size in a YAML scalar buries the note under its own properties block.
//!
//! ## What this file is for
//!
//! Reading, unlike `trips`. A trip projection is a safety copy of rows that cannot be
//! re-queried; a saved link is not — the row is small, the page is still on the web,
//! and what the vault wants is the thing itself in the folder where sources live, so it
//! can be linked from a Knowledge note. That is also why the body is the summary and
//! not a JSON dump.
//!
//! ## What is deliberately not here
//!
//! No import. One way, like every other machine→vault write in the repo. And no second
//! export: `keeper_export_dir` (`src/main.rs`) is the ad-hoc exporter Q49 replaces — it
//! is unset in this host's overlay, so nothing depends on it, and it is left where it
//! is rather than removed in a commit about something else.

use markdown_root::{MarkdownRoot, ProjectionOutcome, RegionSpec, RootError};

use crate::store::FeedItem;

/// The projection marker owner. Stable forever: changing it makes every file already
/// in the vault look foreign, and `write_projection` then refuses all of them.
pub const OWNER: &str = "comms";

/// Bumped when the rendered shape changes, so a later generator can recognise output
/// it no longer knows how to produce.
pub const VERSION: u32 = 1;

/// Q49's destination for the feed library. Vault-relative and not configurable, for
/// the reason `trips::projection::DIR` gives: a second declaration of where machine
/// output goes is how two hosts write to two folders and neither notices.
pub const DIR: &str = "Resources/Sources";

/// The store's `status` value that means saved. The library is exactly this set —
/// `dismissed` is the explicit no, `new` is the unread queue.
pub const SAVED: &str = "keeper";

/// One saved item's file name and body, ready to write.
pub struct Projection {
    /// Vault-relative, `Resources/Sources/<name>.md`.
    pub path: String,
    pub body: String,
}

/// Render every saved item, and name the files that should exist afterwards.
///
/// Takes the whole set because the file name comes from the title and two items may
/// share one — the live library already holds four GitHub repositories whose titles
/// share a shape. A collision gives *both* files an id suffix, which is decidable only
/// with the set in hand and keeps the name a function of the data rather than of
/// iteration order.
pub fn render_all(items: &[FeedItem]) -> Vec<Projection> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in items {
        *counts.entry(file_stem(item)).or_default() += 1;
    }

    items
        .iter()
        .map(|item| {
            let stem = file_stem(item);
            let unique = if counts.get(&stem).copied().unwrap_or(0) > 1 {
                format!("{stem} ({})", id_suffix(&item.id))
            } else {
                stem
            };
            Projection {
                path: format!("{DIR}/{unique}.md"),
                body: render(item),
            }
        })
        .collect()
}

/// Write every saved item, and delete the projections of items no longer saved.
///
/// The sweep is what makes unsaving work: `POST /feed/:id/status` with `dismissed` or
/// `new` takes the row out of the library, and the note has to go with it or the vault
/// keeps a source the human removed. Only files carrying comms' own header are
/// considered, so a human note in the folder — and another capability's projection —
/// are left exactly as they are.
pub fn export_all(root: &MarkdownRoot, items: &[FeedItem]) -> Result<Report, RootError> {
    let spec = RegionSpec::new(OWNER, VERSION);
    let rendered = render_all(items);
    let mut report = Report::default();

    for projection in &rendered {
        match root.write_projection(&projection.path, &spec, &projection.body)? {
            ProjectionOutcome::Created => report.created += 1,
            ProjectionOutcome::Updated => report.updated += 1,
            ProjectionOutcome::Unchanged => report.unchanged += 1,
            ProjectionOutcome::NotOurs => report.refused.push(projection.path.clone()),
        }
    }

    // A missing folder is zero files, not an error: on this host `Resources/Sources/`
    // does not exist until the first run, and `write_projection` has just created it.
    let existing = match root.markdown_files(&format!("{DIR}/*.md")) {
        Ok(files) => files,
        Err(RootError::Unreadable { .. }) => Vec::new(),
        Err(e) => return Err(e),
    };
    let wanted: std::collections::HashSet<&str> =
        rendered.iter().map(|p| p.path.as_str()).collect();
    for file in existing {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        if wanted.contains(id.as_str()) {
            continue;
        }
        if root.remove_projection(&id, &spec)? {
            report.removed.push(id);
        }
    }

    Ok(report)
}

/// What one export run did. Counts for the ordinary outcomes, paths for the two a
/// human has to look at.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    /// Paths holding a file comms did not write — a human's note, or another
    /// capability's projection. Q31's promotion signal in the first case, and in both
    /// cases the item silently has no note until somebody looks.
    pub refused: Vec<String>,
    /// Notes whose item is no longer saved, or whose title changed.
    pub removed: Vec<String>,
}

/// One saved item as a whole document, frontmatter first.
pub fn render(item: &FeedItem) -> String {
    let title = title(item);
    let mut out = String::new();

    out.push_str("---\n");
    // The vault's own keys first, in the Source template's order, because this is the
    // half a human reads. `title` is absent on purpose: the vault's title is the file
    // name (`capabilities/vault/README.md`), and Media.base renders `file.name` as
    // Title.
    push_field(&mut out, "type", "source");
    if let Some(word) = format_word(&item.kind) {
        push_field(&mut out, "format", word);
    }
    // Saved and not yet dismissed is exactly what the vault calls a backlog. The
    // store has no vocabulary for "read", so nothing here can ever say `completed`;
    // the way off this list is to dismiss the item or to write your own note, which
    // the projection then refuses to overwrite.
    push_field(&mut out, "status", "backlog");
    if let Some(author) = item.author.as_deref().filter(|a| !a.trim().is_empty()) {
        push_field(&mut out, "author", author);
    }
    push_field(&mut out, "url", &item.url);
    // The day the item entered the feed. The saved date now EXISTS -- a keep
    // writes a `kept` row into `comms_feed_interactions` in the same transaction
    // as the status change -- but this note still renders the ingest day, because
    // what the frontmatter should say is a separate decision about the note and
    // not one this projection gets to take on its own. `created` rather than a
    // new `saved` key because the ten Clippings notes merging into this folder
    // already name this fact.
    if !item.day.trim().is_empty() {
        push_field(&mut out, "created", &item.day);
    }
    // The machine half. `axon_feed_id` is the row id, which is also the argument
    // `comms dismiss <id>` takes — so the note names the one command that removes it.
    push_field(&mut out, "axon_feed_id", &item.id);
    push_field(&mut out, "axon_projection_version", &VERSION.to_string());
    out.push_str("---\n\n");

    out.push_str(&format!("# {title}\n\n"));
    match item.summary.as_deref().map(str::trim) {
        Some(summary) if !summary.is_empty() => {
            // Verbatim. A summary is prose somebody (or some model) already wrote for
            // this item, and re-wrapping or truncating it here would make the note
            // disagree with the feed reader showing the same text.
            out.push_str(summary);
            out.push('\n');
        }
        // One of the eight live saved items has no summary. An empty note would look
        // like a failed write rather than a pending one.
        _ => out.push_str("_No summary stored yet._\n"),
    }

    out
}

/// A feed `kind` as the word this vault already uses for that format.
///
/// Every value is measured from `Atlas/Media`'s 103 notes rather than invented:
/// `article` (11 notes), `thread` (7), `paper` (6), `repo` (2), `video` (1), and
/// `podcast` from the Source template's own vocabulary. Emitting a word the folder
/// does not otherwise contain would put a value in the vault's vocabulary that no
/// Base groups by and no human recognises.
///
/// `mail` returns `None`: nothing in the vault names a kept mail as a format, and the
/// honest output for a fact with no vocabulary is no key. All four kinds present in
/// the live store — arxiv, github, article, youtube — map.
fn format_word(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "youtube" | "instagram" => "video",
        "podcast" => "podcast",
        "article" => "article",
        "github" | "huggingface" => "repo",
        "arxiv" => "paper",
        "reddit" => "thread",
        _ => return None,
    })
}

/// The item's title, or a stand-in that still says what the note is about.
///
/// A saved item with no title exists in the schema (`title TEXT`, nullable) though not
/// in the live library today, and a note headed "untitled" is worse than one headed by
/// its URL — the URL is the thing the human saved.
fn title(item: &FeedItem) -> &str {
    match item.title.as_deref().map(str::trim) {
        Some(title) if !title.is_empty() => title,
        _ => &item.url,
    }
}

/// The item's title as a file name. `markdown_root::projection::file_stem` owns the
/// rules; the fallback for an item whose title survives none of them is the tail of its
/// id, which is not pretty and is not ambiguous.
fn file_stem(item: &FeedItem) -> String {
    markdown_root::projection::file_stem(title(item), &id_suffix(&item.id))
}

/// The tail of a feed id. Ids are the sha256 of the canonical URL
/// (`Store::feed_id`), so any eight characters differ; the last eight are taken for
/// the same reason trips takes the last eight of a plan id.
fn id_suffix(id: &str) -> String {
    let tail: String = id.chars().rev().take(8).collect();
    tail.chars().rev().collect()
}

/// A frontmatter scalar, always quoted.
///
/// Quoted unconditionally rather than when it looks necessary, for the reason
/// `trips::projection` gives: live titles already carry `:` and `—`, and a rule that
/// decides per value will one day decide wrong on a value nobody anticipated. Live
/// library titles include `bannedbook/fanqiang — 翻墙-科学上网`.
fn push_field(out: &mut String, key: &str, value: &str) {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    out.push_str(&format!("{key}: \"{escaped}\"\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, title: Option<&str>, summary: Option<&str>) -> FeedItem {
        FeedItem {
            id: id.to_string(),
            stream: "media".into(),
            kind: kind.to_string(),
            title: title.map(str::to_string),
            url: format!("https://example.test/{id}"),
            author: Some("A Person".into()),
            summary: summary.map(str::to_string),
            transcript: None,
            day: "2026-08-08".into(),
            created_at: "2026-08-08 21:36:26.352+00:00".into(),
            status: SAVED.into(),
            content_status: "full".into(),
            transcript_source: "full-text".into(),
            summary_attempts: 0,
            summary_last_error: None,
            summary_next_attempt: None,
            captured_via: None,
            raw_content: None,
            summary_provenance: None,
            data_class: "c1".into(),
            data_class_rationale: "test".into(),
            data_classification_method: "legacy".into(),
            data_classification_version: "data-class-legacy-v1".into(),
        }
    }

    fn temp_root() -> (std::path::PathBuf, MarkdownRoot) {
        let dir = std::env::temp_dir().join(format!(
            "axon-comms-projection-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();
        (dir, root)
    }

    #[test]
    fn the_body_is_the_summary_verbatim() {
        let summary =
            "Richard Hipp on why SQLite tests the way it does.\n\nTwo paragraphs, kept as written.";
        let rendered = render(&item(
            "aaaa1111",
            "youtube",
            Some("SQLite lessons"),
            Some(summary),
        ));
        assert!(
            rendered.ends_with(&format!("{summary}\n")),
            "the summary must survive unrewrapped: {rendered}"
        );
        assert!(rendered.contains("# SQLite lessons\n"));
    }

    #[test]
    fn an_item_with_no_summary_says_so_rather_than_writing_an_empty_note() {
        let rendered = render(&item(
            "bbbb2222",
            "article",
            Some("Polarity Dispenser"),
            None,
        ));
        assert!(rendered.contains("_No summary stored yet._"), "{rendered}");
    }

    /// V2: every key here has a reader, and the store's other columns do not.
    #[test]
    fn the_frontmatter_is_the_keys_a_reader_exists_for() {
        let rendered = render(&item(
            "cccc3333",
            "github",
            Some("ladybird"),
            Some("A browser."),
        ));
        let fields = markdown_root::frontmatter(&rendered).unwrap();
        assert_eq!(fields.get("type").unwrap(), "source");
        assert_eq!(fields.get("format").unwrap(), "repo");
        assert_eq!(fields.get("status").unwrap(), "backlog");
        assert_eq!(fields.get("created").unwrap(), "2026-08-08");
        assert_eq!(fields.get("url").unwrap(), "https://example.test/cccc3333");
        assert_eq!(fields.get("axon_feed_id").unwrap(), "cccc3333");
        for unread in ["data_class", "stream", "content_status", "summary"] {
            assert!(
                !fields.contains_key(unread),
                "{unread} has no reader in the vault and must not be written"
            );
        }
    }

    #[test]
    fn a_kind_the_vault_has_no_word_for_gets_no_format_key() {
        let rendered = render(&item("dddd4444", "mail", Some("A kept mail"), Some("x")));
        assert!(!markdown_root::frontmatter(&rendered)
            .unwrap()
            .contains_key("format"));
    }

    #[test]
    fn two_items_with_one_title_both_get_an_id_suffix() {
        let both = vec![
            item("1111aaaabbbbcccc", "github", Some("skills"), None),
            item("2222ddddeeeeffff", "github", Some("skills"), None),
        ];
        let paths: Vec<String> = render_all(&both).into_iter().map(|p| p.path).collect();
        assert_eq!(
            paths,
            vec![
                "Resources/Sources/skills (bbbbcccc).md",
                "Resources/Sources/skills (eeeeffff).md",
            ],
            "both sides are renamed, so neither name depends on order"
        );
    }

    #[test]
    fn a_titleless_item_is_headed_by_the_url_it_saved() {
        let rendered = render(&item("eeee5555", "article", None, Some("x")));
        assert!(
            rendered.contains("# https://example.test/eeee5555"),
            "{rendered}"
        );
    }

    /// The whole contract against a real directory: saving creates, a second run
    /// writes nothing, and unsaving removes.
    #[test]
    fn an_export_creates_settles_and_removes_what_was_unsaved() {
        let (dir, root) = temp_root();
        let saved = vec![
            item("aaaa1111", "youtube", Some("SQLite lessons"), Some("Why.")),
            item("bbbb2222", "github", Some("ladybird"), Some("A browser.")),
        ];

        let first = export_all(&root, &saved).unwrap();
        assert_eq!((first.created, first.updated, first.unchanged), (2, 0, 0));
        let note = dir.join("Resources/Sources/ladybird.md");
        assert!(note.exists());
        assert!(std::fs::read_to_string(&note)
            .unwrap()
            .contains("A browser."));

        let second = export_all(&root, &saved).unwrap();
        assert_eq!(
            (second.created, second.updated, second.unchanged),
            (0, 0, 2),
            "an unchanged export must not touch a file, or every run is a vault commit"
        );

        // Unsaving one item: the store no longer returns it, so the note goes.
        let third = export_all(&root, &saved[..1]).unwrap();
        assert_eq!(third.removed, vec!["Resources/Sources/ladybird.md"]);
        assert!(!note.exists());
        assert!(dir.join("Resources/Sources/SQLite lessons.md").exists());

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The guard this folder needs and `Resources/Sjel/` never did.
    #[test]
    fn a_human_note_holding_the_name_is_refused_and_never_deleted() {
        let (dir, root) = temp_root();
        std::fs::create_dir_all(dir.join("Resources/Sources")).unwrap();
        let human = dir.join("Resources/Sources/ladybird.md");
        std::fs::write(&human, "---\ntype: source\n---\n\nWhy I care about it.\n").unwrap();

        let saved = vec![item(
            "bbbb2222",
            "github",
            Some("ladybird"),
            Some("A browser."),
        )];
        let report = export_all(&root, &saved).unwrap();
        assert_eq!(report.refused, vec!["Resources/Sources/ladybird.md"]);
        assert_eq!(report.created, 0);
        assert_eq!(
            std::fs::read_to_string(&human).unwrap(),
            "---\ntype: source\n---\n\nWhy I care about it.\n",
            "a human's note is not overwritten, and no near-miss file is written beside it"
        );

        // And unsaving the item does not delete the human's note either.
        let after = export_all(&root, &[]).unwrap();
        assert!(after.removed.is_empty());
        assert!(human.exists());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sweep_leaves_human_notes_and_other_owners_alone() {
        let (dir, root) = temp_root();
        export_all(
            &root,
            &[item(
                "aaaa1111",
                "youtube",
                Some("SQLite lessons"),
                Some("Why."),
            )],
        )
        .unwrap();
        let human = dir.join("Resources/Sources/Reading notes on SQLite.md");
        std::fs::write(&human, "# Mine\n").unwrap();
        root.write_projection(
            "Resources/Sources/A subscription.md",
            &RegionSpec::new("finance", 1),
            "# theirs\n",
        )
        .unwrap();

        let report = export_all(&root, &[]).unwrap();
        assert_eq!(
            report.removed,
            vec!["Resources/Sources/SQLite lessons.md"],
            "only comms' own notes are swept"
        );
        assert!(human.exists());
        assert!(dir.join("Resources/Sources/A subscription.md").exists());

        std::fs::remove_dir_all(dir).unwrap();
    }
}

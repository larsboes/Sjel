//! The Action kind, read out of the vault.
//!
//! The vault contract (`PRD Axon.md` §5.1b, "The no-doubling law") gives the
//! Action kind exactly one owner: `Projects/**/Tasks/`. Q48 (2026-08-27) put it
//! back there by retiring the `tasks` capability, so this module is the reader
//! that replaced a database table. It writes nothing — a task is marked done in
//! Obsidian, in a note a human owns.
//!
//! ## What counts as a task, and why not just "type: task"
//!
//! The vault already answers this question in `Resources/Bases/Tasks.base`,
//! whose filter is `hasTag("🔲") OR inFolder("Projects/Tasks") OR type ==
//! "task"`. That Base is the operator's own surface over the same notes, so
//! this reader tracks it rather than inventing a second definition of "task"
//! — two surfaces that disagree about the same folder is the failure the
//! no-doubling law exists to prevent.
//!
//! ## Where a task folder sits (Q104)
//!
//! The Base's middle clause selects nothing. `Projects/Tasks/` holds no note in
//! the vault, and **Q104 (2026-09-09) rules that it is never created: a task
//! lives under the project that owns it.** The folder half of the rule below is
//! therefore `Projects/<project>/**/Tasks/` — a `Tasks` folder always has a
//! project above it, so the segment is never the first one under `Projects/`.
//! A note filed directly in `Projects/Tasks/` is not in that shape and this
//! reader does not put it back into one.
//!
//! Every `Tasks` folder that follows the ruling sits under a project, so the
//! clause the Base still carries reaches none of them. That is what `vault
//! bases` reports and proposes a replacement for. Changing the `.base` itself
//! is the operator's move — §5.5 is one-way.
//!
//! The rule below is that filter with three stated divergences:
//!
//! 1. **Scoped to `Projects/`.** §5.1b puts the Action kind there and nowhere
//!    else. The template every task note is stamped from, for example
//!    `Resources/Templates/Task-Template.md`, falls outside it, and the Base
//!    excludes that template by name as well.
//! 2. **No `archive`/`Archive` segment.** A wound-down project's leftovers are
//!    not on today's decision list. This is also why the Base's `hasTag("🔲")`
//!    clause is not implemented: the tag is an older convention that survives
//!    only under archived folders such as
//!    `Projects/Archive/Old-Project/archive/Tasks/`, and no live note uses it.
//!    Such a folder can also hold a note that carries neither the tag nor a
//!    `type:` key, which `Tasks.base` selects by no clause at all.
//! 3. **A `done:` key is required.** A `Tasks/` folder can hold an index or a
//!    stub, and a row with no state cannot be ranked. A live task note written
//!    from the template always carries `done:`, so this excludes only notes
//!    that are not actions.
//!
//! ## Why a key is served only when the ladder reads it
//!
//! A task note carries eleven frontmatter keys; five are served here because
//! the dashboard's decision ladder consumes them: `summary` renders the row
//! (with `title`, which is the filename, not a key), `due` and `priority`
//! rank it, `projects` labels it, and `done` decides whether it is a decision
//! at all. The other six — `scheduled`, `context`, `energy`, `focus`,
//! `events` and `blocked_by` — have no reader on the ladder, so serving them
//! would publish a contract nothing checks, and an unread field is the one
//! that rots without anything failing.

use std::collections::HashMap;
use std::path::Path;

use content_item::DataClass;
use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Serialize;

use crate::class::CLASS_KEY;

/// The folder the Action kind lives under (vault contract §5.1b).
pub const PROJECTS: &str = "Projects";

/// The folder a project keeps its actions in. Matched as a whole path segment,
/// never as a prefix: `Tasks - to sort in` is a different folder from `Tasks`,
/// and a prefix match would claim both.
const TASKS: &str = "Tasks";

/// The template's default when a note leaves `priority:` blank, and the same
/// fallback `Tasks.base` applies in `priority || 2`.
const DEFAULT_PRIORITY: u8 = 2;

/// One action, as the ladder needs it.
///
/// No `status` field: the vault expresses completion as `done`, and adding a
/// second spelling of the same fact here is exactly the dialect drift
/// `vault lint` exists to report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    /// Vault-relative path, slash-separated. The identity that survives a
    /// machine, and the only handle the ladder needs to link back.
    pub id: String,
    /// The file name without `.md`. In Obsidian the file name *is* the title —
    /// an `# H1` inside the note repeats it where it exists at all (measured:
    /// identical on every note that has one), so the file name is the owner.
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub summary: Option<String>,
    /// Display names of the `projects:` wikilinks, path form stripped.
    pub projects: Vec<String>,
    /// Where the operator goes to act on it. Obsidian is the writer; this
    /// server is not.
    pub uri: String,
    /// What this task is worth protecting: `c0`, `c1`, `c2` or `c3`.
    ///
    /// Decided by `content_item::DataClass::classify_vault_note`, which is the
    /// same call `class.rs` makes for the whole-vault report. Not a second
    /// classifier and not a copy of its rules: a second place deciding what c2
    /// means is the one outcome §6.1 forbids, and `class.rs`'s own module doc
    /// says so about this exact function.
    ///
    /// Never absent. A note that declares nothing gets its folder's default,
    /// which is c1 — the fail-closed answer, never c0. Publishing is an act and
    /// no folder in the vault means "already public".
    pub data_class: String,
    /// Why that class, in the classifier's own words.
    ///
    /// Served where the feed list serves the value alone, because the vault
    /// rule has a branch the value cannot show: a note whose frontmatter
    /// declares a class OUTSIDE the vocabulary is refused, the folder default
    /// answers, and the rationale is the only place that says so. `class.rs`
    /// calls that the note "whose author believed they had set a class and had
    /// not — the failure this whole section exists to make visible rather than
    /// silent", and `/api/tasks` is the only vault surface the dashboard reads.
    pub data_class_rationale: String,
}

/// Resolve the folder the Action kind lives in.
///
/// A separate declared root rather than a filter over the whole vault: reading
/// every note to answer for 23 of them costs 220 ms against 6 ms for
/// `Projects/` alone (measured 2026-08-28, 2,102 notes / 14.6 MB against 334 /
/// 3.7 MB), and that is a per-request cost. Containment is unchanged — a nested
/// root is canonicalized and proven the same way.
pub fn projects_root(vault: &MarkdownRoot) -> Result<MarkdownRoot, String> {
    MarkdownRoot::declare(vault.path().join(PROJECTS))
        .map_err(|e| format!("{PROJECTS}/ under the vault root: {e}"))
}

/// The vault's name as Obsidian knows it: the root directory's own name. Taken
/// from the path rather than configured, because Obsidian derives it the same
/// way and a second declaration could only ever disagree.
pub fn vault_name(vault: &MarkdownRoot) -> String {
    vault
        .path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every action in the vault, newest state on disk, read now.
///
/// A note that cannot be read is skipped rather than failing the request: one
/// unreadable file must not blank the whole list, and iCloud can hold a note
/// evicted at the moment of the read.
pub fn read(projects: &MarkdownRoot, vault_name: &str) -> Result<Vec<Task>, String> {
    let files = projects
        .markdown_files_recursive()
        .map_err(|e| format!("walking {PROJECTS}/: {e}"))?;

    let mut tasks = Vec::new();
    for path in files {
        let Some(relative) = projects.relative_id(&path) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = frontmatter_spanned(&text) else {
            continue;
        };
        if !is_task(&relative, &parsed.fields) {
            continue;
        }
        let id = format!("{PROJECTS}/{relative}");
        // The vault-relative id, which is what the folder half of Q9a reads, and
        // the frontmatter key, which overrides it. Both are already in hand: the
        // classifier refuses a body parameter on purpose, so classifying costs
        // this walk nothing.
        let class =
            DataClass::classify_vault_note(&id, parsed.fields.get(CLASS_KEY).map(String::as_str));
        tasks.push(Task {
            title: title_of(&path),
            done: is_done(&parsed.fields),
            due: value(&parsed.fields, "due"),
            priority: priority_of(&parsed.fields),
            summary: value(&parsed.fields, "summary"),
            projects: projects_of(&parsed.fields),
            uri: obsidian_uri(vault_name, &id),
            data_class: class.value,
            data_class_rationale: class.rationale,
            id,
        });
    }
    // Open first, then by due date, then by priority — the order the ladder
    // reads them in, and the same one `Tasks.base`'s Standard view sorts by.
    tasks.sort_by(|a, b| {
        a.done
            .cmp(&b.done)
            .then_with(|| by_due(&a.due).cmp(&by_due(&b.due)))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(tasks)
}

/// Undated sorts after every dated task rather than before it: a deadline is
/// the thing that expires, and `None` sorting first would put "someday" above
/// "tomorrow".
fn by_due(due: &Option<String>) -> (bool, &str) {
    match due {
        Some(value) => (false, value.as_str()),
        None => (true, ""),
    }
}

/// The selection rule. See the module doc for what it tracks and where it
/// deliberately diverges.
fn is_task(relative: &str, fields: &HashMap<String, String>) -> bool {
    if !fields.contains_key("done") {
        return false;
    }
    let segments: Vec<&str> = relative.split('/').collect();
    let folders = &segments[..segments.len().saturating_sub(1)];
    if folders
        .iter()
        .any(|segment| segment.eq_ignore_ascii_case("archive"))
    {
        return false;
    }
    // Q104: a task folder belongs to a project, so `Tasks` is never the first
    // segment under `Projects/`. `skip(1)` is the whole of that rule — it
    // steps over the project's own folder, so `Projects/Tasks/` matches
    // nothing and `Projects/<project>/**/Tasks/` matches.
    folders.iter().skip(1).any(|segment| *segment == TASKS)
        || fields.get("type").map(String::as_str) == Some("task")
}

/// `done: true` and nothing else. An empty or missing value is an open task,
/// which is what the template ships (`done: false`) and what `Tasks.base`
/// assumes with `done != true`.
fn is_done(fields: &HashMap<String, String>) -> bool {
    fields.get("done").map(String::as_str) == Some("true")
}

fn title_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Present AND carrying something. A key left blank by the template is not a
/// value, and reporting `due: ""` as a deadline would put an undated task in
/// the overdue band.
fn value(fields: &HashMap<String, String>, key: &str) -> Option<String> {
    fields
        .get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn priority_of(fields: &HashMap<String, String>) -> u8 {
    value(fields, "priority")
        .and_then(|v| v.parse().ok())
        .filter(|p| (1..=3).contains(p))
        .unwrap_or(DEFAULT_PRIORITY)
}

/// `projects: - "[[Projects/Home-Lab/Home-Lab|Home-Lab]]"` becomes `Home-Lab`.
///
/// The display half of a path-form wikilink, because that is what the operator
/// wrote it to read as. `markdown-root` has already flattened the YAML list to
/// one comma-separated string.
fn projects_of(fields: &HashMap<String, String>) -> Vec<String> {
    value(fields, "projects")
        .unwrap_or_default()
        .split(',')
        .map(|item| {
            let inner = item.trim().trim_start_matches("[[").trim_end_matches("]]");
            inner.rsplit('|').next().unwrap_or(inner).trim().to_string()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// The address of one note in Obsidian, which is where it gets acted on.
///
/// `.md` is stripped: the `file` parameter takes a vault-relative path without
/// the extension, and passing it opens a "file not found" pane instead.
fn obsidian_uri(vault_name: &str, id: &str) -> String {
    let file = id.strip_suffix(".md").unwrap_or(id);
    format!(
        "obsidian://open?vault={}&file={}",
        percent_encode(vault_name),
        percent_encode(file)
    )
}

/// RFC 3986 unreserved set, everything else percent-encoded per UTF-8 byte.
///
/// Hand-written rather than a dependency: `markdown-root` is deliberately
/// dependency-free and this is the only encoding this crate does. The set that
/// matters here is real — task titles carry spaces, em dashes and umlauts
/// (`Verlustvortrag prüfen`, `Capture — Renewable Energy…`), and `/` must
/// survive so the path stays a path.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Reads a fixture vault on disk, because every rule above is a claim about
/// files and a rule tested against a hand-built `HashMap` would pass while the
/// walk, the containment check and the frontmatter parser disagreed with it.
#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("vault-tasks-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join(PROJECTS)).expect("a writable temp directory");
            Self { root }
        }

        fn note(&self, relative: &str, frontmatter: &str) -> &Self {
            let path = self.root.join(PROJECTS).join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).unwrap();
            std::fs::write(&path, format!("---\n{}\n---\n\nbody\n", frontmatter.trim())).unwrap();
            self
        }

        fn read(&self) -> Vec<Task> {
            let vault = MarkdownRoot::declare(&self.root).expect("the fixture root");
            let projects = projects_root(&vault).expect("a Projects folder");
            super::read(&projects, &vault_name(&vault)).expect("a readable fixture")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The three ways a note is claimed, and the three ways it is refused.
    /// Each row is a synthetic note with the shape of one the rule must decide.
    #[test]
    fn the_selection_rule_matches_the_vault_it_was_measured_against() {
        let fixture = Fixture::new("selection");
        fixture
            // Claimed: a `Tasks/` folder under a project.
            .note("Home-Lab/Tasks/Buy a drive.md", "type: task\ndone: false")
            // Claimed: a `Tasks/` folder under a project inside a project, on
            // the folder clause alone — a sub-project keeps its own `Tasks/`.
            .note("Garden/Greenhouse/Tasks/Fix the vent.md", "done: false")
            // Claimed: `type: task` outside any Tasks folder.
            .note("Garden/Loose action.md", "type: task\ndone: false")
            // Refused: a folder whose name only starts with `Tasks`. A segment
            // is matched whole, so this is a different folder.
            .note(
                "Home-Lab/Tasks - to sort in/Label the cables.md",
                "done: false\npriority: 3",
            )
            // Refused: a wound-down project's leftovers.
            .note(
                "Garden/archive/Old-Plan/Tasks/Retire the planner.md",
                "type: task\ndone: false",
            )
            // Refused: a project document that is not an action.
            .note("Home-Lab/Home-Lab.md", "type: project");

        let titles: Vec<String> = fixture.read().into_iter().map(|t| t.title).collect();
        assert_eq!(
            titles,
            vec!["Buy a drive", "Fix the vent", "Loose action"],
            "the selection rule drifted from the vault it was measured against"
        );
    }

    /// Q104: `Projects/Tasks/` is never created, so no folder rule may make it
    /// work. The planted note is exactly that shape and the reader refuses it;
    /// the same note one level down, under a project, is claimed.
    ///
    /// The pair is the point. A rule that answered the same for both would be
    /// measuring "is there a `Tasks` segment anywhere", which is the rule Q104
    /// replaced.
    #[test]
    fn a_tasks_folder_with_no_project_above_it_is_refused() {
        let fixture = Fixture::new("q104");
        fixture
            .note("Tasks/Orphan.md", "done: false")
            .note("Home-Lab/Tasks/Owned.md", "done: false");

        let titles: Vec<String> = fixture.read().into_iter().map(|t| t.title).collect();
        assert_eq!(
            titles,
            vec!["Owned"],
            "the folder rule reached `Projects/Tasks/`, which Q104 rules is never created"
        );
    }

    /// The other half of that gate: the folder is what Q104 constrains, not
    /// the note. A note that declares `type: task` is served wherever it sits,
    /// so the ruling never silently drops an action — it only refuses to read
    /// a folder name as a declaration.
    #[test]
    fn a_note_that_declares_itself_a_task_is_served_wherever_it_sits() {
        let fixture = Fixture::new("q104-declared");
        fixture.note("Tasks/Orphan.md", "type: task\ndone: false");

        let tasks = fixture.read();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "Projects/Tasks/Orphan.md");
    }

    /// Open before decided, then by deadline. An undated task must not outrank
    /// one that is due tomorrow, which is what a naive `Option` ordering does.
    #[test]
    fn open_tasks_sort_first_and_undated_ones_sort_last() {
        let fixture = Fixture::new("ordering");
        fixture
            .note("P/Tasks/Undated.md", "type: task\ndone: false")
            .note(
                "P/Tasks/Later.md",
                "type: task\ndone: false\ndue: 2026-12-01",
            )
            .note(
                "P/Tasks/Sooner.md",
                "type: task\ndone: false\ndue: 2026-08-30",
            )
            .note(
                "P/Tasks/Finished.md",
                "type: task\ndone: true\ndue: 2026-08-01",
            );

        let titles: Vec<String> = fixture.read().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, vec!["Sooner", "Later", "Undated", "Finished"]);
    }

    /// The template ships every key blank. An empty `due:` is not a deadline
    /// and an empty `priority:` is not a priority, so both fall back rather
    /// than becoming `Some("")` and `0`.
    #[test]
    fn a_blank_template_key_is_absent_rather_than_empty() {
        let fixture = Fixture::new("blanks");
        fixture.note(
            "P/Tasks/Fresh.md",
            "type: task\nsummary: \"\"\ndone: false\ndue:\nscheduled:\npriority:\nprojects: []",
        );

        let task = &fixture.read()[0];
        assert_eq!(task.due, None);
        assert_eq!(task.summary, None);
        assert_eq!(task.priority, DEFAULT_PRIORITY);
        assert!(task.projects.is_empty());
    }

    #[test]
    fn a_project_link_reads_as_its_display_name() {
        let fixture = Fixture::new("projects");
        fixture.note(
            "P/Tasks/Linked.md",
            "type: task\ndone: false\nprojects:\n  - \"[[Projects/Home-Lab/Home-Lab|Home-Lab]]\"\n  - \"[[Soma]]\"",
        );

        assert_eq!(fixture.read()[0].projects, vec!["Home-Lab", "Soma"]);
    }

    /// The link back is the whole of what the ladder can still do with a task,
    /// so an unencoded space or umlaut is a dead button rather than a cosmetic
    /// problem. `/` stays literal or the path stops being a path.
    #[test]
    fn the_obsidian_uri_encodes_what_a_real_title_contains() {
        assert_eq!(
            obsidian_uri(
                "Knowledge-Base",
                "Projects/Garden/Tasks/Bewässerung prüfen.md"
            ),
            "obsidian://open?vault=Knowledge-Base&file=Projects/Garden/Tasks/Bew%C3%A4sserung%20pr%C3%BCfen"
        );
    }

    /// Every branch of Q9a, reached through the walk rather than through a
    /// hand-built map — the folder default, the folder rule that overrides it,
    /// the frontmatter override, and the declaration that is refused.
    ///
    /// The last one is the reason the rationale is served at all. A note
    /// declaring `class: c22` must not be honoured (the literal is not a class)
    /// and must not be escalated (a typo is not evidence that a note is more
    /// sensitive than its folder), so it lands on the folder default with the
    /// same value as a note that declared nothing — and the rationale is the
    /// only thing that tells those two apart.
    #[test]
    fn a_task_carries_the_class_its_note_earns_and_a_typo_earns_nothing() {
        let fixture = Fixture::new("class");
        fixture
            .note("Home-Lab/Tasks/Plain.md", "type: task\ndone: false")
            .note(
                "Gesundheit/Tasks/Appointment.md",
                "type: task\ndone: false\ndue: 2026-09-20",
            )
            .note(
                "Home-Lab/Tasks/Declared.md",
                "type: task\ndone: false\nclass: c2",
            )
            .note(
                "Home-Lab/Tasks/Typo.md",
                "type: task\ndone: false\nclass: c22",
            );

        let by_title: HashMap<String, Task> = fixture
            .read()
            .into_iter()
            .map(|task| (task.title.clone(), task))
            .collect();

        // The folder default: Mine, never Public. Publishing is an act, and no
        // folder in the vault means "already public".
        assert_eq!(by_title["Plain"].data_class, "c1");
        // A health folder, wherever it sits (Q9a's own rule, not a Projects one).
        assert_eq!(by_title["Appointment"].data_class, "c2");
        // The operator's own word, honoured in either direction.
        assert_eq!(by_title["Declared"].data_class, "c2");
        assert!(
            by_title["Declared"]
                .data_class_rationale
                .contains("frontmatter"),
            "an override must say it was one: {}",
            by_title["Declared"].data_class_rationale
        );

        // The planted input. Same value as `Plain`, and the only difference a
        // reader can see is the sentence.
        assert_eq!(by_title["Typo"].data_class, "c1");
        assert!(
            by_title["Typo"].data_class_rationale.contains("c22"),
            "a refused declaration must name the literal it refused: {}",
            by_title["Typo"].data_class_rationale
        );
        assert_ne!(
            by_title["Typo"].data_class_rationale, by_title["Plain"].data_class_rationale,
            "a note whose author believed they set a class reads identically to one that \
             never tried"
        );
    }

    /// A vault whose `Projects/` folder is missing is a misconfigured root, not
    /// an empty task list. Reporting zero tasks would read as "nothing to do".
    #[test]
    fn a_vault_without_a_projects_folder_is_an_error() {
        let root =
            std::env::temp_dir().join(format!("vault-tasks-{}-noprojects", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Knowledge")).unwrap();
        let vault = MarkdownRoot::declare(&root).unwrap();
        assert!(projects_root(&vault).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

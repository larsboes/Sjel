//! Every operation the CLI and the API share, so both write through one path (ISA.md ISC-71).

use std::{fs, path::Path};

use content_item::DataClass;
use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Serialize;

use crate::{
    record::{self, check_name, Kind, Record, ARCHIVE},
    store::Projects,
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The frontmatter key a frozen vault note carries. The vault reader skips such notes.
pub const MOVED_TO: &str = "moved_to";
/// The vault folder whose law stays in the vault: only its typed notes are imported.
const LAW_FOLDER: &str = "Sjel";
const CLASS_KEY: &str = "class";

/// One task, in the shape `capabilities/vault` serves, so decision-ladder band 620 reads both.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub summary: Option<String>,
    pub projects: Vec<String>,
    /// The dashboard page that shows the task.
    pub uri: String,
    pub data_class: String,
    pub data_class_rationale: String,
}

impl Task {
    pub fn from_record(record: &Record) -> Self {
        let fields = record.fields();
        // Classified as the vault note it came from, so an import does not change its class.
        let class = DataClass::classify_vault_note(
            &format!("Projects/{}", record.path),
            fields.get(CLASS_KEY).map(String::as_str),
        );
        Self {
            id: record.path.clone(),
            title: record.title.clone(),
            done: record.done,
            due: record.due.clone(),
            priority: record.priority,
            summary: record::value(&fields, "summary"),
            projects: project_links(&fields).unwrap_or_else(|| vec![record.project.clone()]),
            uri: format!("/projects?project={}", encode(&record.project)),
            data_class: class.value,
            data_class_rationale: class.rationale,
        }
    }
}

/// The display names of a note's `projects:` wikilinks, as the vault reader labels them.
/// `markdown-root` flattens the YAML list to one comma-separated string.
fn project_links(fields: &std::collections::HashMap<String, String>) -> Option<Vec<String>> {
    let names: Vec<String> = record::value(fields, "projects")?
        .split(',')
        .map(|item| {
            let inner = item
                .trim()
                .trim_matches('"')
                .trim_start_matches("[[")
                .trim_end_matches("]]");
            inner.rsplit('|').next().unwrap_or(inner).trim().to_string()
        })
        .filter(|name| !name.is_empty() && name != "[]")
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Open tasks of live projects, ordered the way the vault reader orders them.
pub fn tasks(projects: &Projects, status: Option<&str>) -> Fallible<Vec<Task>> {
    let mut tasks: Vec<Task> = projects
        .by_kind(Kind::Task)?
        .iter()
        .filter(|r| !r.archived)
        .filter(|r| match status {
            Some("open") => !r.done,
            Some("done") => r.done,
            _ => true,
        })
        .map(Task::from_record)
        .collect();
    tasks.sort_by(|a, b| {
        a.done
            .cmp(&b.done)
            .then_with(|| (a.due.is_none(), &a.due).cmp(&(b.due.is_none(), &b.due)))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(tasks)
}

/// Create a task note in a live project.
///
/// # Errors
/// The project does not exist or is archived, a field is invalid, or the task exists.
pub fn add_task(
    projects: &Projects,
    project: &str,
    title: &str,
    summary: &str,
    due: Option<&str>,
    priority: u8,
) -> Fallible<Record> {
    let project = check_name(project)?;
    let title = check_name(title)?;
    if !(1..=3).contains(&priority) {
        return Err("priority is 1, 2 or 3".into());
    }
    if let Some(due) = due {
        chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d")
            .map_err(|_| format!("{due:?}: due is a date, YYYY-MM-DD"))?;
    }
    if !projects.by_project(project)?.iter().any(|r| !r.archived) {
        return Err(format!("no live project named {project:?}").into());
    }
    let path = format!("{project}/Tasks/{title}.md");
    if projects.get(&path)?.is_some() {
        return Err(format!("{path} exists").into());
    }
    let record = Record::from_content(&path, record::new_task(project, summary, due, priority))?;
    projects.put(&record)?;
    Ok(record)
}

/// Create a project hub.
///
/// # Errors
/// The name is invalid or a project of that name exists.
pub fn add_project(projects: &Projects, name: &str, summary: &str) -> Fallible<Record> {
    let name = check_name(name)?;
    if name == ARCHIVE || !projects.by_project(name)?.is_empty() {
        return Err(format!("a project named {name:?} exists").into());
    }
    let record = Record::from_content(
        &format!("{name}/{name}.md"),
        record::new_project(name, summary),
    )?;
    projects.put(&record)?;
    Ok(record)
}

/// Mark a task done or open again.
///
/// # Errors
/// No task has that id.
pub fn set_done(projects: &Projects, id: &str, done: bool) -> Fallible<Record> {
    let current = projects
        .get(id)?
        .filter(|r| r.kind == Kind::Task)
        .ok_or_else(|| format!("no task {id:?}"))?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let content = record::set_done(&current.content, done, &today)?;
    let record = Record::from_content(id, content)?;
    projects.put(&record)?;
    Ok(record)
}

/// The largest record `save` accepts. The biggest imported note was far below it (measured
/// 2026-10-18 on the 668 imported records).
pub const MAX_CONTENT_BYTES: usize = 512 * 1024;

/// Why `save` wrote nothing.
#[derive(Debug)]
pub enum SaveError {
    /// The record changed after the caller read it. Carries the current content.
    Stale(String),
    Refused(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Stale(_) => {
                write!(f, "the note changed since it was read; nothing was saved")
            }
            SaveError::Refused(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Replace a record's whole content, or create a note when `base` is `None`.
///
/// `base` is the content the caller started from. A save whose base no longer matches the store
/// writes nothing, so two editors never overwrite each other silently.
///
/// # Errors
/// `Stale` when the base is out of date; `Refused` for a bad path, an oversized body, a new note
/// outside a live project, or an existing path with no base.
pub fn save(
    projects: &Projects,
    path: &str,
    content: &str,
    base: Option<&str>,
) -> Fallible<Record> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(
            SaveError::Refused(format!("a note is at most {MAX_CONTENT_BYTES} bytes")).into(),
        );
    }
    let record = Record::from_content(path, content.to_string()).map_err(SaveError::Refused)?;
    match (projects.get(path)?, base) {
        (Some(current), Some(base)) if current.content == base => {}
        (Some(current), Some(_)) => return Err(SaveError::Stale(current.content).into()),
        (Some(_), None) => return Err(SaveError::Refused(format!("{path} exists")).into()),
        (None, Some(_)) => return Err(SaveError::Refused(format!("no record {path:?}")).into()),
        (None, None) => {
            if !projects
                .by_project(&record.project)?
                .iter()
                .any(|r| !r.archived)
            {
                let project = &record.project;
                return Err(
                    SaveError::Refused(format!("no live project named {project:?}")).into(),
                );
            }
        }
    }
    projects.put(&record)?;
    Ok(record)
}

/// The content of a new note, typed at birth like every vault note.
pub fn new_note(project: &str, title: &str, summary: &str) -> Result<(String, String), String> {
    let project = check_name(project)?;
    let title = check_name(title)?;
    let summary = summary.replace('"', "'");
    Ok((
        format!("{project}/{title}.md"),
        format!("---\ntype: note\nsummary: \"{summary}\"\nprojects:\n  - \"[[{project}]]\"\n---\n\n# {title}\n"),
    ))
}

/// What one import run did, note by note.
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub unchanged: Vec<String>,
    /// Not imported, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// Copy the vault's project notes into the store (ISA.md ISC-72).
///
/// Every `.md` under a project folder moves; `Archive/` moves as archived; from the law folder
/// only notes typed `project` or `task` move. Binaries stay in the vault. A row that changed in
/// Sjel since its import is never overwritten.
pub fn import(
    projects: &Projects,
    vault_projects: &Path,
    only: Option<&str>,
) -> Fallible<ImportReport> {
    let root = MarkdownRoot::declare(vault_projects)?;
    let mut report = ImportReport::default();
    for file in root.markdown_files_recursive()? {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(e) => {
                report.skipped.push((id, format!("unreadable: {e}")));
                continue;
            }
        };
        let record = match Record::from_content(&id, content) {
            Ok(record) => record,
            Err(reason) => {
                report.skipped.push((id, reason));
                continue;
            }
        };
        if only.is_some_and(|name| name != record.project) {
            continue;
        }
        if id.starts_with(&format!("{LAW_FOLDER}/")) && record.kind == Kind::Note {
            continue;
        }
        if record.fields().contains_key(MOVED_TO) {
            report.skipped.push((id, "frozen: imported before".into()));
            continue;
        }
        match projects.get(&id)? {
            Some(existing) if existing.content == record.content => report.unchanged.push(id),
            Some(_) => report.skipped.push((
                id,
                "changed in Sjel since its import; not overwritten".into(),
            )),
            None => {
                projects.put(&record)?;
                report.imported.push(id);
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Default, Serialize)]
pub struct FreezeReport {
    pub frozen: Vec<String>,
    pub already: Vec<String>,
    /// Left as they are, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// Mark each imported vault note as moved: a `moved_to` key and a banner (ISA.md ISC-73).
///
/// Only a note whose bytes still equal its imported row is touched, so an edit made in the
/// vault after the import is reported, never frozen over.
pub fn freeze_vault(
    projects: &Projects,
    vault_projects: &Path,
    only: Option<&str>,
) -> Fallible<FreezeReport> {
    let mut report = FreezeReport::default();
    for record in projects.all()? {
        if only.is_some_and(|name| name != record.project) {
            continue;
        }
        let file = vault_projects.join(&record.path);
        let Ok(current) = fs::read_to_string(&file) else {
            report.skipped.push((record.path, "no vault note".into()));
            continue;
        };
        if frontmatter_spanned(&current)
            .map(|f| f.fields.contains_key(MOVED_TO))
            .unwrap_or(false)
        {
            report.already.push(record.path);
            continue;
        }
        if current != record.content {
            report
                .skipped
                .push((record.path, "vault note changed after the import".into()));
            continue;
        }
        fs::write(&file, frozen(&current, &record.path)?)?;
        report.frozen.push(record.path);
    }
    Ok(report)
}

fn frozen(content: &str, path: &str) -> Fallible<String> {
    let key = format!("{MOVED_TO}: \"sjel projects/{path}\"\n");
    let banner = format!(
        "> [!warning] Moved to Sjel\n> Sjel `projects` owns this note now, at `{path}`. This copy is frozen: change it in Sjel.\n\n"
    );
    let parsed = frontmatter_spanned(content)?;
    Ok(match parsed.block {
        Some((_, end)) => format!(
            "{}{key}{}{banner}{}",
            &content[..end],
            &content[end..parsed.body_start],
            &content[parsed.body_start..]
        ),
        None => format!("---\n{key}---\n\n{banner}{content}"),
    })
}

/// RFC 3986 unreserved characters pass, everything else is percent-encoded per byte.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::scratch;

    fn vault(dir: &Path) -> std::path::PathBuf {
        let v = dir.join("vault");
        for (path, content) in [
            (
                "Waschmaschine/Waschmaschine.md",
                "---\ntype: project\n---\n# W\n",
            ),
            (
                "Waschmaschine/Tasks/Schlauch.md",
                "---\ntype: task\ndone: false\npriority: 1\n---\n\nx\n",
            ),
            (
                "Waschmaschine/Waschmaschine Links.md",
                "---\ntype: note\n---\nlinks\n",
            ),
            (
                "Archive/Old/Tasks/Done.md",
                "---\ntype: task\ndone: true\n---\n",
            ),
            ("Sjel/Sjel.md", "---\ntype: project\n---\n"),
            (
                "Sjel/Knowledge-Base/Vault-Contract.md",
                "---\ntype: note\n---\nlaw\n",
            ),
            ("Projects.md", "---\ntype: note\n---\n"),
        ] {
            let file = v.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        v
    }

    #[test]
    fn import_takes_project_notes_and_leaves_the_law() {
        let (projects, dir) = scratch("import");
        let v = vault(&dir);
        let report = import(&projects, &v, None).unwrap();
        assert_eq!(report.imported.len(), 5, "{report:?}");
        assert!(!report.imported.iter().any(|p| p.contains("Vault-Contract")));
        assert_eq!(report.skipped.len(), 1, "Projects.md has no project folder");
        assert_eq!(import(&projects, &v, None).unwrap().unchanged.len(), 5);
        let open = tasks(&projects, Some("open")).unwrap();
        assert_eq!(open.len(), 1, "the archived task is not served");
        assert_eq!(open[0].uri, "/projects?project=Waschmaschine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn freeze_marks_unchanged_notes_and_import_then_skips_them() {
        let (projects, dir) = scratch("freeze");
        let v = vault(&dir);
        import(&projects, &v, Some("Waschmaschine")).unwrap();
        fs::write(
            v.join("Waschmaschine/Waschmaschine Links.md"),
            "edited in vault",
        )
        .unwrap();
        let report = freeze_vault(&projects, &v, None).unwrap();
        assert_eq!(report.frozen.len(), 2);
        assert_eq!(report.skipped.len(), 1);
        let note = fs::read_to_string(v.join("Waschmaschine/Tasks/Schlauch.md")).unwrap();
        assert!(note.starts_with("---\ntype: task\ndone: false\npriority: 1\nmoved_to: \"sjel projects/Waschmaschine/Tasks/Schlauch.md\"\n---\n> [!warning] Moved to Sjel"));
        assert!(note.ends_with("\nx\n"));
        assert_eq!(freeze_vault(&projects, &v, None).unwrap().already.len(), 2);
        let again = import(&projects, &v, Some("Waschmaschine")).unwrap();
        assert!(again.imported.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_never_overwrites_a_change_made_in_sjel() {
        let (projects, dir) = scratch("conflict");
        let v = vault(&dir);
        import(&projects, &v, Some("Waschmaschine")).unwrap();
        set_done(&projects, "Waschmaschine/Tasks/Schlauch.md", true).unwrap();
        let report = import(&projects, &v, Some("Waschmaschine")).unwrap();
        let conflicts: Vec<_> = report
            .skipped
            .iter()
            .filter(|(_, why)| why.contains("changed in Sjel"))
            .collect();
        assert_eq!(conflicts.len(), 1, "{report:?}");
        assert!(
            projects
                .get("Waschmaschine/Tasks/Schlauch.md")
                .unwrap()
                .unwrap()
                .done
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_task_needs_a_live_project_and_a_valid_date() {
        let (projects, dir) = scratch("add");
        assert!(add_task(&projects, "Nope", "x", "", None, 2).is_err());
        add_project(&projects, "Waschmaschine", "W").unwrap();
        assert!(add_project(&projects, "Waschmaschine", "W").is_err());
        assert!(add_task(&projects, "Waschmaschine", "x", "", Some("24.10."), 2).is_err());
        let r = add_task(
            &projects,
            "Waschmaschine",
            "Schelle kaufen",
            "",
            Some("2026-10-20"),
            1,
        )
        .unwrap();
        assert_eq!(r.path, "Waschmaschine/Tasks/Schelle kaufen.md");
        assert!(add_task(&projects, "Waschmaschine", "Schelle kaufen", "", None, 1).is_err());
        assert_eq!(tasks(&projects, Some("open")).unwrap().len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_refuses_a_stale_base_and_creates_notes_only_in_live_projects() {
        let (projects, dir) = scratch("save");
        add_project(&projects, "W", "").unwrap();
        let hub = projects.get("W/W.md").unwrap().unwrap();
        let edited = format!("{}\nmore\n", hub.content);
        save(&projects, "W/W.md", &edited, Some(&hub.content)).unwrap();
        assert_eq!(
            fs::read_to_string(projects.root().join("W/W.md")).unwrap(),
            edited
        );
        let stale = save(&projects, "W/W.md", "x", Some(&hub.content)).unwrap_err();
        assert!(
            matches!(stale.downcast_ref::<SaveError>(), Some(SaveError::Stale(c)) if *c == edited)
        );
        assert!(
            save(&projects, "W/W.md", "x", None).is_err(),
            "an existing path needs a base"
        );
        let (path, content) = new_note("W", "Links", "parts").unwrap();
        assert_eq!(
            save(&projects, &path, &content, None).unwrap().kind,
            Kind::Note
        );
        let (elsewhere, content) = new_note("Nope", "Links", "").unwrap();
        assert!(save(&projects, &elsewhere, &content, None).is_err());
        assert!(save(
            &projects,
            "W/big.md",
            &"x".repeat(MAX_CONTENT_BYTES + 1),
            None
        )
        .is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

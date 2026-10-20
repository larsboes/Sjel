//! Every operation the CLI and the API share, so both write through one path.

use std::{fs, path::Path};

use content_item::DataClass;
use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Serialize;

use crate::{
    record::{self, check_name, Kind, Record, ARCHIVE},
    store::{Attached, Projects},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The frontmatter key a frozen vault note carries. The vault reader skips such notes.
pub const MOVED_TO: &str = "moved_to";
/// The vault folder whose law stays in the vault: nothing in it is imported, its typed hub
/// included. The rest of `Sjel/` is project material and imports like any project (2026-10-18).
const LAW_FOLDER: &str = "Sjel/Knowledge-Base";
const CLASS_KEY: &str = "class";

/// One task, as decision-ladder band 620 and the dashboard tables read it. The core keys keep the
/// shape the retired vault task reader served.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub summary: Option<String>,
    pub projects: Vec<String>,
    /// The project folder the task lives in.
    pub project: String,
    /// Every frontmatter key, lists flattened to `a, b`: the columns of the tasks table.
    pub fields: std::collections::BTreeMap<String, String>,
    /// The dashboard page that shows the task.
    pub uri: String,
    pub data_class: String,
    pub data_class_rationale: String,
}

/// Classify a project record with the same rule as the vault note it came from.
pub fn classify_record(record: &Record) -> DataClass {
    let fields = record.fields();
    DataClass::classify_vault_note(
        &format!("Projects/{}", record.path),
        fields.get(CLASS_KEY).map(String::as_str),
    )
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
            kind: record.kind,
            title: record.title.clone(),
            done: record.done,
            due: record.due.clone(),
            priority: record.priority,
            summary: record::value(&fields, "summary"),
            projects: project_links(&fields).unwrap_or_else(|| vec![record.project.clone()]),
            project: record.project.clone(),
            uri: format!("/projects?project={}", encode(&record.project)),
            data_class: class.value,
            data_class_rationale: class.rationale,
            fields: fields.into_iter().collect(),
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
        .filter(|r| !r.archived && !r.domain)
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

/// Every record of one kind in live projects, in the task shape, ordered by path.
pub fn records(projects: &Projects, kind: Kind) -> Fallible<Vec<Task>> {
    Ok(projects
        .by_kind(kind)?
        .iter()
        .filter(|r| !r.archived && !r.domain)
        .map(Task::from_record)
        .collect())
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
    if record::is_domain(project) {
        return Err(format!("{project:?} is a domain, not a project").into());
    }
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
    if record::is_domain(name) {
        return Err(format!("{name:?} is a domain, not a project").into());
    }
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

/// The states a task can be in. `done:` follows `status:`, never the other way round.
pub const TASK_STATUSES: [&str; 4] = ["open", "doing", "waiting", "done"];
/// The states a project can be in, as the vault's `Projects.base` reads them. An archived project
/// is a folder move, not a status.
pub const PROJECT_STATUSES: [&str; 3] = ["active", "done", "cancelled"];
/// Keys a property edit cannot set: `type` decides the kind, `done` follows `status`, and
/// `moved_to` marks a frozen vault note.
const DERIVED_KEYS: [&str; 3] = ["type", "done", MOVED_TO];

/// Set one frontmatter property of a record, or clear it with an empty value.
///
/// Any key is accepted, as in Obsidian Bases. Where the schema defines a key, the value is
/// checked: `priority` is 1 to 3, `due` and `scheduled` are dates, and a task's `status` is one of
/// `TASK_STATUSES` and sets `done:` and `completed:` with it.
///
/// # Errors
/// No such record, a derived key, a value the schema refuses, or an archived record.
pub fn set_property(projects: &Projects, path: &str, key: &str, value: &str) -> Fallible<Record> {
    let current = projects
        .get(path)?
        .ok_or_else(|| format!("no record {path:?}"))?;
    if current.archived {
        return Err(format!("{path} is archived").into());
    }
    if DERIVED_KEYS.contains(&key) {
        return Err(format!("{key:?} is derived; set status instead").into());
    }
    let value = value.trim();
    match key {
        "priority" if !["1", "2", "3"].contains(&value) => {
            return Err("priority is 1, 2 or 3".into())
        }
        "due" | "scheduled"
            if !value.is_empty()
                && chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() =>
        {
            return Err(format!("{value:?}: {key} is a date, YYYY-MM-DD").into())
        }
        "status" if current.kind == Kind::Task && !TASK_STATUSES.contains(&value) => {
            return Err(format!("status is one of {}", TASK_STATUSES.join(", ")).into())
        }
        "status" if current.kind == Kind::Project && !PROJECT_STATUSES.contains(&value) => {
            return Err(format!("status is one of {}", PROJECT_STATUSES.join(", ")).into())
        }
        _ => {}
    }
    let content = if key == "status" && current.kind == Kind::Task {
        // `set_done` only when done-ness flips, so a move between open states keeps `completed:`.
        let flipped = if current.done == (value == "done") {
            current.content.clone()
        } else {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            record::set_done(&current.content, value == "done", &today)?
        };
        record::set_field(&flipped, "status", Some(value))?
    } else {
        record::set_field(&current.content, key, (!value.is_empty()).then_some(value))?
    };
    let record = Record::from_content(path, content)?;
    projects.put(&record)?;
    Ok(record)
}

/// One task whose properties `normalize` changes, with each change as `key: old -> new`.
#[derive(Debug, Serialize, PartialEq)]
pub struct Normalized {
    pub path: String,
    pub changes: Vec<String>,
}

/// Bring every task onto the schema `set_property` enforces.
///
/// `done:` decides the status, since it is what the store, the vault reader and band 620 read:
/// a done task gets `status: done`, an open one keeps `doing` or `waiting` and otherwise gets
/// `open`. `done_date:` becomes `completed:`. A priority outside 1 to 3 moves to the nearest end.
/// Without `apply` nothing is written.
pub fn normalize(projects: &Projects, apply: bool) -> Fallible<Vec<Normalized>> {
    let mut report = Vec::new();
    for current in projects.by_kind(Kind::Task)? {
        let fields = current.fields();
        let mut content = current.content.clone();
        let mut changes = Vec::new();
        let old = |key: &str| record::value(&fields, key).unwrap_or_default();

        let status = old("status");
        let target = match (current.done, status.as_str()) {
            (true, _) => "done",
            (false, "doing" | "waiting") => status.as_str(),
            (false, _) => "open",
        };
        if status != target {
            content = record::set_field(&content, "status", Some(target))?;
            changes.push(format!("status: {status} -> {target}"));
        }
        if fields.contains_key("done_date") {
            let date = old("done_date");
            content = record::set_field(&content, "done_date", None)?;
            if old("completed").is_empty() && !date.is_empty() {
                content = record::set_field(&content, "completed", Some(&date))?;
            }
            changes.push(format!("done_date: {date} -> completed"));
        }
        if let Ok(p) = old("priority").parse::<i64>() {
            let clamped = p.clamp(1, 3);
            if clamped != p {
                content = record::set_field(&content, "priority", Some(&clamped.to_string()))?;
                changes.push(format!("priority: {p} -> {clamped}"));
            }
        }
        if changes.is_empty() {
            continue;
        }
        if apply {
            projects.put(&Record::from_content(&current.path, content)?)?;
        }
        report.push(Normalized {
            path: current.path,
            changes,
        });
    }
    Ok(report)
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
            if !record.domain
                && !projects
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
    if record::is_domain(project) {
        return Err(format!("{project:?} is a domain, not a project"));
    }
    let summary = summary.replace('"', "'");
    Ok((
        format!("{project}/{title}.md"),
        format!("---\ntype: note\nsummary: \"{summary}\"\nprojects:\n  - \"[[{project}]]\"\n---\n\n# {title}\n"),
    ))
}

/// The largest attachment `attach` accepts over CLI or HTTP (32 MiB).
pub const MAX_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;

/// Store an attachment in a live project and refuse to overwrite different bytes.
///
/// # Errors
/// The path is not a valid attachment path, the project is not live, or the file exceeds
/// [`MAX_ATTACHMENT_BYTES`].
pub fn attach(projects: &Projects, path: &str, bytes: &[u8]) -> Fallible<Attached> {
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!("an attachment is at most {MAX_ATTACHMENT_BYTES} bytes").into());
    }
    record::check_attachment(path)?;
    let (project, archived) = record::project_of(path)?;
    let domain = !archived && record::is_domain(&project);
    if !domain && (archived || !projects.by_project(&project)?.iter().any(|r| !r.archived)) {
        return Err(format!("no live project named {project:?}").into());
    }
    projects.attach(path, bytes)
}

/// Decode standard or URL-safe base64 (with optional `data:...;base64,` prefix) into raw bytes.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let payload = input
        .trim()
        .split_once(";base64,")
        .map_or(input.trim(), |(_, rest)| rest);
    let mut out = Vec::with_capacity(payload.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    let mut padded = false;
    for b in payload.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return Err("invalid base64: characters after padding".into());
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 byte 0x{b:02x}")),
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    if bits >= 6 || (bits > 0 && (buf & ((1 << bits) - 1)) != 0) {
        return Err("invalid base64 trailing bits".into());
    }
    Ok(out)
}

/// What one import run did, note by note.
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub unchanged: Vec<String>,
    /// Attachments copied, and attachments already present with the same bytes.
    pub attached: Vec<String>,
    pub attached_unchanged: usize,
    /// Not imported, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// Copy the vault's project notes into the store.
///
/// Every `.md` under a project folder moves; `Archive/` moves as archived; nothing from the law
/// folder moves. Binaries stay in the vault. A row that changed in
/// Sjel since its import is never overwritten.
///
/// With `folder`, the root is the vault itself and only that domain folder moves, under the
/// same path (`Atlas/Finance/...`), so a link that names the vault path still names the record.
pub fn import(
    projects: &Projects,
    vault_projects: &Path,
    only: Option<&str>,
    folder: Option<&str>,
) -> Fallible<ImportReport> {
    let prefix = match folder {
        Some(folder) => {
            let folder = folder.trim_matches('/');
            let top = folder.split('/').next().unwrap_or_default();
            if !record::is_domain(top) {
                return Err(format!("{folder:?} is not under a domain folder").into());
            }
            Some(format!("{folder}/"))
        }
        None => None,
    };
    let outside = |id: &str| prefix.as_ref().is_some_and(|p| !id.starts_with(p.as_str()));
    let root = MarkdownRoot::declare(vault_projects)?;
    let mut report = ImportReport::default();
    for file in root.markdown_files_recursive()? {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        if outside(&id) {
            continue;
        }
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
        if id.starts_with(&format!("{LAW_FOLDER}/")) {
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
    for file in root.files_recursive()? {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        if outside(&id) || id.starts_with(&format!("{LAW_FOLDER}/")) {
            continue;
        }
        let Ok((project, _)) = record::project_of(&id) else {
            continue;
        };
        if id.ends_with(".md") || only.is_some_and(|name| name != project) {
            continue;
        }
        match projects.attach(&id, &fs::read(&file)?)? {
            Attached::New => report.attached.push(id),
            Attached::Unchanged => report.attached_unchanged += 1,
            Attached::Differs => report.skipped.push((
                id,
                "a different attachment of that name is in Sjel; not overwritten".into(),
            )),
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

/// Mark each imported vault note as moved: a `moved_to` key and a banner.
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
            ("Sjel/PRD Axon.md", "# PRD\n"),
            (
                "Sjel/Knowledge-Base/Vault-Contract.md",
                "---\ntype: note\n---\nlaw\n",
            ),
            ("Projects.md", "---\ntype: note\n---\n"),
            ("Waschmaschine/Assets/Typenschild.jpg", "jpg"),
            ("Sjel/Knowledge-Base/diagram.png", "law"),
            (
                "Sjel/Knowledge-Base/Knowledge-Base.md",
                "---\ntype: project\n---\nlaw hub\n",
            ),
        ] {
            let file = v.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        v
    }

    /// A domain folder imports under its vault path and takes nothing outside it.
    #[test]
    fn a_domain_folder_imports_under_its_vault_path() {
        let (projects, dir) = scratch("domain-import");
        let v = dir.join("vault");
        for (path, content) in [
            (
                "Atlas/Finance/Subscriptions/Claude Pro.md",
                "---\ncost: 20\n---\nwhy\n",
            ),
            ("Atlas/Finance/Finance.md", "---\ntype: note\n---\n"),
            ("Atlas/People/Ron.md", "---\ntype: note\n---\n"),
            ("Knowledge/Rust.md", "---\ntype: note\n---\n"),
            ("Atlas/Finance/receipt.pdf", "pdf"),
        ] {
            let file = v.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let report = import(&projects, &v, None, Some("Atlas/Finance")).unwrap();
        assert_eq!(report.imported.len(), 2, "{report:?}");
        assert_eq!(report.attached, vec!["Atlas/Finance/receipt.pdf"]);
        let note = projects
            .get("Atlas/Finance/Subscriptions/Claude Pro.md")
            .unwrap()
            .unwrap();
        assert!(note.domain);
        assert!(import(&projects, &v, None, Some("Knowledge")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    /// A domain folder holds notes only: never a project, a task or a project-list row.
    #[test]
    fn a_domain_is_not_a_project() {
        let (projects, dir) = scratch("domain");
        let note = Record::from_content(
            "Atlas/Finance/Subscriptions/Claude Pro.md",
            "---\ntype: task\ndone: false\n---\nwhy\n".into(),
        )
        .unwrap();
        assert!(note.domain);
        assert_eq!(
            note.kind,
            Kind::Note,
            "a domain record is a note whatever its type says"
        );
        assert!(
            save(&projects, &note.path, &note.content, None).is_ok(),
            "no hub needed"
        );
        assert!(attach(&projects, "Atlas/Finance/receipt.pdf", b"pdf").is_ok());
        assert!(add_project(&projects, "Atlas", "").is_err());
        assert!(add_project(&projects, "Journal", "").is_err());
        assert!(add_task(&projects, "Atlas", "T", "", None, 2).is_err());
        assert!(new_note("Journal", "N", "").is_err());
        assert!(records(&projects, Kind::Note).unwrap().is_empty());
        assert!(
            !Record::from_content("Archive/Atlas/x.md", String::new())
                .unwrap()
                .domain
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_takes_project_notes_and_leaves_the_law() {
        let (projects, dir) = scratch("import");
        let v = vault(&dir);
        let report = import(&projects, &v, None, None).unwrap();
        assert_eq!(report.imported.len(), 6, "{report:?}");
        assert!(report.imported.iter().any(|p| p == "Sjel/PRD Axon.md"));
        assert_eq!(
            report.attached,
            vec!["Waschmaschine/Assets/Typenschild.jpg"]
        );
        assert!(!report.imported.iter().any(|p| p.contains("Knowledge-Base")));
        assert_eq!(report.skipped.len(), 1, "Projects.md has no project folder");
        assert_eq!(
            import(&projects, &v, None, None).unwrap().unchanged.len(),
            6
        );
        let open = tasks(&projects, Some("open")).unwrap();
        assert_eq!(open.len(), 1, "the archived task is not served");
        assert_eq!(open[0].uri, "/projects?project=Waschmaschine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn freeze_marks_unchanged_notes_and_import_then_skips_them() {
        let (projects, dir) = scratch("freeze");
        let v = vault(&dir);
        import(&projects, &v, Some("Waschmaschine"), None).unwrap();
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
        let again = import(&projects, &v, Some("Waschmaschine"), None).unwrap();
        assert!(again.imported.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_never_overwrites_a_change_made_in_sjel() {
        let (projects, dir) = scratch("conflict");
        let v = vault(&dir);
        import(&projects, &v, Some("Waschmaschine"), None).unwrap();
        set_done(&projects, "Waschmaschine/Tasks/Schlauch.md", true).unwrap();
        let report = import(&projects, &v, Some("Waschmaschine"), None).unwrap();
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
    fn set_property_checks_schema_keys_and_status_drives_done() {
        let (projects, dir) = scratch("property");
        add_project(&projects, "W", "").unwrap();
        let path = add_task(&projects, "W", "x", "", None, 2).unwrap().path;
        assert!(set_property(&projects, &path, "priority", "4").is_err());
        assert!(set_property(&projects, &path, "due", "24.10.").is_err());
        assert!(set_property(&projects, &path, "status", "todo").is_err());
        assert!(set_property(&projects, &path, "done", "true").is_err());
        let r = set_property(&projects, &path, "context", "Werkstatt").unwrap();
        assert_eq!(r.fields()["context"], "Werkstatt");
        let r = set_property(&projects, &path, "status", "done").unwrap();
        assert!(r.done && r.fields().contains_key("completed"));
        let r = set_property(&projects, &path, "status", "waiting").unwrap();
        assert!(!r.done && !r.fields().contains_key("completed"));
        assert_eq!(r.fields()["status"], "waiting");
        let r = set_property(&projects, &path, "due", "").unwrap();
        assert_eq!(r.due, None);
        assert!(set_property(&projects, "W/W.md", "status", "open").is_err());
        set_property(&projects, "W/W.md", "status", "done").unwrap();
        assert_eq!(records(&projects, Kind::Project).unwrap().len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn normalize_lets_done_decide_status_and_only_writes_when_applied() {
        let (projects, dir) = scratch("normalize");
        let messy =
            "---\ntype: task\ndone: true\nstatus: open\ndone_date: 2026-01-02\npriority: 5\n---\n";
        projects
            .put(&Record::from_content("W/Tasks/a.md", messy.into()).unwrap())
            .unwrap();
        let clean = "---\ntype: task\ndone: false\nstatus: waiting\npriority: 1\n---\n";
        projects
            .put(&Record::from_content("W/Tasks/b.md", clean.into()).unwrap())
            .unwrap();
        let report = normalize(&projects, false).unwrap();
        assert_eq!(report.len(), 1, "{report:?}");
        assert_eq!(report[0].changes.len(), 3);
        assert_eq!(
            projects.get("W/Tasks/a.md").unwrap().unwrap().content,
            messy
        );
        normalize(&projects, true).unwrap();
        let a = projects.get("W/Tasks/a.md").unwrap().unwrap().content;
        assert_eq!(
            a,
            "---\ntype: task\ndone: true\nstatus: done\npriority: 3\ncompleted: 2026-01-02\n---\n"
        );
        assert!(normalize(&projects, false).unwrap().is_empty());
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

    #[test]
    fn attach_requires_a_live_project_and_refuses_different_bytes() {
        let (projects, dir) = scratch("ops-attach");
        assert!(attach(&projects, "W/Assets/manual.pdf", b"pdf-1").is_err());
        add_project(&projects, "W", "").unwrap();
        assert_eq!(
            attach(&projects, "W/Assets/manual.pdf", b"pdf-1").unwrap(),
            Attached::New
        );
        assert_eq!(
            attach(&projects, "W/Assets/manual.pdf", b"pdf-1").unwrap(),
            Attached::Unchanged
        );
        assert_eq!(
            attach(&projects, "W/Assets/manual.pdf", b"pdf-2").unwrap(),
            Attached::Differs
        );
        assert_eq!(
            projects
                .attachment("W/Assets/manual.pdf")
                .unwrap()
                .as_deref(),
            Some(&b"pdf-1"[..])
        );
        let decoded = decode_base64("data:application/pdf;base64,cGRmLTE=").unwrap();
        assert_eq!(decoded, b"pdf-1");
        fs::remove_dir_all(dir).unwrap();
    }
}

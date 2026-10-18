//! One record is one markdown file: its path under the projects root and its bytes.
//!
//! Every other column is derived from those two, so a store rebuilt from the rendered files
//! equals the live store (ISA.md ISC-69). The selection rules follow
//! `capabilities/vault/src/tasks.rs`, so a task means the same thing on both sides of the import.

use std::collections::HashMap;

use markdown_root::frontmatter_spanned;
use serde::Serialize;

/// The folder segment that marks a finished project.
pub const ARCHIVE: &str = "Archive";
const TASKS: &str = "Tasks";
const DEFAULT_PRIORITY: u8 = 2;
const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Project,
    Task,
    Note,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Project => "project",
            Kind::Task => "task",
            Kind::Note => "note",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Slash-separated, relative to the projects root, ends in `.md`.
    pub path: String,
    pub project: String,
    pub kind: Kind,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub archived: bool,
    pub content: String,
}

impl Record {
    /// Derive a record from a path and the file's bytes.
    ///
    /// # Errors
    /// The path escapes the root, is not a markdown file, or sits directly in the root with no
    /// project folder above it; or the frontmatter is not closed.
    pub fn from_content(path: &str, content: String) -> Result<Self, String> {
        let segments = checked_segments(path)?;
        let archived = segments[0] == ARCHIVE;
        let folders = &segments[..segments.len() - 1];
        let title = segments[segments.len() - 1]
            .strip_suffix(".md")
            .unwrap_or_default()
            .to_string();
        let project = match (archived, folders.len()) {
            (false, 0) => return Err(format!("{path}: a record needs a project folder")),
            (true, 1) => title.clone(),
            (true, _) => folders[1].to_string(),
            (false, _) => folders[0].to_string(),
        };
        let fields = frontmatter_spanned(&content)
            .map_err(|e| format!("{path}: {e}"))?
            .fields;
        let kind = match fields.get("type").map(String::as_str) {
            Some("project") => Kind::Project,
            Some("task") => Kind::Task,
            _ if fields.contains_key("done") && folders.iter().skip(1).any(|f| *f == TASKS) => {
                Kind::Task
            }
            _ => Kind::Note,
        };
        Ok(Self {
            path: path.to_string(),
            project,
            kind,
            title,
            done: fields.get("done").map(String::as_str) == Some("true"),
            due: value(&fields, "due"),
            priority: value(&fields, "priority")
                .and_then(|v| v.parse().ok())
                .filter(|p| (1..=3).contains(p))
                .unwrap_or(DEFAULT_PRIORITY),
            archived,
            content,
        })
    }

    pub fn fields(&self) -> HashMap<String, String> {
        frontmatter_spanned(&self.content)
            .map(|f| f.fields)
            .unwrap_or_default()
    }
}

fn checked_segments(path: &str) -> Result<Vec<&str>, String> {
    if !path.ends_with(".md") || path.contains('\\') {
        return Err(format!("{path}: not a markdown path"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == ".." || s.starts_with('.'))
    {
        return Err(format!("{path}: path leaves the projects root"));
    }
    Ok(segments)
}

/// Present and not blank, the same rule the vault reader applies.
pub fn value(fields: &HashMap<String, String>, key: &str) -> Option<String> {
    fields
        .get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Check a title or project name that becomes one path segment.
///
/// # Errors
/// Empty, too long, or contains a character that would change the path.
pub fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("a name needs 1 to {MAX_TITLE_CHARS} characters"));
    }
    if name.starts_with('.') || name.contains(['/', '\\', '\n', '\r', '"']) {
        return Err(format!(
            "{name:?}: a name cannot start with '.' or contain / \\ \" or a line break"
        ));
    }
    Ok(name)
}

/// The content of a new task note, with the keys of the vault's task template.
pub fn new_task(project: &str, summary: &str, due: Option<&str>, priority: u8) -> String {
    format!(
        "---\ntype: task\nsummary: \"{summary}\"\ndone: false\nstatus: open\ndue: {due}\nscheduled:\npriority: {priority}\ncontext: personal\nenergy:\nprojects:\n  - \"[[{project}]]\"\nfocus: []\nevents: []\nblocked_by:\n---\n\n## Details\n\n- [ ]\n",
        summary = summary.replace('"', "'"),
        due = due.unwrap_or_default(),
    )
}

/// The content of a new project hub.
pub fn new_project(name: &str, summary: &str) -> String {
    format!(
        "---\ntype: project\nsummary: \"{summary}\"\nstatus: active\npriority: 2\ncontext: personal\noutcome:\n---\n\n# {name}\n",
        summary = summary.replace('"', "'"),
    )
}

/// Set `done:` in the frontmatter, and `status:` and `completed:` with it where the note uses them.
///
/// Only those lines change. The rest of the file keeps its bytes, which is what keeps an imported
/// note's Bases embeds, callouts and Mermaid fences intact.
///
/// # Errors
/// The note has no frontmatter block.
pub fn set_done(content: &str, done: bool, today: &str) -> Result<String, String> {
    let parsed = frontmatter_spanned(content)?;
    let (start, end) = parsed.block.ok_or("the note has no frontmatter")?;
    let mut lines: Vec<String> = Vec::new();
    let mut has_done = false;
    let mut has_status = false;
    for line in content[start..end].lines() {
        let key = line.split(':').next().unwrap_or_default();
        match key {
            "done" => {
                has_done = true;
                lines.push(format!("done: {done}"));
            }
            "status" if !line.starts_with(' ') => {
                has_status = true;
                lines.push(format!("status: {}", if done { "done" } else { "open" }));
                if done {
                    lines.push(format!("completed: {today}"));
                }
            }
            "completed" => {}
            _ => lines.push(line.to_string()),
        }
    }
    if !has_done {
        lines.push(format!("done: {done}"));
    }
    if done && !has_status {
        lines.push(format!("completed: {today}"));
    }
    let mut yaml = lines.join("\n");
    if content[start..end].ends_with('\n') {
        yaml.push('\n');
    }
    Ok(format!("{}{yaml}{}", &content[..start], &content[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = "---\ntype: task\nsummary: \"x\"\ndone: false\nstatus: open\ndue: 2026-10-24\npriority: 1\nprojects:\n  - \"[[Waschmaschine]]\"\n---\n\n## Details\n\n> [!note] kept\n";

    #[test]
    fn derives_project_kind_and_fields() {
        let r = Record::from_content("Waschmaschine/Tasks/Nachfassen.md", TASK.into()).unwrap();
        assert_eq!(r.project, "Waschmaschine");
        assert_eq!(r.kind, Kind::Task);
        assert_eq!(r.title, "Nachfassen");
        assert_eq!(r.due.as_deref(), Some("2026-10-24"));
        assert_eq!(r.priority, 1);
        assert!(!r.done && !r.archived);
    }

    #[test]
    fn archive_project_is_the_second_segment_or_the_file() {
        let a = Record::from_content("Archive/Ledger/Ledger.md", "x".into()).unwrap();
        assert_eq!((a.project.as_str(), a.archived), ("Ledger", true));
        let b = Record::from_content("Archive/Loose.md", "x".into()).unwrap();
        assert_eq!(b.project, "Loose");
    }

    #[test]
    fn refuses_paths_outside_a_project() {
        for bad in [
            "Projects.md",
            "../x/y.md",
            "a/./b.md",
            "a/.hidden/b.md",
            "a/b.txt",
            "/a/b.md",
        ] {
            assert!(Record::from_content(bad, String::new()).is_err(), "{bad}");
        }
    }

    #[test]
    fn set_done_changes_only_state_lines() {
        let done = set_done(TASK, true, "2026-10-17").unwrap();
        assert!(done.contains("done: true\nstatus: done\ncompleted: 2026-10-17\n"));
        assert!(done.ends_with("> [!note] kept\n"));
        let open = set_done(&done, false, "2026-10-18").unwrap();
        assert_eq!(open, TASK);
    }

    #[test]
    fn new_task_is_read_back_as_a_task() {
        let content = new_task("Waschmaschine", "Schelle kaufen", Some("2026-10-20"), 1);
        let r = Record::from_content("Waschmaschine/Tasks/Schelle.md", content).unwrap();
        assert_eq!(
            (r.kind, r.priority, r.due.as_deref()),
            (Kind::Task, 1, Some("2026-10-20"))
        );
    }

    #[test]
    fn check_name_refuses_path_characters() {
        assert!(check_name("a/b").is_err());
        assert!(check_name(".x").is_err());
        assert_eq!(check_name("  Ablauf  ").unwrap(), "Ablauf");
    }
}

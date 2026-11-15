//! Read-only task views over explicitly declared project Markdown sources.

use std::{collections::HashSet, fs};

use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Deserialize;

use crate::{record, Kind, Projects, Record};

type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const SOURCE_FOLDER: &str = "External tasks";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub project: String,
    pub root: String,
    pub pattern: String,
    pub format: Format,
    #[serde(default = "private_class")]
    pub data_class: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Notes,
    Checklist,
}

fn private_class() -> String {
    "c2".into()
}

/// Whether a path belongs to a source view rather than to the Projects store.
pub fn is_external(path: &str) -> bool {
    projects_api::is_external_task(path)
}

/// Read current source tasks for live, stored project hubs.
///
/// # Errors
/// A declared source is invalid, missing, unreadable, oversized or escapes its root.
pub async fn records(projects: &Projects) -> Fallible<Vec<(Record, String)>> {
    let Some(config) = projects.source_config.clone() else {
        return Ok(Vec::new());
    };
    let live: HashSet<String> = projects
        .by_kind(Kind::Project)
        .await?
        .into_iter()
        .filter(|r| !r.archived && !r.domain)
        .map(|r| r.project)
        .collect();
    tokio::task::spawn_blocking(move || {
        let bytes = match fs::read(&config) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let sources: Vec<Source> = serde_json::from_slice(&bytes)?;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for source in sources {
            record::check_name(&source.project)?;
            if !["c1", "c2"].contains(&source.data_class.as_str()) {
                return Err("a task source class must be c1 or c2".into());
            }
            if !seen.insert(source.project.clone()) {
                return Err(format!("duplicate task source for {}", source.project).into());
            }
            if live.contains(&source.project) {
                result.extend(read(&source)?);
            }
        }
        Ok(result)
    })
    .await?
}

fn read(source: &Source) -> Fallible<Vec<(Record, String)>> {
    let root = MarkdownRoot::declare(sjel_config::expand_tilde(&source.root))?;
    let files = root.markdown_files(&source.pattern)?;
    if files.len() > 1000 {
        return Err("a task source may contain at most 1000 files".into());
    }
    // An exact file disappearing must not look like an empty task list.
    if files.is_empty() && !source.pattern.ends_with("*.md") {
        return Err(format!("task source {} is missing", source.pattern).into());
    }
    let mut result = Vec::new();
    for file in files {
        let metadata = fs::metadata(&file)?;
        if metadata.len() > crate::ops::MAX_CONTENT_BYTES as u64 {
            return Err("a task source file exceeds the record size limit".into());
        }
        let content = fs::read_to_string(&file)?;
        let fields = frontmatter_spanned(&content)?.fields;
        let declared_class = fields
            .get("class")
            .map_or(source.data_class.as_str(), String::as_str);
        if !["c0", "c1", "c2", "c3"].contains(&declared_class) {
            return Err("a source note declares an unknown class".into());
        }
        let class = source.data_class.as_str().max(declared_class);
        let updated: chrono::DateTime<chrono::Utc> = metadata.modified()?.into();
        let updated = updated.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let relative = root
            .relative_id(&file)
            .ok_or("source path has no relative identity")?;
        let source_uri = file.to_string_lossy();
        let file_uri = reqwest::Url::from_file_path(&file)
            .map_err(|_| "source path cannot form a file URL")?
            .to_string();
        match source.format {
            Format::Notes => {
                if fields.get("typ").map(String::as_str) != Some("Aufgabe")
                    && fields.get("type").map(String::as_str) != Some("task")
                {
                    continue;
                }
                let status = normalize_status(fields.get("status").map_or("open", String::as_str))?;
                let title = file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or("invalid task filename")?;
                let mut view = content;
                for (key, value) in [
                    ("type", "task"),
                    ("class", class),
                    ("status", status),
                    (
                        "done",
                        if crate::ops::status_closes(status) {
                            "true"
                        } else {
                            "false"
                        },
                    ),
                    ("read_only", "true"),
                    ("source_path", &source_uri),
                    ("source_uri", &file_uri),
                ] {
                    view = record::set_field(&view, key, Some(value))?;
                }
                for (from, to) in [("wer", "owner"), ("naechster_schritt", "summary")] {
                    if let Some(value) = fields.get(from) {
                        view = record::set_field(&view, to, Some(value))?;
                    }
                }
                let mut view = Record::from_content(
                    &format!("{}/{SOURCE_FOLDER}/{relative}", source.project),
                    view,
                )?;
                view.title = title.into();
                result.push((view, updated.clone()));
            }
            Format::Checklist => {
                let mut fenced = false;
                for (line, text) in content.lines().enumerate() {
                    let text = text.trim();
                    if text.starts_with("```") || text.starts_with("~~~") {
                        fenced = !fenced;
                        continue;
                    }
                    if fenced {
                        continue;
                    }
                    let Some((done, title)) = checklist_item(text) else {
                        continue;
                    };
                    let title = title.trim();
                    if title.is_empty() {
                        continue;
                    }
                    let view = format!(
                        "---\ntype: task\nclass: {class}\nread_only: true\ndone: {done}\nstatus: {}\nsource_path: {}\nsource_uri: {}\nsource_line: {}\nsummary: {}\n---\n\n{title}\n",
                        if done { "done" } else { "open" },
                        serde_json::to_string(source_uri.as_ref())?, serde_json::to_string(&file_uri)?, line + 1,
                        serde_json::to_string(title)?,
                    );
                    let mut view = Record::from_content(
                        &format!(
                            "{}/{SOURCE_FOLDER}/{relative}/{:04}.md",
                            source.project,
                            line + 1
                        ),
                        view,
                    )?;
                    view.title = title.chars().take(200).collect();
                    result.push((view, updated.clone()));
                }
            }
        }
    }
    Ok(result)
}

fn checklist_item(text: &str) -> Option<(bool, &str)> {
    let item = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .or_else(|| {
            let (number, rest) = text.split_once(". ")?;
            (!number.is_empty() && number.bytes().all(|c| c.is_ascii_digit())).then_some(rest)
        })?;
    if let Some(title) = item.strip_prefix("[ ] ") {
        Some((false, title))
    } else if let Some(title) = item
        .strip_prefix("[x] ")
        .or_else(|| item.strip_prefix("[X] "))
    {
        Some((true, title))
    } else if text.starts_with(['-', '*']) {
        None
    } else {
        Some((false, item))
    }
}

fn normalize_status(status: &str) -> Fallible<&str> {
    match status {
        "Offen" | "open" => Ok("open"),
        "Wartet" | "waiting" => Ok("waiting"),
        "Erledigt" | "done" => Ok("done"),
        "doing" | "blocked" | "cancelled" => Ok(status),
        _ => Err(format!("unsupported source task status {status:?}").into()),
    }
}

/// Refuse a second task authority for a project with a declared source.
///
/// # Errors
/// The source configuration cannot be read, or the project uses an external task source.
pub fn require_local_tasks(projects: &Projects, project: &str) -> Fallible<()> {
    let Some(path) = projects.source_config.as_ref() else {
        return Ok(());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let sources: Vec<Source> = serde_json::from_slice(&bytes)?;
    if sources.iter().any(|s| s.project == project) {
        return Err("this project's tasks belong to its declared source; edit that source".into());
    }
    Ok(())
}

/// Resolve one source view for detail reads without storing a second copy.
///
/// # Errors
/// A configured source cannot be read.
pub async fn get(projects: &Projects, path: &str) -> Fallible<Option<(Record, String)>> {
    if !is_external(path) {
        return Ok(None);
    }
    Ok(records(projects)
        .await?
        .into_iter()
        .find(|(r, _)| r.path == path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn source_edits_and_deletions_are_live_and_never_become_rows() {
        let (projects, dir) = crate::store::tests::scratch("source-views").await;
        crate::ops::add_project(&projects, "Client", "")
            .await
            .unwrap();
        let root = dir.join("client");
        fs::create_dir(&root).unwrap();
        let file = root.join("Question.md");
        fs::write(
            &file,
            "---\ntyp: Aufgabe\nstatus: Wartet\nwer: Owner\n---\n\nQuestion\n",
        )
        .unwrap();
        let config = dir.join("sources.json");
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!([{
                "project":"Client", "root":root, "pattern":"*.md", "format":"notes"
            }]))
            .unwrap(),
        )
        .unwrap();
        let projects = projects.with_sources(Some(config));
        let first = records(&projects).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0.fields()["status"], "waiting");
        assert_eq!(first[0].0.fields()["class"], "c2");
        assert!(projects.get(&first[0].0.path).await.unwrap().is_none());
        assert!(
            crate::ops::add_task(&projects, "Client", "Duplicate", "", None, 2)
                .await
                .is_err()
        );
        assert!(
            crate::ops::save(&projects, &first[0].0.path, &first[0].0.content, None)
                .await
                .is_err()
        );
        fs::write(
            &file,
            "---\ntyp: Aufgabe\nstatus: Erledigt\nclass: c3\n---\n",
        )
        .unwrap();
        let next = records(&projects).await.unwrap();
        assert!(next[0].0.done);
        assert_eq!(next[0].0.fields()["class"], "c3");
        fs::remove_file(file).unwrap();
        assert!(records(&projects).await.unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checklist_ignores_prose_and_keeps_completion() {
        assert_eq!(
            checklist_item("1. Review mobile"),
            Some((false, "Review mobile"))
        );
        assert_eq!(checklist_item("- [x] Finished"), Some((true, "Finished")));
        assert_eq!(checklist_item("- Uncommitted idea"), None);
        assert_eq!(checklist_item("Developer follow-up: later"), None);
    }

    #[test]
    fn symlinks_outside_the_declared_source_are_refused() {
        let dir = std::env::temp_dir().join(format!("sjel-source-boundary-{}", std::process::id()));
        fs::create_dir_all(dir.join("source")).unwrap();
        fs::write(dir.join("outside.md"), "not a task").unwrap();
        std::os::unix::fs::symlink(dir.join("outside.md"), dir.join("source/link.md")).unwrap();
        let source = Source {
            project: "Client".into(),
            root: dir.join("source").to_string_lossy().into(),
            pattern: "*.md".into(),
            format: Format::Notes,
            data_class: "c2".into(),
        };
        assert!(read(&source).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

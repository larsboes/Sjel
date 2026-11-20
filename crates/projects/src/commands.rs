//! The Projects command set, shared by the CLI and the owner's local job socket.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::{ops, Projects};

type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const USAGE: &str = "usage: projects <command>
  list [--all|--done]                          open tasks of live projects
  projects                                     every project with its task counts
  add <project> <title> [--due YYYY-MM-DD] [--priority 1|2|3] [--summary TEXT]
  new-project <name> [--summary TEXT]
  archive <project>                            move a live project's folder under Archive/
  unarchive <project>                          move an archived project's folder back
  new-note <project> <title> [--summary TEXT]
  edit <path>                                  open a record in $EDITOR and save it
  put <path> <file> [--base <file>]            create a record from a file; with --base, replace it
                                               only while the stored content still equals that file
  done <task-id>  |  reopen <task-id>
  set <path> <key> [value]                     set one property; no value clears it
  attach <path> <file>                         store a binary attachment in a live project
  remove <path>                                delete a record and its file
  rename <path> <title>                        move a record's file to the new title and
                                               rewrite the [[links]] that name the old one
  normalize [--apply]                          put every task on the status schema; dry run first
  import <vault>/Projects [--only PROJECT]     copy vault project notes into the store
  import <vault> --folder Atlas/<Domain>       copy one domain folder, keeping its path
  freeze-vault <vault>/Projects [--only PROJECT]  mark imported vault notes as moved
  show <path>                                  print a record's stored content
  verify                                       compare the index with the files
  reindex                                      adopt file edits; refuse a lowered class
  workspace adopt <manifest.json>              adopt private indexes and delivery assets
  workspace setup <name> <template-directory>   prepare a private project from Markdown templates
  workspace status [--github]                   source tasks, next actions, checkout and optional CI
  workspace lesson draft|publish ...            prepare and explicitly review reusable lessons";

/// Run one command against an open store. Command output goes to `out`; notes from the implicit
/// reindex go to `log`.
///
/// # Errors
/// The command is unknown, its arguments are invalid, or the store refuses the change.
pub async fn run(
    projects: &Projects,
    args: &[String],
    out: &mut String,
    log: &mut String,
) -> Fallible<()> {
    let Some(command) = args.first() else {
        return Err(USAGE.into());
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let positional = |n: usize| {
        args.get(n)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .ok_or(USAGE)
    };
    // The files are the authority, so every command but those that inspect the index starts
    // from them.
    if !matches!(command.as_str(), "verify" | "reindex" | "show") {
        report_reindex(&projects.reindex().await?, false, out, log);
    }
    match command.as_str() {
        "workspace" => crate::workspace::run(projects, &args[1..], out).await?,
        "list" => {
            let status = if args.iter().any(|a| a == "--all") {
                None
            } else if args.iter().any(|a| a == "--done") {
                Some("done")
            } else {
                Some("open")
            };
            for t in ops::tasks(projects, status).await? {
                writeln!(
                    out,
                    "{} P{} {:10} {}  [{}]",
                    if t.done { "x" } else { " " },
                    t.priority,
                    t.due.as_deref().unwrap_or("-"),
                    t.title,
                    t.id
                )?;
            }
        }
        "projects" => {
            let mut rows: std::collections::BTreeMap<(bool, String), (usize, usize, usize)> =
                Default::default();
            for r in ops::view_records(projects)
                .await?
                .into_iter()
                .filter(|r| !r.domain)
            {
                let row = rows.entry((r.archived, r.project.clone())).or_default();
                match (r.kind, r.done) {
                    (crate::Kind::Task, false) => row.0 += 1,
                    (crate::Kind::Task, true) => row.1 += 1,
                    _ => row.2 += 1,
                }
            }
            for ((archived, name), (open, done, notes)) in rows {
                let state = if archived { "archived" } else { "live" };
                writeln!(
                    out,
                    "{state:8} {name:30} {open:3} open {done:3} done {notes:3} notes"
                )?;
            }
        }
        "add" => {
            let priority = flag("--priority").map(str::parse).transpose()?.unwrap_or(2);
            let r = ops::add_task(
                projects,
                positional(1)?,
                positional(2)?,
                flag("--summary").unwrap_or_default(),
                flag("--due"),
                priority,
            )
            .await?;
            writeln!(out, "added {}", r.path)?;
        }
        "new-project" => {
            let r = ops::add_project(
                projects,
                positional(1)?,
                flag("--summary").unwrap_or_default(),
            )
            .await?;
            writeln!(out, "added {}", r.path)?;
        }
        "new-note" => {
            let (path, content) = ops::new_note(
                positional(1)?,
                positional(2)?,
                flag("--summary").unwrap_or_default(),
            )?;
            writeln!(
                out,
                "added {}",
                ops::save(projects, &path, &content, None).await?.path
            )?;
        }
        "edit" => return Err("edit runs in the CLI, which forwards the saved file".into()),
        "show" => {
            let path = positional(1)?;
            let record = projects
                .get(path)
                .await?
                .ok_or_else(|| format!("no record {path:?}"))?;
            out.push_str(&record.content);
        }
        "put" => {
            let path = positional(1)?;
            let content = std::fs::read_to_string(positional(2)?)?;
            let base = match flag("--base") {
                Some(file) => Some(std::fs::read_to_string(file)?),
                None => None,
            };
            if base.as_deref() == Some(content.as_str()) {
                writeln!(out, "unchanged {path}")?;
            } else {
                let saved = ops::save(projects, path, &content, base.as_deref()).await?;
                writeln!(
                    out,
                    "{} {}",
                    if base.is_some() { "saved" } else { "added" },
                    saved.path
                )?;
            }
        }
        "done" | "reopen" => {
            let r = ops::set_done(projects, positional(1)?, command == "done").await?;
            writeln!(out, "{} {}", if r.done { "done" } else { "open" }, r.path)?;
        }
        "set" => {
            let value = args.get(3).map(String::as_str).unwrap_or_default();
            let r = ops::set_property(projects, positional(1)?, positional(2)?, value).await?;
            writeln!(
                out,
                "{}: {}",
                r.path,
                r.fields().get(positional(2)?).map_or("", String::as_str)
            )?;
        }
        "attach" => {
            let path = positional(1)?;
            let file = positional(2)?;
            let bytes = std::fs::read(file)?;
            match ops::attach(projects, path, &bytes).await? {
                crate::store::Attached::New => writeln!(out, "attached {path}")?,
                crate::store::Attached::Unchanged => writeln!(out, "unchanged {path}")?,
                crate::store::Attached::Differs => {
                    return Err(format!(
                        "a different attachment already exists at {path:?}; not overwritten"
                    )
                    .into());
                }
            }
        }
        "remove" => {
            let path = positional(1)?;
            let removed = projects.remove(path).await?;
            writeln!(
                out,
                "{} {path}",
                if removed { "removed" } else { "no record" }
            )?;
        }
        "archive" | "unarchive" => {
            let name = positional(1)?;
            let moved = if command == "archive" {
                ops::archive(projects, name).await?
            } else {
                ops::unarchive(projects, name).await?
            };
            writeln!(
                out,
                "{command}d {name}: {} record(s), {} attachment(s)",
                moved.records.len(),
                moved.attachments.len()
            )?;
        }
        "rename" => {
            let renamed = ops::rename(projects, positional(1)?, positional(2)?).await?;
            writeln!(
                out,
                "{} -> {} ({} link(s) rewritten)",
                positional(1)?,
                renamed.path,
                renamed.links.len()
            )?;
            for left in &renamed.failed {
                writeln!(out, "  still names the old title: {left}")?;
            }
        }
        "normalize" => {
            let apply = args.iter().any(|a| a == "--apply");
            let report = ops::normalize(projects, apply).await?;
            for n in &report {
                writeln!(out, "{}\n  {}", n.path, n.changes.join("\n  "))?;
            }
            let verb = if apply {
                "normalized"
            } else {
                "would normalize (dry run; add --apply)"
            };
            writeln!(out, "{verb} {} task(s)", report.len())?;
        }
        "import" => {
            let report = ops::import(
                projects,
                &PathBuf::from(positional(1)?),
                flag("--only"),
                flag("--folder"),
            )
            .await?;
            for (path, reason) in &report.skipped {
                writeln!(out, "skipped {path}: {reason}")?;
            }
            writeln!(
                out,
                "imported {}, unchanged {}, attachments {} new and {} unchanged, skipped {}",
                report.imported.len(),
                report.unchanged.len(),
                report.attached.len(),
                report.attached_unchanged,
                report.skipped.len()
            )?;
        }
        "freeze-vault" => {
            let report =
                ops::freeze_vault(projects, &PathBuf::from(positional(1)?), flag("--only")).await?;
            for (path, reason) in &report.skipped {
                writeln!(out, "skipped {path}: {reason}")?;
            }
            writeln!(
                out,
                "frozen {}, already frozen {}, skipped {}",
                report.frozen.len(),
                report.already.len(),
                report.skipped.len()
            )?;
        }
        "verify" => {
            let drift = projects.verify().await?;
            for path in &drift.stale {
                writeln!(out, "stale    {path}")?;
            }
            for path in &drift.orphaned {
                writeln!(out, "orphaned {path}")?;
            }
            if drift.stale.is_empty() && drift.orphaned.is_empty() {
                writeln!(out, "index and files match")?;
            } else {
                return Err("index and files differ; `projects reindex` adopts the files".into());
            }
        }
        "reindex" => report_reindex(&projects.reindex().await?, true, out, log),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// Changes go to stderr on an implicit reindex, so a command's own output stays parseable.
fn report_reindex(
    report: &crate::store::Reindexed,
    explicit: bool,
    out: &mut String,
    log: &mut String,
) {
    let mut line = |text: String| {
        if explicit {
            out.push_str(&text);
            out.push('\n');
        } else {
            log.push_str("projects: ");
            log.push_str(&text);
            log.push('\n');
        }
    };
    for path in &report.adopted {
        line(format!("adopted  {path}"));
    }
    for path in &report.added {
        line(format!("added    {path}"));
    }
    for path in &report.dropped {
        line(format!("dropped  {path}"));
    }
    for (path, why) in &report.refused {
        line(format!("refused  {path}: {why}"));
    }
    for (path, why) in &report.skipped {
        line(format!("skipped  {path}: {why}"));
    }
    if explicit
        && report.adopted.is_empty()
        && report.added.is_empty()
        && report.dropped.is_empty()
        && report.refused.is_empty()
    {
        line("index and files match".into());
    }
}

use std::path::PathBuf;

use projects::{open_default, ops};

const USAGE: &str = "usage: projects <command>
  list [--all|--done]                          open tasks of live projects
  projects                                     every project with its task counts
  add <project> <title> [--due YYYY-MM-DD] [--priority 1|2|3] [--summary TEXT]
  new-project <name> [--summary TEXT]
  new-note <project> <title> [--summary TEXT]
  edit <path>                                  open a record in $EDITOR and save it
  done <task-id>  |  reopen <task-id>
  set <path> <key> [value]                     set one property; no value clears it
  attach <path> <file>                         store a binary attachment in a live project
  remove <path>                                delete a record and its rendered file
  normalize [--apply]                          put every task on the status schema; dry run first
  import <vault>/Projects [--only PROJECT]     copy vault project notes into the store
  import <vault> --folder Atlas/<Domain>       copy one domain folder, keeping its path
  freeze-vault <vault>/Projects [--only PROJECT]  mark imported vault notes as moved
  verify                                       compare the store with the rendered files
  render                                       re-render every record";

fn main() {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("projects: tokio runtime could not start: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = runtime.block_on(run()) {
        eprintln!("projects: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    let projects = open_default().await?;
    match command.as_str() {
        "list" => {
            let status = if args.iter().any(|a| a == "--all") {
                None
            } else if args.iter().any(|a| a == "--done") {
                Some("done")
            } else {
                Some("open")
            };
            for t in ops::tasks(&projects, status).await? {
                println!(
                    "{} P{} {:10} {}  [{}]",
                    if t.done { "x" } else { " " },
                    t.priority,
                    t.due.as_deref().unwrap_or("-"),
                    t.title,
                    t.id
                );
            }
        }
        "projects" => {
            let mut rows: std::collections::BTreeMap<(bool, String), (usize, usize, usize)> =
                Default::default();
            for r in projects.all().await?.into_iter().filter(|r| !r.domain) {
                let row = rows.entry((r.archived, r.project.clone())).or_default();
                match (r.kind, r.done) {
                    (projects::Kind::Task, false) => row.0 += 1,
                    (projects::Kind::Task, true) => row.1 += 1,
                    _ => row.2 += 1,
                }
            }
            for ((archived, name), (open, done, notes)) in rows {
                let state = if archived { "archived" } else { "live" };
                println!("{state:8} {name:30} {open:3} open {done:3} done {notes:3} notes");
            }
        }
        "add" => {
            let priority = flag("--priority").map(str::parse).transpose()?.unwrap_or(2);
            let r = ops::add_task(
                &projects,
                positional(1)?,
                positional(2)?,
                flag("--summary").unwrap_or_default(),
                flag("--due"),
                priority,
            )
            .await?;
            println!("added {}", r.path);
        }
        "new-project" => {
            let r = ops::add_project(
                &projects,
                positional(1)?,
                flag("--summary").unwrap_or_default(),
            )
            .await?;
            println!("added {}", r.path);
        }
        "new-note" => {
            let (path, content) = ops::new_note(
                positional(1)?,
                positional(2)?,
                flag("--summary").unwrap_or_default(),
            )?;
            println!(
                "added {}",
                ops::save(&projects, &path, &content, None).await?.path
            );
        }
        "edit" => {
            let path = positional(1)?;
            let base = projects
                .get(path)
                .await?
                .ok_or_else(|| format!("no record {path:?}"))?
                .content;
            let scratch =
                std::env::temp_dir().join(format!("sjel-projects-edit-{}.md", std::process::id()));
            std::fs::write(&scratch, &base)?;
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new(&editor).arg(&scratch).status()?;
            let edited = std::fs::read_to_string(&scratch);
            let _ = std::fs::remove_file(&scratch);
            if !status.success() {
                return Err(format!("{editor} exited with {status}; nothing saved").into());
            }
            let edited = edited?;
            if edited == base {
                println!("unchanged {path}");
            } else {
                ops::save(&projects, path, &edited, Some(&base)).await?;
                println!("saved {path}");
            }
        }
        "done" | "reopen" => {
            let r = ops::set_done(&projects, positional(1)?, command == "done").await?;
            println!("{} {}", if r.done { "done" } else { "open" }, r.path);
        }
        "set" => {
            let value = args.get(3).map(String::as_str).unwrap_or_default();
            let r = ops::set_property(&projects, positional(1)?, positional(2)?, value).await?;
            println!(
                "{}: {}",
                r.path,
                r.fields().get(positional(2)?).map_or("", String::as_str)
            );
        }
        "attach" => {
            let path = positional(1)?;
            let file = positional(2)?;
            let bytes = std::fs::read(file)?;
            match ops::attach(&projects, path, &bytes).await? {
                projects::store::Attached::New => println!("attached {path}"),
                projects::store::Attached::Unchanged => println!("unchanged {path}"),
                projects::store::Attached::Differs => {
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
            println!("{} {path}", if removed { "removed" } else { "no record" });
        }
        "normalize" => {
            let apply = args.iter().any(|a| a == "--apply");
            let report = ops::normalize(&projects, apply).await?;
            for n in &report {
                println!("{}\n  {}", n.path, n.changes.join("\n  "));
            }
            let verb = if apply {
                "normalized"
            } else {
                "would normalize (dry run; add --apply)"
            };
            println!("{verb} {} task(s)", report.len());
        }
        "import" => {
            let report = ops::import(
                &projects,
                &PathBuf::from(positional(1)?),
                flag("--only"),
                flag("--folder"),
            )
            .await?;
            for (path, reason) in &report.skipped {
                println!("skipped {path}: {reason}");
            }
            println!(
                "imported {}, unchanged {}, attachments {} new and {} unchanged, skipped {}",
                report.imported.len(),
                report.unchanged.len(),
                report.attached.len(),
                report.attached_unchanged,
                report.skipped.len()
            );
        }
        "freeze-vault" => {
            let report =
                ops::freeze_vault(&projects, &PathBuf::from(positional(1)?), flag("--only"))
                    .await?;
            for (path, reason) in &report.skipped {
                println!("skipped {path}: {reason}");
            }
            println!(
                "frozen {}, already frozen {}, skipped {}",
                report.frozen.len(),
                report.already.len(),
                report.skipped.len()
            );
        }
        "verify" => {
            let drift = projects.verify().await?;
            for path in &drift.stale {
                println!("stale    {path}");
            }
            for path in &drift.orphaned {
                println!("orphaned {path}");
            }
            if drift.stale.is_empty() && drift.orphaned.is_empty() {
                println!("store and rendered files match");
            } else {
                return Err(
                    "store and rendered files differ; `projects render` repairs stale files".into(),
                );
            }
        }
        "render" => println!("rendered {} records", projects.render_all().await?),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

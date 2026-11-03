//! `sjel-import plan|run|status <folder>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use import::{Ledger, Outcome};

const USAGE: &str = "usage: sjel-import <command> <folder>
  plan <folder> [--list]   sort every file; changes nothing
  run <folder>             hand each file to its owner and record the outcome
  status <folder>          what the ledger holds for this folder
  serve                    the operator API on port 8107";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sjel-import: {error}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> import::Fallible<ExitCode> {
    if args == ["serve"] {
        import::server::serve()?;
        return Ok(ExitCode::SUCCESS);
    }
    let (Some(command), Some(folder)) = (args.first(), args.get(1)) else {
        return Err(USAGE.into());
    };
    let folder = canonical(folder)?;
    match (command.as_str(), &args[2..]) {
        ("plan", rest) if rest.is_empty() || rest == ["--list"] => {
            let entries = import::plan(&folder)?;
            if !rest.is_empty() {
                for entry in &entries {
                    println!("{:<10} {}", entry.route.name(), entry.path.display());
                }
            }
            print_plan(&entries);
            Ok(ExitCode::SUCCESS)
        }
        ("run", []) => {
            let entries = import::plan(&folder)?;
            let ledger = Ledger::open(&sjel_config::database_path())?;
            let documents = import::DocumentsHttp::from_deployment()?;
            let summary = import::run(&entries, &ledger, &documents, |entry, outcome, detail| {
                if matches!(outcome, Some(Outcome::Imported | Outcome::Failed)) {
                    println!(
                        "{:<9} {}  {}",
                        outcome.map_or("", Outcome::name),
                        entry.path.display(),
                        detail.unwrap_or("")
                    );
                }
            })?;
            println!(
                "\n{} file(s) unchanged since the last run",
                summary.unchanged
            );
            for (outcome, count) in &summary.outcomes {
                println!("{count:>7}  {outcome}");
            }
            let failed = summary.outcomes.get("failed").copied().unwrap_or(0);
            Ok(if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        ("status", []) => {
            let ledger = Ledger::open(&sjel_config::database_path())?;
            let status = ledger.status(&folder)?;
            if status.is_empty() {
                println!(
                    "nothing recorded under {}; run `sjel-import run` first",
                    folder.display()
                );
            }
            for (tally, count) in status {
                println!(
                    "{count:>7}  {:<10} {:<9} {}",
                    tally.route,
                    tally.outcome,
                    tally.detail.unwrap_or_default()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

fn canonical(folder: &str) -> import::Fallible<PathBuf> {
    Path::new(folder)
        .canonicalize()
        .map_err(|error| format!("{folder}: {error}").into())
}

fn print_plan(entries: &[import::Entry]) {
    let tally = import::tally(entries);
    println!(
        "{} entr(ies) sorted; nothing was imported or written",
        entries.len()
    );
    for (route, (count, bytes)) in &tally.routes {
        println!("{count:>7}  {route:<10} {:>9.1} MB", *bytes as f64 / 1e6);
    }
    for (reason, count) in &tally.skipped {
        println!("{count:>7}    skip: {reason}");
    }
}

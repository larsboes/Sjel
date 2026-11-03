use std::path::PathBuf;

use web_links::{HistoryDecision, LinksStore, WriteLink};

fn store() -> Result<LinksStore, String> {
    LinksStore::open(&sjel_config::database_path()).map_err(|error| error.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    let parts: Vec<_> = args.iter().map(String::as_str).collect();
    let store = store()?;
    match parts.as_slice() {
        ["list"] => {
            for link in store.list().map_err(|error| error.to_string())? {
                println!(
                    "{}\t{}\t{}{}",
                    if link.pinned { "★" } else { " " },
                    link.group,
                    link.title,
                    link.tenant
                        .map(|tenant| format!(" · {tenant}"))
                        .unwrap_or_default()
                );
                println!("  {}", link.url);
                println!("  id: {}", link.id);
            }
            Ok(())
        }
        ["add", url, rest @ ..] => {
            let mut request = WriteLink {
                url: (*url).into(),
                title: String::new(),
                group: None,
                pinned: false,
            };
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--title" if i + 1 < rest.len() => {
                        request.title = rest[i + 1].into();
                        i += 2;
                    }
                    "--group" if i + 1 < rest.len() => {
                        request.group = Some(rest[i + 1].into());
                        i += 2;
                    }
                    "--pin" => {
                        request.pinned = true;
                        i += 1;
                    }
                    flag => return Err(format!("unknown or incomplete option: {flag}")),
                }
            }
            let link = store.add(&request).map_err(|error| error.to_string())?;
            println!("{}\t{}\t{}", link.id, link.group, link.url);
            Ok(())
        }
        ["remove", id] => {
            if store.remove(id).map_err(|error| error.to_string())? {
                Ok(())
            } else {
                Err("destination not found".into())
            }
        }
        ["history", "scan", ..] => {
            let days = args
                .iter()
                .position(|arg| arg == "--days")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(90);
            let min_visits = args
                .iter()
                .position(|arg| arg == "--min-visits")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or("HOME is not set")?;
            let candidates = web_links::collect_history(&home, days, min_visits)?;
            let staged = store
                .stage(&candidates)
                .map_err(|error| error.to_string())?;
            println!("{staged} new suggestions staged; nothing was saved automatically.");
            Ok(())
        }
        ["history", "candidates"] => {
            for candidate in store.candidates().map_err(|error| error.to_string())? {
                println!(
                    "{}\t{} visits\t{}\t{}",
                    candidate.id, candidate.visits, candidate.title, candidate.url
                );
            }
            Ok(())
        }
        ["history", decision, id] if *decision == "keep" || *decision == "dismiss" => {
            let decision = if *decision == "keep" {
                HistoryDecision::Keep
            } else {
                HistoryDecision::Dismiss
            };
            let result = store
                .decide_candidate(id, decision)
                .map_err(|error| error.to_string())?;
            if !result.found {
                return Err("candidate not found or no longer pending".into());
            }
            println!(
                "{}",
                if decision == HistoryDecision::Keep {
                    "kept"
                } else {
                    "dismissed"
                }
            );
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

const USAGE: &str = "links — local web destinations

  links list
  links add <url> [--title TITLE] [--group GROUP] [--pin]
  links remove <id>
  links history scan [--days 90] [--min-visits 3]
  links history candidates
  links history keep|dismiss <candidate-id>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args.as_slice(), [arg] if arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return;
    }
    if let Err(error) = run(&args) {
        eprintln!("links: {error}");
        std::process::exit(2);
    }
}

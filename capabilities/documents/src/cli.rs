use documents::{open_default, Review, Stored};
use std::os::unix::fs::OpenOptionsExt;

const USAGE: &str = "usage: documents <command>\n  intake <file>                      add original bytes to the inbox\n  inbox <folder> [--limit N] [--dry-run]  latest documents (default 100), local only\n  watch <folder>                     poll all documents every 60 seconds\n  list [--status inbox|filed|archived]\n  search <words...>                  search filenames and local extracted text\n  review <id> <category> [--project NAME]\n  archive <id>\n  export <id> <destination>      copy an original without overwriting\n  restore-index                      reconstruct missing rows from sidecars\n  verify                              hash-check every stored original";

fn main() {
    if let Err(error) = run() {
        eprintln!("documents: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().ok_or(USAGE)?.as_str();
    if command == "serve" && args.len() == 1 {
        return documents::server::serve();
    }
    let positional = |index: usize| {
        args.get(index)
            .filter(|s| !s.starts_with("--"))
            .map(String::as_str)
            .ok_or(USAGE)
    };
    let flag = |name: &str| {
        args.iter()
            .position(|s| s == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    if command == "inbox" {
        return intake_folder(&args[1..]);
    }
    if command == "watch" && args.len() == 2 {
        let folder = std::path::Path::new(positional(1)?);
        let mut plan = documents::inbox::scan(folder, usize::MAX)?;
        let library = open_default()?;
        println!(
            "watching {}; scans every 60 seconds; stop with Ctrl-C",
            folder.display()
        );
        loop {
            if let Err(error) = import_plan(&library, &plan, true) {
                eprintln!("documents: {error}");
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
            plan = documents::inbox::scan(folder, usize::MAX)?;
        }
    }
    let library = open_default()?;
    match command {
        "intake" => {
            let path = std::path::PathBuf::from(positional(1)?);
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("input path has no UTF-8 filename")?;
            let bytes = std::fs::read(&path)?;
            match library.intake(filename, &bytes)? {
                Stored::New(document) => println!(
                    "added {} {} {} bytes [{}]",
                    document.id,
                    document.filename,
                    document.byte_len,
                    document.extractor.as_deref().unwrap_or("no extractor")
                ),
                Stored::Unchanged(document) => {
                    println!("already stored {} {}", document.id, document.filename)
                }
            }
        }
        "list" => {
            let status = flag("--status");
            for document in library.list(status)? {
                let category = document
                    .category
                    .as_deref()
                    .map(str::to_string)
                    .or_else(|| {
                        document
                            .suggested_category
                            .as_ref()
                            .map(|value| format!("?{value}"))
                    })
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{}  {:8}  {:10}  {:8}  {}{}",
                    document.id,
                    document.status,
                    category,
                    document.byte_len,
                    document.filename,
                    document
                        .project
                        .as_deref()
                        .map(|p| format!("  [{p}]"))
                        .unwrap_or_default()
                );
            }
        }
        "search" => {
            let query = args
                .iter()
                .skip(1)
                .filter(|arg| !arg.starts_with("--"))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if query.is_empty() {
                return Err(USAGE.into());
            }
            for document in library.list(None)?.into_iter().filter(|document| {
                document.filename.to_lowercase().contains(&query)
                    || document
                        .extracted_title
                        .as_deref()
                        .is_some_and(|title| title.to_lowercase().contains(&query))
                    || document
                        .extracted_text
                        .as_deref()
                        .is_some_and(|text| text.to_lowercase().contains(&query))
            }) {
                println!(
                    "{}  {}  {}{}",
                    document.id,
                    document.status,
                    document.filename,
                    document
                        .category
                        .as_deref()
                        .map(|category| format!("  [{category}]"))
                        .unwrap_or_default()
                );
            }
        }
        "review" => {
            let review = Review {
                category: positional(2)?.to_string(),
                project: flag("--project").map(str::to_string),
            };
            let document = library.review(positional(1)?, &review)?;
            println!(
                "filed {} as {}{}",
                document.id,
                document.category.as_deref().unwrap_or(""),
                document
                    .project
                    .as_deref()
                    .map(|p| format!(" [{p}]"))
                    .unwrap_or_default()
            );
        }
        "archive" => {
            let document = library.archive(positional(1)?)?;
            println!("archived {} (original retained)", document.id);
        }
        "export" => {
            let id = positional(1)?;
            let destination = std::path::PathBuf::from(positional(2)?);
            let bytes = library.bytes(id)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&destination)?;
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()?;
            println!("exported {id} to {}", destination.display());
        }
        "restore-index" => {
            let (restored, unchanged) = library.restore_index()?;
            println!("restored {restored} metadata row(s); {unchanged} already matched");
        }
        "verify" => {
            let issues = library.verify()?;
            if issues.is_empty() {
                println!("all stored originals match their SHA-256 digests");
            } else {
                for id in &issues {
                    println!("missing or corrupt {id}");
                }
                return Err(format!("{} document(s) need repair", issues.len()).into());
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn intake_folder(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let folder = args.first().filter(|s| !s.starts_with('-')).ok_or(USAGE)?;
    let mut limit = 100;
    let mut dry_run = false;
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--dry-run" if !dry_run => dry_run = true,
            "--limit" => limit = options.next().ok_or(USAGE)?.parse::<usize>()?,
            _ => return Err(USAGE.into()),
        }
    }
    let plan = documents::inbox::scan(std::path::Path::new(folder), limit)?;
    println!("selected {} eligible document(s); {} excluded by filename; {} unready (empty, oversized, or modified within 30 seconds)",
        plan.files.len(), plan.excluded, plan.unready);
    if dry_run {
        for file in &plan.files {
            println!("would import {}", file.filename);
        }
        return Ok(());
    }
    let library = open_default()?;
    import_plan(&library, &plan, false)
}

fn import_plan(
    library: &documents::Library,
    plan: &documents::inbox::Plan,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut added, mut unchanged, mut failed) = (0, 0, 0);
    for file in &plan.files {
        match file
            .read()
            .and_then(|bytes| library.intake(&file.filename, &bytes))
        {
            Ok(stored) => {
                let document = match stored {
                    Stored::New(document) => {
                        added += 1;
                        document
                    }
                    Stored::Unchanged(document) => {
                        unchanged += 1;
                        if quiet {
                            continue;
                        }
                        document
                    }
                };
                println!(
                    "{}  {}  {}  {}  [{}]",
                    document.id,
                    document.status,
                    document
                        .suggested_category
                        .as_deref()
                        .unwrap_or("unclassified"),
                    document.filename,
                    document.extractor.as_deref().unwrap_or("no extracted text")
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!("could not import {}: {error}", file.filename);
            }
        }
    }
    if !quiet || added > 0 || failed > 0 {
        println!(
            "{added} added; {unchanged} already stored; {failed} failed; source files retained"
        );
    }
    if failed > 0 {
        return Err("inbox intake incomplete; retry is idempotent".into());
    }
    Ok(())
}

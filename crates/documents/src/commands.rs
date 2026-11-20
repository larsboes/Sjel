//! Local Documents commands shared by the CLI and the protected owner socket.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

use crate::{Library, Review, Stored};

macro_rules! line {
    ($out:expr, $($arg:tt)*) => {
        writeln!($out, $($arg)*)?
    };
}

pub const USAGE: &str = "usage: documents <command>\n  intake <file>                      add original bytes to the inbox\n  inbox <folder> [--limit N] [--dry-run] [--recursive] [--include-sensitive]\n                                     latest documents (default 100), local only\n  duplicates                         copies, rescans and split parts; changes nothing\n  merge-copies [--apply]             keep one record per exact-copy group, archive the rest\n  collection list|show <name>|new <name> [--category C]|add <name> <id>...|remove <name> <id>...\n  watch <folder>                     poll all documents every 60 seconds\n  list [--status inbox|filed|archived]\n  next [--limit N] [--offset N]       show local review steps (default 100)\n  search <words...> [--status STATE] [--limit N] ranked local search; show matching pages\n  analyze <id>                       refresh page-level extraction locally\n  suggest-splits <id>                suggest explicit page-one boundaries\n  suggest-model <id>                 ask the configured local model for a review proposal\n  pages <id>                         show per-page extraction status without page text\n  split <id> <ranges>                store child PDFs in Sjel, e.g. 1-2,3-4\n  review <id> <category> [--project NAME]\n  archive <id>\n  export <id> <destination>      copy a stored PDF without overwriting\n  restore-index                      reconstruct missing rows from sidecars\n  verify                              hash-check every stored document";

/// Run a local command using the owning library.
///
/// # Errors
/// Invalid arguments, document operations or output were rejected.
pub fn run(
    library: &Library,
    args: &[String],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let command = args.first().ok_or(USAGE)?.as_str();
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
        return intake_folder(library, &args[1..], out);
    }
    if command == "watch-scan" && args.len() == 2 {
        let plan = crate::inbox::scan(std::path::Path::new(positional(1)?), usize::MAX)?;
        return import_plan(library, &plan, true, out);
    }
    match command {
        "intake" => {
            let path = std::path::PathBuf::from(positional(1)?);
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("input path has no UTF-8 filename")?;
            let bytes = std::fs::read(&path)?;
            match library.intake(filename, &bytes)? {
                Stored::New(document) => line!(
                    out,
                    "added {} {} {} bytes [{}]",
                    document.id,
                    document.filename,
                    document.byte_len,
                    document.extractor.as_deref().unwrap_or("no extractor")
                ),
                Stored::Unchanged(document) => {
                    line!(out, "already stored {} {}", document.id, document.filename)
                }
            }
        }
        "list" => {
            let status = flag("--status");
            let documents = library.list(status)?;
            let ids = documents
                .iter()
                .map(|document| document.id.clone())
                .collect::<Vec<_>>();
            let pending = library.pending_proposal_documents(&ids)?;
            let parents_with_children = library.documents_with_children(&ids)?;
            for document in documents {
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
                line!(
                    out,
                    "{}  {:8}  {:16}  {:10}  {:8}  {}{}{}",
                    document.id,
                    document.status,
                    crate::workflow::stage(
                        &document,
                        pending.contains(&document.id),
                        parents_with_children.contains(&document.id),
                    )
                    .label(),
                    category,
                    document.byte_len,
                    document.filename,
                    document
                        .project
                        .as_deref()
                        .map(|p| format!("  [{p}]"))
                        .unwrap_or_default(),
                    document
                        .page_range
                        .map(|range| format!(
                            "  ← {} pages {}-{}",
                            document.parent_id.as_deref().unwrap_or("?"),
                            range.first,
                            range.last
                        ))
                        .unwrap_or_default()
                );
            }
        }
        "next" => {
            let limit = flag("--limit").unwrap_or("100").parse::<usize>()?;
            if !(1..=500).contains(&limit) {
                return Err("next limit must be 1..=500".into());
            }
            let offset = flag("--offset").unwrap_or("0").parse::<usize>()?;
            let (documents, total) = library.list_page(Some("inbox"), offset, limit)?;
            let ids = documents
                .iter()
                .map(|document| document.id.clone())
                .collect::<Vec<_>>();
            let pending = library.pending_proposal_documents(&ids)?;
            let parents_with_children = library.documents_with_children(&ids)?;
            let shown = documents.len();
            for document in documents {
                let stage = crate::workflow::stage(
                    &document,
                    pending.contains(&document.id),
                    parents_with_children.contains(&document.id),
                );
                if matches!(
                    stage,
                    crate::workflow::Stage::Analyze
                        | crate::workflow::Stage::ReviewSplit
                        | crate::workflow::Stage::ReviewEnrichment
                        | crate::workflow::Stage::File
                ) {
                    line!(
                        out,
                        "{}  {:16}  {}{}",
                        stage.label(),
                        document.id,
                        document.filename,
                        document
                            .suggested_category
                            .as_deref()
                            .map(|category| format!("  ?{category}"))
                            .unwrap_or_default()
                    );
                }
            }
            line!(
                out,
                "showing {shown} of {total} inbox record(s) from offset {offset}"
            );
        }
        "search" => {
            let mut query_words = Vec::new();
            let mut index = 1;
            while let Some(argument) = args.get(index) {
                if argument == "--status" || argument == "--limit" {
                    index += 2;
                } else if argument.starts_with("--") {
                    index += 1;
                } else {
                    query_words.push(argument.clone());
                    index += 1;
                }
            }
            let query = query_words.join(" ");
            if query.is_empty() {
                return Err(USAGE.into());
            }
            let limit = flag("--limit").unwrap_or("50").parse::<usize>()?;
            if !(1..=500).contains(&limit) {
                return Err("search limit must be 1..=500".into());
            }
            let page = library.search(&query, flag("--status"), 0, limit)?;
            let ids = page
                .documents
                .iter()
                .map(|document| document.id.clone())
                .collect::<Vec<_>>();
            let pending = library.pending_proposal_documents(&ids)?;
            let parents_with_children = library.documents_with_children(&ids)?;
            for (document, matched_pages) in page.documents.iter().zip(page.matched_pages) {
                line!(
                    out,
                    "{}  {}  {}  {}{}{}",
                    document.id,
                    document.status,
                    crate::workflow::stage(
                        document,
                        pending.contains(&document.id),
                        parents_with_children.contains(&document.id),
                    )
                    .label(),
                    document.filename,
                    document
                        .category
                        .as_deref()
                        .map(|category| format!("  [{category}]"))
                        .unwrap_or_default(),
                    if matched_pages.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "  pages {}",
                            matched_pages
                                .iter()
                                .map(u32::to_string)
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                );
            }
            line!(
                out,
                "showing {} of {} matching document(s)",
                page.documents.len(),
                page.total
            );
        }
        "review" => {
            let review = Review {
                category: positional(2)?.to_string(),
                project: flag("--project").map(str::to_string),
            };
            let document = library.review(positional(1)?, &review)?;
            line!(
                out,
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
        "analyze" => {
            let document = library.analyze(positional(1)?)?;
            line!(
                out,
                "analyzed {}: {} page(s), {} extracted characters [{}]",
                document.id,
                document.pages.len(),
                document.extracted_text.as_deref().map_or(0, |text| {
                    text.chars()
                        .filter(|character| !character.is_whitespace())
                        .count()
                }),
                document.extractor.as_deref().unwrap_or("no extractor")
            );
        }
        "suggest-splits" => {
            let suggestion = library.split_suggestions(positional(1)?)?;
            if let Some(reason) = suggestion.abstention {
                line!(out, "no split suggestion: {reason}");
            } else {
                let ranges = suggestion
                    .ranges
                    .iter()
                    .map(|range| format!("{}-{}", range.first, range.last))
                    .collect::<Vec<_>>()
                    .join(", ");
                line!(out, "review suggested ranges: {ranges}");
                for boundary in suggestion.boundaries {
                    line!(
                        out,
                        "cut before page {} · signal {}",
                        boundary.before_page,
                        boundary.marker
                    );
                }
                line!(out, "suggestions never split or file documents");
            }
        }
        "suggest-model" => {
            for record in library.generate_model_proposal(positional(1)?)? {
                line!(out, "{}  {:?}", record.id, record.proposal.outcome);
            }
        }
        "pages" => {
            let document = library
                .get(positional(1)?)?
                .ok_or("no document with that id")?;
            if document.pages.is_empty() {
                line!(
                    out,
                    "no page evidence; run documents analyze {}",
                    document.id
                );
            }
            for page in document.pages {
                let failure = page
                    .failure
                    .map(|_| " · extraction needs review")
                    .unwrap_or_default();
                line!(
                    out,
                    "page {} · {} · {} characters{}",
                    page.page_number,
                    page.engine,
                    page.text.chars().count(),
                    failure
                );
            }
        }
        "split" => {
            let id = positional(1)?;
            let parent = library.get(id)?.ok_or("no document with that id")?;
            let ranges = parse_ranges(positional(2)?)?;
            let stem = std::path::Path::new(&parent.filename)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Document");
            let request = crate::split::SplitRequest {
                page_count: u32::try_from(parent.pages.len())?,
                children: ranges
                    .into_iter()
                    .map(|(first, last)| crate::split::ChildInput {
                        first,
                        last,
                        filename: format!("{stem} pages {first}-{last}.pdf"),
                    })
                    .collect(),
            };
            for child in library.split(id, &request)? {
                if let Some(range) = child.page_range {
                    line!(
                        out,
                        "created {}  {}  pages {}-{}",
                        child.id,
                        child.filename,
                        range.first,
                        range.last
                    );
                }
            }
        }
        "archive" => {
            let document = library.archive(positional(1)?)?;
            line!(out, "archived {} (original retained)", document.id);
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
            file.write_all(&bytes)?;
            file.sync_all()?;
            line!(out, "exported {id} to {}", destination.display());
        }
        "collection" => collection(library, &args[1..], out)?,
        "merge-copies" => {
            let apply = match args.get(1).map(String::as_str) {
                None => false,
                Some("--apply") if args.len() == 2 => true,
                _ => return Err(USAGE.into()),
            };
            merge_copies(library, apply, out)?;
        }
        "duplicates" => {
            let documents = library.list(None)?;
            let report = crate::duplicates::report(&documents, &library.sources()?);
            let name = |id: &str| {
                documents
                    .iter()
                    .find(|d| d.id == id)
                    .map_or("?", |d| d.filename.as_str())
                    .to_string()
            };
            line!(out, "{} exact copy group(s)", report.copies.len());
            for group in &report.copies {
                line!(out, "  sha256 {}", &group.sha256[..12]);
                for id in &group.ids {
                    line!(out, "    record {}  {}", &id[..12], name(id));
                }
                for path in &group.paths {
                    line!(out, "    found at {path}");
                }
            }
            line!(
                out,
                "{} probable rescan pair(s) — review",
                report.rescans.len()
            );
            for pair in &report.rescans {
                line!(
                    out,
                    "  {:.2}  {}  ≈  {}",
                    pair.score,
                    name(&pair.a),
                    name(&pair.b)
                );
                line!(out, "        {}  {}", &pair.a[..12], &pair.b[..12]);
            }
            line!(
                out,
                "{} probable split part(s) — review",
                report.parts.len()
            );
            for pair in &report.parts {
                line!(
                    out,
                    "  {:.2}  {}  ⊂  {}",
                    pair.score,
                    name(&pair.a),
                    name(&pair.b)
                );
                line!(out, "        {}  {}", &pair.a[..12], &pair.b[..12]);
            }
        }
        "restore-index" => {
            let (restored, unchanged) = library.restore_index()?;
            line!(
                out,
                "restored {restored} metadata row(s); {unchanged} already matched"
            );
        }
        "verify" => {
            let issues = library.verify()?;
            if issues.is_empty() {
                line!(out, "all stored originals match their SHA-256 digests");
            } else {
                for id in &issues {
                    line!(out, "missing or corrupt {id}");
                }
                return Err(format!("{} document(s) need repair", issues.len()).into());
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
fn parse_ranges(value: &str) -> Result<Vec<(u32, u32)>, Box<dyn std::error::Error>> {
    let mut ranges = Vec::new();
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (first, last) = item
            .split_once('-')
            .ok_or("ranges must use inclusive form such as 1-2,3-4")?;
        ranges.push((first.parse()?, last.parse()?));
    }
    if ranges.is_empty() {
        return Err("enter every child range, such as 1-2,3-4".into());
    }
    Ok(ranges)
}

/// `collection list | show <name> | new <name> [--category C] | add|remove <name> <id>...`
fn collection(
    library: &Library,
    args: &[String],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = || args.get(1).map(String::as_str).ok_or(USAGE);
    match args.first().map(String::as_str) {
        Some("list") if args.len() == 1 => {
            for collection in library.collections()? {
                line!(
                    out,
                    "{}  {:3} member(s)  {}  [{}]",
                    collection.id,
                    collection.member_ids.len(),
                    collection.name,
                    collection.category.as_deref().unwrap_or("-")
                );
            }
        }
        Some("show") if args.len() == 2 => {
            let collection = library.collection(name()?)?.ok_or("no such collection")?;
            line!(
                out,
                "{} [{}]",
                collection.name,
                collection.category.as_deref().unwrap_or("-")
            );
            for id in &collection.member_ids {
                let filename = library.get(id)?.map(|d| d.filename).unwrap_or_default();
                line!(out, "{id}  {filename}");
            }
        }
        Some("new") => {
            let category = match &args[2..] {
                [] => None,
                [flag, category] if flag == "--category" => Some(category.as_str()),
                _ => return Err(USAGE.into()),
            };
            let collection = library.create_collection(name()?, category)?;
            line!(
                out,
                "created collection {} {}",
                collection.id,
                collection.name
            );
        }
        Some(verb @ ("add" | "remove")) if args.len() > 2 => {
            let ids = &args[2..];
            let (add, remove): (&[String], &[String]) = if verb == "add" {
                (ids, &[])
            } else {
                (&[], ids)
            };
            let collection = library.update_collection(name()?, add, remove)?;
            line!(
                out,
                "{} now has {} member(s)",
                collection.name,
                collection.member_ids.len()
            );
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// Confirm every open exact-copy group with the preferred record as survivor.
fn merge_copies(
    library: &Library,
    apply: bool,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let documents = library.list(None)?;
    let (mut groups, mut archived) = (0, 0);
    for candidate in crate::duplicates::open_candidates(library)? {
        if candidate.kind != crate::review::CandidateKind::ExactCopy {
            continue;
        }
        let members = documents
            .iter()
            .filter(|document| candidate.member_ids.contains(&document.id));
        let survivor =
            crate::duplicates::preferred(members).ok_or("exact-copy group has no stored member")?;
        groups += 1;
        if apply {
            let decision = crate::review::DuplicateDecision {
                candidate_key: candidate.candidate_key,
                kind: candidate.kind,
                member_ids: candidate.member_ids,
                action: crate::review::DecisionAction::ConfirmGroup {
                    survivor_id: survivor.id.clone(),
                },
            };
            archived += library.save_duplicate_decision(&decision)?.len();
        } else {
            archived += candidate.member_ids.len() - 1;
        }
    }
    let verb = if apply { "archived" } else { "would archive" };
    line!(
        out,
        "{groups} exact-copy group(s); {verb} {archived} record(s); originals kept"
    );
    Ok(())
}

fn intake_folder(
    library: &Library,
    args: &[String],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let folder = args.first().filter(|s| !s.starts_with('-')).ok_or(USAGE)?;
    let mut limit = 100;
    let (mut dry_run, mut recursive, mut include_sensitive) = (false, false, false);
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--dry-run" if !dry_run => dry_run = true,
            "--recursive" if !recursive => recursive = true,
            "--include-sensitive" if !include_sensitive => include_sensitive = true,
            "--limit" => limit = options.next().ok_or(USAGE)?.parse::<usize>()?,
            _ => return Err(USAGE.into()),
        }
    }
    let plan = crate::inbox::scan_with(
        std::path::Path::new(folder),
        limit,
        recursive,
        include_sensitive,
    )?;
    line!(out,
        "selected {} eligible document(s); {} excluded by filename; {} unready (empty, oversized, or modified within 30 seconds)",
        plan.files.len(),
        plan.excluded,
        plan.unready
    );
    if dry_run {
        let (mut known, mut new, mut unreadable) = (0, 0, 0);
        for file in &plan.files {
            let Ok(bytes) = file.read() else {
                unreadable += 1;
                continue;
            };
            let sha256 = crate::library::hex(&<sha2::Sha256 as sha2::Digest>::digest(&bytes));
            if library.roots_by_hash(&sha256)?.is_empty() {
                new += 1;
                line!(out, "would import {}", file.path().display());
            } else {
                known += 1;
            }
        }
        line!(
            out,
            "{new} new, {known} already stored, {unreadable} unreadable"
        );
        return Ok(());
    }
    import_plan(library, &plan, false, out)
}

fn import_plan(
    library: &Library,
    plan: &crate::inbox::Plan,
    quiet: bool,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut added, mut unchanged, mut failed) = (0, 0, 0);
    for file in &plan.files {
        match file
            .read()
            .and_then(|bytes| library.intake_known(&file.filename, &bytes))
        {
            Ok(stored) => {
                let id = match &stored {
                    Stored::New(document) | Stored::Unchanged(document) => &document.id,
                };
                if let Err(error) = library.record_source(id, file.path()) {
                    eprintln!(
                        "could not record where {} was found: {error}",
                        file.filename
                    );
                }
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
                line!(
                    out,
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
        line!(
            out,
            "{added} added; {unchanged} already stored; {failed} failed; source files retained"
        );
    }
    if failed > 0 {
        return Err("inbox intake incomplete; retry is idempotent".into());
    }
    Ok(())
}

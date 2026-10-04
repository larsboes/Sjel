//! `comms` CLI. Manual arg parsing (no clap), mirroring scouting's main.rs.
//!
//! Subcommands:
//!   comms sweep [--limit N=25] [--dry-run]   read-only inbox triage proposals
//!   comms ingest <url>                        media/article ingest -> feed
//!   comms feed [--stream S] [--days N=7] [--include-dismissed]
//!   comms keep <id> | dismiss <id>            set feed item status
//!   comms summarize --pending                 retry missing summaries
//!   comms export-sources [--dry-run]          reconcile the feed library with the vault
//!   comms mail classify [--shadow|--apply]    the local model classification rung
//!   comms --help
//!
//! `sweep` is strictly read-only against Gmail either way -- `--dry-run` only
//! controls whether proposals are persisted to the store. Gmail writes are
//! available only through authenticated, explicit server actions.

use std::collections::BTreeMap;

use comms::config::Config;
use comms::mail_events;
use comms::mail_model::{self, Mode};
use comms::store::Store;
use comms::{google, intake, media, normalize};

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
}

fn open_store(cfg: &Config) -> Store {
    Store::open(&cfg.database_path).unwrap_or_else(|e| {
        eprintln!(
            "error: could not open store at {}: {e}",
            cfg.database_path.display()
        );
        std::process::exit(1);
    })
}

fn print_help() {
    println!("comms — read-only Gmail triage + share-link media ingest\n");
    println!("usage: comms <command> [flags]\n");
    println!("  sweep [--limit N] [--dry-run]   list inbox threads (READ-ONLY), classify into");
    println!(
        "                                  streams, print proposals; persists unless --dry-run"
    );
    println!("                                  (default --limit 25)");
    println!("  ingest <url>                    ingest a YouTube/Instagram/podcast/article URL");
    println!("  feed [--stream news|media]      list stored feed items grouped by day");
    println!("       [--days N] [--include-dismissed]   (default --days 7)");
    println!("  keep <id>                       feed item -> 'keeper' (+ export if configured);");
    println!("                                  mail -> a distilled note in keeper_export_dir.");
    println!("                                  Never a Gmail write: archiving stays explicit.");
    println!("  dismiss <id>                    mark a feed item or mail 'dismissed' (local only)");
    println!("  summarize --pending             summarize feed items that still lack a summary");
    println!("  normalize --explain             print the normalization rules and what each drops");
    println!("  normalize --all                 re-run normalization over stored raw content");
    println!("  export-sources [--dry-run]      reconcile every saved feed item with");
    println!("                                  Resources/Sources/ in the configured vault");
    println!("  reclassify-feed --rationale <t> re-derive `legacy` feed data classes from the");
    println!("       [--dry-run]                CURRENT feed source declarations. Lowering a");
    println!("                                  class is a human act, so the rationale is");
    println!("                                  required and is stored on every row it changes.");
    println!("  mail classify [--shadow]        run the local model rung over the mail the");
    println!("       [--apply] [--limit N]      deterministic rules did not decide. Shadow by");
    println!("       [--report]                 default: it writes verdicts and moves no");
    println!("       [--revert <id>]            category. --apply needs mail_model.apply and a");
    println!("       [--revert-all]             non-zero min_confidence_bp in the overlay, and");
    println!("                                  never raises a data class. It writes the stored");
    println!("                                  shadow verdicts rather than asking again.");
    println!("                                  --limit defaults to mail_model.limit, else 200.");
    println!("  mail corpus --out <path>        write the labelling skeleton for the frozen");
    println!("       [--force]                  mail-classification corpus: one fixture per");
    println!("                                  fallback row, `label` and `urgency_band` EMPTY.");
    println!("                                  Fill both by hand BEFORE reading any model");
    println!("                                  output. Refuses to overwrite without --force.");
    println!("  digest corpus --out <path>      write the judgement skeleton for the frozen");
    println!("       [--per-producer N]         digest-quality corpus: N generated digests per");
    println!("       [--force]                  rung, `faithful` and `useful_band` EMPTY. The");
    println!("                                  gate is the unfaithful rate; usefulness is");
    println!("                                  reported and does not gate (PRD D16).");
    println!("  relevance backfill              re-score stored feed items through the running");
    println!("       [--days N=3650]            server, page by page, until every item in the");
    println!("       [--batch N=100] [--max N]  window has been seen. Drains rows that were");
    println!("       [--force]                  written lexical while the embedding role was");
    println!("                                  down. Only the full 3650-day window can mark");
    println!("                                  the corpus complete. Needs comms-server up: it");
    println!("                                  is an HTTP client, not a second opener of the");
    println!("                                  database.");
    println!("  egress-log [--limit N=25]       show outbound cloud model calls with token counts");
    println!("       [--audit]                  and costs, or audit egress payloads for C2 leaks");
    println!("  --help, -h                      show this help");
    println!("\nThis CLI's Gmail sweep is READ-ONLY. Archive, Trash and the Waiting label require an explicit authenticated dashboard action.");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let cfg = Config::load();
    let command = args[1].as_str();

    match command {
        "sweep" => cmd_sweep(&args, &cfg),
        "ingest" => cmd_ingest(&args, &cfg),
        "feed" => cmd_feed(&args, &cfg),
        "keep" => cmd_set_status(&args, &cfg, "keeper"),
        "dismiss" => cmd_set_status(&args, &cfg, "dismissed"),
        "summarize" => cmd_summarize(&args, &cfg),
        "normalize" => cmd_normalize(&args, &cfg),
        "export-sources" => cmd_export_sources(&args, &cfg),
        "reclassify-feed" => cmd_reclassify_feed(&args, &cfg),
        "mail" => cmd_mail(&args, &cfg),
        "digest" => cmd_digest(&args, &cfg),
        "relevance" => cmd_relevance(&args, &cfg),
        "egress-log" => cmd_egress_log(&args, &cfg),
        other => {
            eprintln!("error: unknown command '{other}'\n");
            print_help();
            std::process::exit(1);
        }
    }
}

// -- sweep ---------------------------------------------------------------

fn cmd_sweep(args: &[String], cfg: &Config) {
    let limit: usize = arg_after(args, "--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let token = match google::access_token(&cfg.google_env_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: could not obtain Gmail access token: {e}");
            eprintln!("       (expected creds in {:?})", cfg.google_env_path);
            std::process::exit(1);
        }
    };

    let stubs = match google::list_inbox_threads(&token, limit) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not list inbox threads: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "comms sweep — {} inbox threads (READ-ONLY){}\n",
        stubs.len(),
        if dry_run { ", dry-run" } else { "" }
    );

    let store = if dry_run { None } else { Some(open_store(cfg)) };

    // stream -> Vec<(from, subject, rationale)>
    let mut grouped: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    let mut total = 0usize;
    let mut persisted_new = 0usize;
    let mut fetched_ids = Vec::new();
    let mut redacted = 0usize;

    for stub in &stubs {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("  warning: skipping thread {}: {e}", stub.id);
                continue;
            }
        };
        let id = meta.id.clone();
        fetched_ids.push(id.clone());
        let from = meta.from_addr.clone().unwrap_or_default();
        let intake = intake::from_thread(meta, &cfg.rules);
        total += 1;
        redacted += usize::from(intake.redaction_count() > 0);

        if let Some(st) = &store {
            match st.upsert_triage_with_rules(&intake.item, &intake.verdict()) {
                Ok(true) => persisted_new += 1,
                Ok(false) => {}
                Err(e) => eprintln!("  warning: could not persist {id}: {e}"),
            }
        }

        // The redacted subject, not the swept one: what the terminal prints is
        // as much an output surface as the database is.
        let subject = intake.item.subject.clone().unwrap_or_default();
        grouped
            .entry(intake.item.stream.clone())
            .or_default()
            .push((from, subject, intake.item.rationale.clone()));
    }

    for (stream, items) in &grouped {
        println!("── {} ({}) ──", stream, items.len());
        for (from, subject, rationale) in items {
            println!("  {} | {}", truncate(from, 40), truncate(subject, 60));
            println!("      {rationale}");
        }
        println!();
    }

    let should_scan_events = store.is_some();
    drop(store);
    let events = should_scan_events.then(|| mail_events::analyze_batch(cfg, &fetched_ids));

    println!("total: {total} threads across {} streams", grouped.len());
    if redacted > 0 {
        println!("redacted: {redacted} c2/c3 thread(s) — subject and snippet stored with markers");
    }
    if let Some(events) = events {
        println!("persisted: {persisted_new} new proposals; {} Calendar event proposal(s), {} event analysis issue(s)", events.proposals, events.failed);
    } else {
        println!("dry-run: nothing persisted");
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

// -- ingest --------------------------------------------------------------

fn cmd_ingest(args: &[String], cfg: &Config) {
    let url = match args.get(2) {
        Some(u) if !u.starts_with("--") => u,
        _ => {
            eprintln!("usage: comms ingest <url>");
            std::process::exit(1);
        }
    };

    let item = match media::ingest(url, cfg) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: ingest failed: {e}");
            std::process::exit(1);
        }
    };

    let store = open_store(cfg);
    if let Err(e) = store.upsert_feed(&item) {
        eprintln!("error: could not persist feed item: {e}");
        std::process::exit(1);
    }
    // Read back the stored row for accurate day/created_at/status.
    let stored = store.get_feed(&item.id).ok().flatten().unwrap_or(item);

    println!("ingested:");
    println!("  id      : {}", stored.id);
    println!("  kind    : {} ({})", stored.kind, stored.stream);
    println!(
        "  title   : {}",
        stored.title.as_deref().unwrap_or("(none)")
    );
    match &stored.summary {
        Some(s) => println!("  summary :\n{}", indent(s, 4)),
        None => println!("  summary : summary pending"),
    }
}

fn indent(s: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    s.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- feed ----------------------------------------------------------------

fn cmd_feed(args: &[String], cfg: &Config) {
    let stream = arg_after(args, "--stream").map(|s| s.as_str());
    let days: i32 = arg_after(args, "--days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let include_dismissed = args.iter().any(|a| a == "--include-dismissed");

    let store = open_store(cfg);
    let items = match store.list_feed(stream, None, days, include_dismissed) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: could not list feed: {e}");
            std::process::exit(1);
        }
    };

    if items.is_empty() {
        println!("comms feed — no items in the last {days} days");
        return;
    }

    println!(
        "comms feed — {} items (last {days} days){}\n",
        items.len(),
        stream.map(|s| format!(", stream={s}")).unwrap_or_default()
    );

    // Grouped by day (list_feed already orders by created_at DESC).
    let mut current_day = String::new();
    for item in &items {
        if item.day != current_day {
            current_day = item.day.clone();
            println!("── {current_day} ──");
        }
        let title = item.title.as_deref().unwrap_or("(untitled)");
        println!(
            "  [{}] {} · {}",
            item.kind,
            truncate(title, 70),
            item.status
        );
        println!("      {}", item.url);
        if let Some(s) = &item.summary {
            println!("      {}", truncate(&s.replace('\n', " "), 100));
        }
        println!("      id: {}", item.id);
    }
}

// -- keep / dismiss ------------------------------------------------------

fn cmd_set_status(args: &[String], cfg: &Config, status: &str) {
    let id = match args.get(2) {
        Some(i) if !i.starts_with("--") => i,
        _ => {
            eprintln!(
                "usage: comms {} <id>",
                if status == "keeper" {
                    "keep"
                } else {
                    "dismiss"
                }
            );
            std::process::exit(1);
        }
    };

    let store = open_store(cfg);
    // `keep` names a thing, not a table. The feed first, since that is where most ids come from,
    // then mail — which had no keep path at all, and so had no way out of the inbox except
    // staying in it. That is the outcome the comms doctrine exists to prevent
    // — the Information lane of the comms doctrine: a kept mail becomes a distilled statement in
    // the system that owns it, never a second copy of the mail.
    match store.set_feed_status(id, status, "cli") {
        Ok(true) => {
            println!("{id} -> {status}");
            if status == "keeper" {
                if let Some(dir) = &cfg.keeper_export_dir {
                    match store.get_feed(id) {
                        Ok(Some(item)) => match export_keeper(&item, dir) {
                            Ok(path) => println!("exported: {}", path.display()),
                            Err(e) => eprintln!("warning: keeper export failed: {e}"),
                        },
                        _ => eprintln!("warning: could not re-read item for export"),
                    }
                }
            }
        }
        Ok(false) => keep_mail(&store, cfg, id, status),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// The mail half of `keep` / `dismiss`, reached when the id is not a feed item.
///
/// Keeping a mail writes the distilled note and changes nothing in Gmail. Not an oversight:
/// archiving is a mutation the doctrine permits only on explicit approval, and folding it into
/// "the information has been extracted" would archive as a side effect. The two are printed as
/// what they are — one done, one still yours to ask for.
fn keep_mail(store: &Store, cfg: &Config, id: &str, status: &str) {
    let item = match store.get_triage(id) {
        Ok(Some(item)) => item,
        Ok(None) => {
            eprintln!("error: no feed item or mail with id '{id}'");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    if status != "keeper" {
        // `dismiss` on a mail is a local status and never a Gmail write — the same word meaning
        // the same thing on both sides of the store.
        match store.set_triage_status(id, "dismissed") {
            Ok(_) => println!("{id} -> dismissed (mail; Gmail untouched)"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let Some(dir) = &cfg.keeper_export_dir else {
        eprintln!(
            "error: keeping a mail means writing it somewhere, and keeper_export_dir is not set."
        );
        eprintln!("       Set it in the overlay's comms.json (see comms.config.example.json).");
        std::process::exit(1);
    };
    match export_mail_keeper(&item, dir) {
        Ok(path) => {
            println!("exported: {}", path.display());
            println!(
                "note: the mail is still in the Inbox — archiving is a separate, explicit action."
            );
        }
        Err(e) => {
            eprintln!("error: mail export failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Write the distilled statement a kept mail leaves behind: what it was, where to find it, and
/// why it was classified as it was. Refuses to overwrite, like its feed sibling.
///
/// What is deliberately NOT in here is the mail. No snippet, no body, no re-fetch — the snippet
/// is the first couple of hundred characters of the message, which is exactly the raw mail this
/// lane exists to avoid keeping a copy of. Subject, sender and date are carried because they are
/// what makes the note findable. Every one comes from the STORED row, so for a c2 or c3 mail they are the
/// redacted form the intake gate produced, and nothing here can reconstruct what it removed.
fn export_mail_keeper(
    item: &comms::store::TriageItem,
    dir: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let subject = item.subject.as_deref().unwrap_or("(no subject)");
    // The stored TIMESTAMPTZ, cut at the date. `internal_date_text` is the read-side field;
    // `internal_date_ms` is write-side only and is None here.
    let day = item
        .internal_date_text
        .as_deref()
        .and_then(|stamp| stamp.split(' ').next())
        .filter(|day| !day.is_empty())
        .unwrap_or("undated");
    let path = dir.join(format!("{day}-mail-{}.md", slug(subject)));
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists — refusing to overwrite", path.display()),
        ));
    }

    // The one link back: the Gmail permalink, so the note and the thread it distils point at
    // the same place. This was also the shape the retired `tasks` capability keyed a promoted
    // mail on (PRD Q48), which is why it is a derived permalink rather than a stored column.
    let permalink = format!("https://mail.google.com/mail/u/0/#all/{}", item.id);
    let mut body = format!("# {subject}\n\n");
    if let Some(from) = &item.from_addr {
        body.push_str(&format!("- From: {from}\n"));
    }
    body.push_str(&format!("- Date: {day}\n"));
    body.push_str(&format!("- Gmail: {permalink}\n"));
    body.push_str(&format!("- Stream: {}\n", item.stream));
    // The class travels with the content instead of being re-derived at the destination:
    // re-deriving it from a redacted note would classify the redaction, not the mail.
    body.push_str(&format!("- Class: {}\n", item.data_class));
    body.push_str(&format!("\n## Why this was kept\n\n{}\n", item.rationale));
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Write a distilled keeper note (title, url, date, summary — NOT the raw
/// transcript). Refuses to overwrite an existing file.
fn export_keeper(
    item: &comms::store::FeedItem,
    dir: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let title = item.title.as_deref().unwrap_or("untitled");
    let day = if item.day.is_empty() {
        "undated"
    } else {
        &item.day
    };
    let path = dir.join(format!("{day}-{}.md", slug(title)));
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists — refusing to overwrite", path.display()),
        ));
    }
    let mut body = format!("# {title}\n\n- URL: {}\n- Date: {}\n\n", item.url, day);
    match &item.summary {
        Some(s) => body.push_str(&format!("## Destillat\n\n{s}\n")),
        None => body.push_str("## Digest\n\n_(no summary yet)_\n"),
    }
    std::fs::write(&path, body)?;
    Ok(path)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    let capped: String = trimmed.chars().take(60).collect();
    if capped.is_empty() {
        "note".into()
    } else {
        capped
    }
}

// -- summarize -----------------------------------------------------------

fn cmd_summarize(args: &[String], cfg: &Config) {
    if !args.iter().any(|a| a == "--pending") {
        eprintln!("usage: comms summarize --pending");
        std::process::exit(1);
    }
    let store = open_store(cfg);
    match media::summarize_pending(&store, cfg) {
        Ok(pass) => println!(
            "summarized {} pending feed item(s); {} past the on-device window, left for a press",
            pass.summarized, pass.over_window
        ),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// -- reclassify-feed -----------------------------------------------------

/// Re-derive `legacy` feed classes from the feed source declarations as they stand today.
///
/// Rows carry the class they were given when they arrived. On 2026-08-13 the deterministic
/// source-declared rule landed; everything ingested before it is stamped `legacy`, which names
/// *when* a row arrived and not *where it came from*. In this deployment that left 89 arXiv
/// abstracts and 68 GitHub READMEs in the redaction lane, while the very same sources — declared
/// `c0` in `config::default_feed_sources`, with the reasons written there — were putting
/// identical content in the `c0` lane. Nothing was at risk. Quality and quota were: a
/// published preprint reaching a provider with its authors stripped is a worse summary bought
/// with a redaction nobody needed.
///
/// Three properties make this a repair rather than a loosening:
///
/// * It reads the DECLARATIONS. A row is matched to a source by `sources::item_kind`, which is
///   the same adapter list `fetch` dispatches on. A kind no declared source produces — `article`
///   and `youtube`, what a hand-pasted URL falls back to — matches nothing and is left alone.
/// * It goes through `set_feed_data_class`, so `admit_reclassification` judges every row. A
///   human method plus the operator's own words is what lets a class fall, and nothing else does.
///   A row a human already classified is refused by that rule, not by a special case here.
/// * The rationale is required, not defaulted. `human_reclassification` will invent a canned
///   sentence for an escalation; a lowering must be answerable afterwards, so an empty one exits
///   before a single row is read.
fn cmd_reclassify_feed(args: &[String], cfg: &Config) {
    let rationale = arg_after(args, "--rationale")
        .map(String::as_str)
        .unwrap_or_default();
    if rationale.trim().is_empty() {
        eprintln!(
            "usage: comms reclassify-feed --rationale <text> [--dry-run]\n\n\
             Lowering a data class is a human decision and is stored with the row. Say why."
        );
        std::process::exit(1);
    }
    let dry_run = args.iter().any(|a| a == "--dry-run");

    // kind -> the class the source producing that kind declares TODAY. Disabled sources are
    // skipped: a source that is off is not making a claim about anything.
    let mut declared: std::collections::BTreeMap<&'static str, String> =
        std::collections::BTreeMap::new();
    for source in cfg.feed_sources.iter().filter(|s| s.enabled) {
        if let Some(kind) = comms::sources::item_kind(&source.adapter) {
            declared.insert(kind, source.data_class.clone());
        }
    }
    if declared.is_empty() {
        eprintln!("no enabled feed source declares a kind — nothing to re-derive against");
        std::process::exit(1);
    }

    let store = open_store(cfg);
    let rows = match store.feed_items_classified_by("legacy") {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };

    let mut changed: std::collections::BTreeMap<String, usize> = Default::default();
    let mut unchanged: std::collections::BTreeMap<String, usize> = Default::default();
    let mut refused: Vec<String> = Vec::new();
    let mut no_source: std::collections::BTreeMap<String, usize> = Default::default();

    for comms::store::FeedClassRow {
        id,
        kind,
        data_class: stored_class,
    } in rows
    {
        let Some(target) = declared.get(kind.as_str()) else {
            *no_source.entry(kind).or_default() += 1;
            continue;
        };
        if target == &stored_class {
            *unchanged.entry(kind).or_default() += 1;
            continue;
        }
        if dry_run {
            *changed.entry(kind).or_default() += 1;
            continue;
        }
        match store.set_feed_data_class(&id, target, Some(rationale)) {
            Ok(true) => *changed.entry(kind).or_default() += 1,
            // The row vanished between the read and the write. Counted as refused rather than
            // silently dropped: this pass is the record of what it did.
            Ok(false) => refused.push(format!("{id}: no such row")),
            Err(error) => refused.push(format!("{id}: {error}")),
        }
    }

    for (kind, count) in &changed {
        println!(
            "{}{kind}: {count} -> {}",
            if dry_run {
                "would reclassify "
            } else {
                "reclassified "
            },
            declared[kind.as_str()]
        );
    }
    for (kind, count) in &unchanged {
        println!("{kind}: {count} already match the declaration");
    }
    for (kind, count) in &no_source {
        println!("{kind}: {count} left alone — no enabled source declares this kind");
    }
    for line in &refused {
        println!("refused {line}");
    }
    println!(
        "{} row(s) {}, {} left alone, {} refused",
        changed.values().sum::<usize>(),
        if dry_run { "would change" } else { "changed" },
        unchanged.values().sum::<usize>() + no_source.values().sum::<usize>(),
        refused.len()
    );
}

// -- normalize -----------------------------------------------------------

fn cmd_normalize(args: &[String], cfg: &Config) {
    if args.iter().any(|a| a == "--explain") {
        println!("normalization rules — each line says what that rule throws away\n");
        for rule in normalize::RULES {
            println!("  {:<26} {}", rule.name, rule.drops);
        }
        for (name, drops) in normalize::structural_rules() {
            println!("  {name:<26} {drops}");
        }
        return;
    }

    if !args.iter().any(|a| a == "--all") {
        eprintln!("usage: comms normalize --all | --explain");
        std::process::exit(1);
    }

    let store = open_store(cfg);
    match media::renormalize_all(&store) {
        Ok(report) => {
            println!("renormalized {} item(s)", report.updated);
            if report.skipped > 0 {
                println!(
                    "{} item(s) skipped — no retained raw content, only a re-fetch can fix those",
                    report.skipped
                );
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// -- export-sources ------------------------------------------------------

/// Reconcile every saved feed item with `Resources/Sources/` in the configured vault.
///
/// The repair half of the bridge, and the reason the server's layer is allowed to fail
/// quietly: a vault that was unplugged, an iCloud folder that was not down yet, or a
/// summary that arrived from the background drain after the last mutation all leave the
/// folder stale, and all of them are fixed by one run of this — with the server down,
/// which is when a vault problem is usually being worked on.
///
/// It is also the only path that projects the library as it stands *today* on a host
/// that has never run the bridge. `--dry-run` prints the files it would write and
/// touches nothing, because the first thing anybody wants to know about a command that
/// writes into their notes is what it is about to write.
fn cmd_export_sources(args: &[String], cfg: &Config) {
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let Some(root_path) = cfg.obsidian_root.clone() else {
        eprintln!("error: no vault configured: set obsidian.root in <overlay>/config/comms.json");
        std::process::exit(1);
    };

    let store = open_store(cfg);
    let saved = match store.feed_library() {
        Ok(saved) => saved,
        Err(e) => {
            eprintln!("error: could not read the feed library: {e}");
            std::process::exit(1);
        }
    };

    if dry_run {
        for projection in comms::projection::render_all(&saved) {
            println!("{}  ({} bytes)", projection.path, projection.body.len());
        }
        println!("{} saved item(s) — nothing written", saved.len());
        return;
    }

    let root = match markdown_root::MarkdownRoot::declare(root_path) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let report = match comms::projection::export_all(&root, &saved) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "{} saved item(s) → {}: {} created, {} updated, {} unchanged",
        saved.len(),
        comms::projection::DIR,
        report.created,
        report.updated,
        report.unchanged
    );
    for path in &report.removed {
        println!("  removed (no longer saved, or retitled): {path}");
    }
    for path in &report.refused {
        println!("  refused, this file is not comms' to write: {path}");
    }
}

// -- mail classify -------------------------------------------------------

/// `comms digest corpus` — the judgement skeleton for the digest-quality corpus (D16).
///
/// Balanced across producers on purpose: the question the corpus exists to answer is whether
/// the 4B rung is as good as the 9B was and whether a public-tier provider is better than
/// either, and a sample drawn from the whole table would be whatever the ladder happened to
/// route most. `--per-producer` caps each rung, and the first N of each in the store's own
/// stable order — never a random draw, so a re-export from an unchanged database is the same
/// file.
///
/// Judgements are `null` here, and `comms-digest-eval` refuses a corpus that still has one.
fn cmd_digest(args: &[String], cfg: &Config) {
    if args.get(2).map(String::as_str) != Some("corpus") {
        eprintln!("error: usage: comms digest corpus --out <path> [--per-producer N] [--force]\n");
        std::process::exit(2);
    }
    let Some(out) = arg_after(args, "--out") else {
        eprintln!("error: usage: comms digest corpus --out <path> [--per-producer N] [--force]");
        std::process::exit(2);
    };
    let path = std::path::Path::new(out);
    if path.exists() && !args.iter().any(|a| a == "--force") {
        eprintln!("error: {out} exists. Refusing to overwrite hand-written judgements — pass --force if that is what you mean.");
        std::process::exit(2);
    }
    let per_producer: usize = arg_after(args, "--per-producer")
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);

    let store = open_store(cfg);
    let digests = match store.generated_digests() {
        Ok(digests) => digests,
        Err(error) => {
            eprintln!("error: could not read the digests: {error}");
            std::process::exit(2);
        }
    };

    let mut per: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut fixtures: Vec<serde_json::Value> = Vec::new();
    for digest in &digests {
        let taken = per.entry(digest.producer.clone()).or_default();
        if *taken >= per_producer {
            continue;
        }
        *taken += 1;
        fixtures.push(serde_json::json!({
            "source": digest.source,
            "item_id": digest.item_id,
            "producer": digest.producer,
            "shape": digest.shape,
            "source_chars": digest.source_chars,
            "generated_at": digest.generated_at,
            // The judgement is made against the SOURCE, so the fixture carries where to read
            // it rather than an excerpt: faithfulness judged against the first 500 characters
            // of an article is not faithfulness.
            "read_the_source_at": match digest.source.as_str() {
                "feed" => format!("/feed/{}", digest.item_id),
                "mail" => format!("/feed?view=mail#{}", digest.item_id),
                other => format!("{other}:{}", digest.item_id),
            },
            "digest_text": digest.text.clone().unwrap_or_default(),
            "faithful": serde_json::Value::Null,
            "useful_band": serde_json::Value::Null,
            "note": ""
        }));
    }

    let corpus = serde_json::json!({
        "_doc": "The frozen digest-quality corpus (PRD D16). It quotes real articles and real \
                 mail: this file belongs in the private overlay and never in the repository. \
                 Read by comms-digest-eval, which makes zero model calls.",
        "_method": "Written by `comms digest corpus`, balanced across producers. For each row, \
                    read the SOURCE at read_the_source_at, then write `faithful` — does every \
                    claim in the digest follow from it — and `useful_band` 0-3, where 0 says \
                    nothing the title did not and 3 means the source was not needed. Judge \
                    before comparing rungs: knowing which model wrote a digest is exactly the \
                    thing that makes a judgement unusable.",
        "_gate": "The unfaithful rate, and only that. A digest that asserts what its source does \
                  not support is read instead of the article and nothing downstream can catch \
                  it. Usefulness is reported per producer and never gates: thin but true is a \
                  preference, confident and false is a defect.",
        "acceptance": {
            "_why": "max_unfaithful_percent is a policy judgement and carries a value from the \
                     first run, like max_false_eviction_percent in the mail corpus. \
                     minimum_useful_percent is null until the first run has been read.",
            "max_unfaithful_percent": 2.0,
            "minimum_useful_percent": serde_json::Value::Null
        },
        "fixtures": fixtures
    });

    let body = match serde_json::to_string_pretty(&corpus) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("error: could not serialise the corpus: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = std::fs::write(path, body + "\n") {
        eprintln!("error: could not write {out}: {error}");
        std::process::exit(2);
    }
    println!(
        "{} fixture(s) written to {out}, from {} generated digest(s) across {} rung(s)",
        fixtures_len(&corpus),
        digests.len(),
        per.len()
    );
    for (producer, taken) in &per {
        println!("  {taken}\t{producer}");
    }
    println!("faithful and useful_band are null. comms-digest-eval REFUSES a corpus with an");
    println!("unjudged row, so a half-filled file cannot be read as a pass.");
}

/// The fixture count of a built corpus value. A helper rather than a second `len()` on the
/// vector, because the vector is moved into the JSON above and the printed number must be the
/// number that was written.
fn fixtures_len(corpus: &serde_json::Value) -> usize {
    corpus
        .get("fixtures")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

/// `comms mail corpus` — the labelling skeleton for the frozen corpus (B49).
///
/// Writes one fixture per fallback row with `label` and `urgency_band` EMPTY, because the
/// order is the whole discipline: both labels are written by hand, in one pass, BEFORE any
/// model output is read. A skeleton pre-filled from a verdict would be a corpus that agrees
/// with the model by construction.
///
/// Both labels in one pass for a cheaper reason: re-reading 102 threads to add the second
/// one costs the same 102 threads twice.
///
/// It writes into the OVERLAY, never into this repository. The rows carry real subjects and
/// snippets — already redacted for c2 and c3 at intake, which is why they may be written at
/// all — and `capabilities/comms/eval/README.md` names the destination.
fn cmd_mail_corpus(args: &[String], cfg: &Config) {
    let Some(out) = arg_after(args, "--out") else {
        eprintln!("error: usage: comms mail corpus --out <path> [--force]");
        eprintln!(
            "       the path belongs in the overlay, e.g. \
                   \"$SJEL_PERSONAL_ROOT/config/comms-mail-stream-shadow.json\""
        );
        std::process::exit(2);
    };
    let path = std::path::Path::new(out);
    if path.exists() && !args.iter().any(|a| a == "--force") {
        // Refused rather than merged: the labels in an existing file are hand-written work,
        // and there is no rule by which this command could decide which of two answers wins.
        eprintln!("error: {out} exists. Refusing to overwrite hand-written labels — pass --force if that is what you mean.");
        std::process::exit(2);
    }

    let store = open_store(cfg);
    let candidates = match store.model_rung_candidates() {
        Ok(candidates) => candidates,
        Err(error) => {
            eprintln!("error: could not read the fallback rows: {error}");
            std::process::exit(2);
        }
    };

    let fixtures: Vec<serde_json::Value> = candidates
        .iter()
        .map(|candidate| {
            serde_json::json!({
                "id": candidate.id,
                "language": guessed_language(candidate),
                "data_class": candidate.data_class,
                "sender_domain": sender_domain(candidate.from_addr.as_deref()),
                "subject": candidate.subject.clone().unwrap_or_default(),
                "snippet": candidate.snippet.clone().unwrap_or_default(),
                "rule_stream": candidate.rule_stream,
                "label": "",
                "urgency_band": serde_json::Value::Null,
                "label_note": ""
            })
        })
        .collect();

    let corpus = serde_json::json!({
        "_doc": "The frozen mail-classification corpus (PRD B49). Real mail: this file belongs \
                 in the private overlay and never in the repository. Read by comms-mail-model-eval, \
                 which makes zero model calls.",
        "_method": "Written by `comms mail corpus`, one fixture per fallback row. Fill `label` \
                    (a stream from rules::STREAMS) and `urgency_band` (0-3) by hand, both in one \
                    pass, BEFORE running `comms mail classify --shadow` or reading any verdict. \
                    `language` is a MECHANICAL GUESS from the subject and snippet and is meant to \
                    be corrected; nothing else here is guessed. Then run the shadow pass, then \
                    `comms-mail-model-eval <this path>`.",
        "_scope": "The fallback rows only. A thread a config rule or a heuristic decided is not \
                   this rung's question, and scoring it here would measure the rules.",
        "acceptance": {
            "_why": "Three thresholds are null until the first run has been read — a threshold \
                     invented before the measurement is a number chosen to be met. \
                     max_false_eviction_percent is a stated policy judgement, not a measurement.",
            "minimum_agreement_percent": serde_json::Value::Null,
            "max_false_eviction_percent": 2.0,
            "max_urgency_band_error": serde_json::Value::Null,
            "max_urgency_overstatement_percent": serde_json::Value::Null
        },
        "fixtures": fixtures
    });

    let body = match serde_json::to_string_pretty(&corpus) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("error: could not serialise the corpus: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = std::fs::write(path, body + "\n") {
        eprintln!("error: could not write {out}: {error}");
        std::process::exit(2);
    }
    println!("{} fixture(s) written to {out}", candidates.len());
    println!("label and urgency_band are empty. The eval REFUSES a corpus with an empty label,");
    println!("so a half-filled file cannot be mistaken for a low score.");
}

/// The domain of a sender address, or an empty string. Never the local part: the corpus is
/// about what kind of mail this is, and the mailbox name is a person.
fn sender_domain(from_addr: Option<&str>) -> String {
    from_addr
        .and_then(|address| address.rsplit_once('@'))
        .map(|(_, domain)| domain.trim_end_matches('>').trim().to_lowercase())
        .unwrap_or_default()
}

/// `de` or `en`, guessed from the subject and snippet.
///
/// A guess, said so in the corpus's own `_method`, and the only guessed field in the file.
/// The split by language is what makes one English prompt over a mixed mailbox measurable,
/// so the field has to be filled somehow; leaving 102 blanks for it would cost the labeller
/// a judgement they can make faster by correcting one.
fn guessed_language(candidate: &comms::store::ModelCandidate) -> &'static str {
    let text = format!(
        "{} {}",
        candidate.subject.clone().unwrap_or_default(),
        candidate.snippet.clone().unwrap_or_default()
    )
    .to_lowercase();
    const GERMAN: [&str; 12] = [
        " der ", " die ", " das ", " und ", " ist ", " nicht ", " mit ", " für ", " sie ",
        " ihre ", " wir ", " werden ",
    ];
    if text.contains('ä') || text.contains('ö') || text.contains('ü') || text.contains('ß') {
        return "de";
    }
    let padded = format!(" {text} ");
    if GERMAN.iter().any(|word| padded.contains(word)) {
        "de"
    } else {
        "en"
    }
}

/// `comms mail classify` — the model rung's operator surface.
///
/// Explicit only. There is no timer: an unattended local-model drain is what
/// made this machine hot once already, and the category axis writes a decision
/// rather than a derived field.
fn cmd_mail(args: &[String], cfg: &Config) {
    match args.get(2).map(String::as_str) {
        Some("classify") => {}
        Some("corpus") => return cmd_mail_corpus(args, cfg),
        _ => {
            eprintln!("error: usage: comms mail classify [--shadow|--apply] [--limit N] [--report] [--revert <id>|--revert-all]");
            eprintln!("              comms mail corpus --out <path> [--force]\n");
            std::process::exit(2);
        }
    }
    let store = open_store(cfg);

    // `--revert` with nothing after it used to fall through to `None`, which is
    // the argument that reverts EVERY model-written row — the same shape the
    // HTTP route refuses with a 400. The two surfaces answer alike now (review,
    // 2026-09-05).
    if args.iter().any(|a| a == "--revert") && arg_after(args, "--revert").is_none() {
        eprintln!("error: --revert needs a thread id. To revert every model-written row, say --revert-all.");
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--revert-all") || args.iter().any(|a| a == "--revert") {
        let ids: Option<Vec<String>> = arg_after(args, "--revert").map(|id| vec![id.clone()]);
        match store.revert_model_streams(ids.as_deref()) {
            Ok((reverted, not_model, no_rules)) => {
                println!("reverted {reverted} row(s) to their deterministic verdict");
                println!("skipped: {not_model} not model-written, {no_rules} with no rules row");
                println!(
                    "the data class and any narrowing this rung caused are NOT restored — \
                     the class update is escalation-only and the narrowing is not a delete"
                );
            }
            Err(error) => {
                eprintln!("error: could not revert: {error}");
                std::process::exit(2);
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--report") {
        print_classify_report(cfg, &store);
        return;
    }

    let requested = if args.iter().any(|a| a == "--apply") {
        Mode::Apply
    } else {
        Mode::Shadow
    };
    let mode = match mail_model::apply_allowed(cfg.mail_model.as_ref(), requested) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    // The overlay's `mail_model.limit` is the default when `--limit` is absent.
    let limit = mail_model::pass_limit(
        cfg.mail_model.as_ref(),
        arg_after(args, "--limit").and_then(|value| value.parse().ok()),
    );
    let min_confidence_bp = cfg
        .mail_model
        .as_ref()
        .map_or(0, |section| i64::from(section.min_confidence_bp));

    println!(
        "comms mail classify — {} mode, limit {limit}, producer {}",
        mode.as_str(),
        mail_model::producer(cfg)
    );
    let started = std::time::Instant::now();
    let receipt = match mail_model::run_pass(cfg, &store, mode, limit, min_confidence_bp) {
        Ok(receipt) => receipt,
        Err(error) => {
            eprintln!("error: the pass failed: {error}");
            std::process::exit(2);
        }
    };
    println!(
        "\nreviewed {} · eligible {} · prompted {} · refused as Secret {} · over window {}",
        receipt.reviewed,
        receipt.eligible,
        receipt.prompted,
        receipt.refused_c3,
        receipt.over_window
    );
    println!(
        "unparseable {} · outside the vocabulary {} · other errors {}",
        receipt.unparseable, receipt.invalid_stream, receipt.errors
    );
    println!(
        "agreed (nothing written) {} · disagreed {} · applied {} · held as class-raising {} · below confidence {}",
        receipt.agreed_no_write,
        receipt.disagreed,
        receipt.applied,
        receipt.held_class_escalation,
        receipt.below_confidence
    );
    if receipt.awaiting_apply > 0 {
        // Counted before this pass acted, so in apply mode it is the total this
        // pass found rather than what it left behind.
        println!(
            "{} stored disagreement(s) had no category write yet; this pass applied {}",
            receipt.awaiting_apply, receipt.applied
        );
    }
    println!("wall time {:.1}s", started.elapsed().as_secs_f64());
    print_classify_report(cfg, &store);
}

/// The agreement table, the state counts and the receipt line.
///
/// Built from `model_verdict_summaries`, whose SELECT list cannot carry a
/// subject, a snippet or a rationale — so this output is safe to paste.
fn print_classify_report(cfg: &Config, store: &Store) {
    let summaries = match store.model_verdict_summaries(None) {
        Ok(summaries) => summaries,
        Err(error) => {
            eprintln!("error: could not read the verdicts: {error}");
            std::process::exit(2);
        }
    };
    if summaries.is_empty() {
        println!("\nno verdicts stored yet — run `comms mail classify --shadow` first");
        return;
    }

    println!("\nrule stream\tn\tagree\tagree%\tmodel proposed");
    for (rule_stream, n, agree, model_streams) in mail_model::agreement(&summaries) {
        let mut streams: Vec<(String, usize)> = model_streams.into_iter().collect();
        streams.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let rendered: Vec<String> = streams
            .iter()
            .map(|(stream, count)| format!("{stream} {count}"))
            .collect();
        println!(
            "{rule_stream}\t{n}\t{agree}\t{:.1}%\t{}",
            agree as f64 * 100.0 / n as f64,
            rendered.join(", ")
        );
    }

    let mut states: BTreeMap<String, usize> = BTreeMap::new();
    let mut classes: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut held: BTreeMap<String, usize> = BTreeMap::new();
    for summary in &summaries {
        *states.entry(summary.state.clone()).or_default() += 1;
        let entry = classes.entry(summary.data_class.clone()).or_default();
        entry.0 += 1;
        if summary.model_stream.is_some() {
            entry.1 += 1;
        }
        if let Some(reason) = &summary.held_reason {
            *held.entry(reason.clone()).or_default() += 1;
        }
    }
    println!("\nstate\tn");
    for (state, n) in &states {
        println!("{state}\t{n}");
    }
    println!("\ndata class\tverdicts\tanswered");
    for (class, (n, answered)) in &classes {
        println!("{class}\t{n}\t{answered}");
    }
    if !held.is_empty() {
        println!("\nheld for a human\tn");
        for (reason, n) in &held {
            println!("{reason}\t{n}");
        }
    }
    println!(
        "\nproducer {} · prompt {} · classifier {} · cloud calls 0",
        mail_model::producer(cfg),
        mail_model::MAIL_MODEL_PROMPT_REVISION,
        mail_model::MAIL_MODEL_VERSION
    );
}

// -- relevance -----------------------------------------------------------

/// Re-score the stored feed through the running server, page by page.
///
/// An HTTP client, deliberately not a store opener. `Store::open` runs the
/// whole migration on every call and two openers on one SQLite file deadlock —
/// `tools/feed-sweep` records that reason, and comms is `autostart = true`,
/// so a server answering is the expected state rather than a precondition this
/// verb has to arrange.
fn cmd_relevance(args: &[String], cfg: &Config) {
    let Some(verb) = args.get(2).filter(|value| !value.starts_with("--")) else {
        eprintln!(
            "error: usage: comms relevance backfill [--days N] [--batch N] [--max N] [--force]"
        );
        std::process::exit(1);
    };
    if verb != "backfill" {
        eprintln!("error: unknown relevance verb '{verb}' -- the only verb is `backfill`");
        std::process::exit(1);
    }
    // Ten years, not one. The route's window is the corpus-completion test:
    // `POST /feed/relevance/refresh` only marks the relevance revision complete
    // for a pass that asked for the widest window (server/feed.rs
    // `FULL_WINDOW_DAYS`), and a bare `backfill` that asked for 365 both left
    // older rows unreachable and could not finish the chain. The design
    // (/tmp/axon-night/designs/feed-personalization.md) specifies 3650.
    let days: i32 = arg_after(args, "--days")
        .and_then(|value| value.parse().ok())
        .unwrap_or(3650);
    let batch: usize = arg_after(args, "--batch")
        .and_then(|value| value.parse().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let max: Option<usize> = arg_after(args, "--max").and_then(|value| value.parse().ok());
    let force = args.iter().any(|value| value == "--force");

    let base = format!("http://127.0.0.1:{}", cfg.port);
    let client = match sjel_http::client(
        sjel_http::Purpose::new("comms-cli"),
        std::time::Duration::from_secs(600),
    ) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("error: could not build an HTTP client: {error}");
            std::process::exit(1);
        }
    };

    let mut offset = 0usize;
    let mut considered = 0usize;
    let mut rescored = 0usize;
    let mut reused = 0usize;
    let mut refused = 0usize;
    let mut last_mode;
    loop {
        let mut request =
            client
                .post(format!("{base}/feed/relevance/refresh"))
                .json(&serde_json::json!({
                    "days": days,
                    "limit": batch,
                    "offset": offset,
                    "force": force,
                }));
        if let Some(secret) = cfg.api_secret.as_deref() {
            request = request.header("X-Sjel-Token", secret);
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                eprintln!("error: comms is not answering on {base} ({error})");
                eprintln!("       start it first -- this verb re-scores through the server.");
                std::process::exit(1);
            }
        };
        if !response.status().is_success() {
            eprintln!("error: {base} answered HTTP {}", response.status());
            std::process::exit(1);
        }
        let page: serde_json::Value = match response.json() {
            Ok(page) => page,
            Err(error) => {
                eprintln!("error: could not read the page ({error})");
                std::process::exit(1);
            }
        };
        let count = |key: &str| page[key].as_u64().unwrap_or(0) as usize;
        considered += count("considered");
        rescored += count("rescored");
        reused += count("reused_relevance");
        refused += count("refused_class");
        last_mode = page["embedding"]["mode"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        println!(
            "  offset {offset:>5}: considered {}, re-scored {}, re-evaluated {}, refused {}, mode {last_mode}",
            count("considered"),
            count("rescored"),
            count("reused_relevance"),
            count("refused_class"),
        );
        if let Some(class) = page["embedding"]["error_class"].as_str() {
            println!("               embedding fell back: {class}");
        }
        let has_more = page["has_more"].as_bool().unwrap_or(false);
        offset += batch;
        if !has_more || max.is_some_and(|cap| considered >= cap) {
            break;
        }
    }
    println!(
        "backfill finished: {considered} considered, {rescored} re-scored, {reused} re-evaluated from stored matches, {refused} refused by class; last mode {last_mode}"
    );
}

// -- egress-log ----------------------------------------------------------

fn cmd_egress_log(args: &[String], cfg: &Config) {
    let limit: usize = arg_after(args, "--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let audit = args.iter().any(|a| a == "--audit");
    let store = open_store(cfg);

    if audit {
        let report = match store.egress_audit() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: could not run egress audit: {e}");
                std::process::exit(1);
            }
        };
        println!("comms egress audit (PRD §6, ISC-22, ISC-23)");
        println!("───────────────────────────────────────────");
        println!("  total model calls:     {}", report.total_calls);
        println!("  succeeded:             {}", report.succeeded_calls);
        println!("  failed:                {}", report.failed_calls);
        println!("  prompt tokens:         {}", report.total_prompt_tokens);
        println!(
            "  completion tokens:     {}",
            report.total_completion_tokens
        );
        println!("  total tokens:          {}", report.total_tokens);
        println!(
            "  total cost:            ${:.4} ({:.2} cents)",
            report.total_cost_cents / 100.0,
            report.total_cost_cents
        );
        println!(
            "  raw C2 violations:     {}",
            report.raw_c2_violations.len()
        );
        if !report.raw_c2_violations.is_empty() {
            println!("\nVIOLATIONS FOUND:");
            for v in &report.raw_c2_violations {
                println!("  [!] {v}");
            }
            std::process::exit(1);
        } else {
            println!("  [✓] no raw data about other people found in egress log.");
        }
    } else {
        let entries = match store.list_egress_entries(limit) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("error: could not read egress log: {e}");
                std::process::exit(1);
            }
        };
        println!(
            "comms egress log — {} recent outbound cloud call(s)\n",
            entries.len()
        );
        for entry in entries {
            let status_badge = if entry.status == "succeeded" {
                "✓"
            } else {
                "✗"
            };
            println!(
                "[{}] #{} {} | {}/{} ({}) | tokens: {} (p:{} c:{}) | ${:.4}",
                status_badge,
                entry.id,
                entry.timestamp,
                entry.provider,
                entry.model,
                entry.task,
                entry.total_tokens,
                entry.prompt_tokens,
                entry.completion_tokens,
                entry.cost_cents / 100.0,
            );
            if let Some(err) = &entry.error {
                println!("    error: {err}");
            }
        }
    }
}

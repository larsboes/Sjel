//! Trips' commands that are not the server.
//!
//! `draft-intent` turns a sentence into a plan draft nobody has submitted. It posts
//! to the local model rung directly rather than depending on `libs/inference`, because
//! this is one request to a loopback URL and a role lookup would be more machinery
//! than the call it wraps. When the model is unreachable it says how to start it and
//! prints the heuristic draft that `POST /api/intent/draft` would also return.
//!
//! `export-vault` writes the safety copy PRD Q47 requires. It has no HTTP route
//! either, and for a different reason: the server already re-exports after every write
//! (`src/server.rs`, `project_after_write`), so a route would be a third caller of one
//! function with no reader. What a human needs is a command they can run when the
//! server is down — which is exactly when a safety copy matters — and that is this.

use std::io::Read;

const USAGE: &str = "\
Usage:
  trips draft-intent \"somewhere warm in October, under 300 euro, by train\"
  trips draft-intent -            read the sentence from stdin
  trips export-vault              write every plan to the vault projection
  trips export-vault --dry-run    print what would be written, touch nothing
  trips gear import               propose interior items from the overlay's gear notes
  trips gear import --json        the same proposals as JSON
  trips gear import --apply       write each proposal to interior (409 = already there)

draft-intent prints a CreatePlan-shaped draft plus what it could not resolve.
Persists nothing and resolves no station: every destination comes back as a
place slug with null coordinates, exactly as typed text does.

export-vault writes one Markdown file per plan under Resources/Sjel/Trips/ in
the vault named by <overlay>/config/trips.json, each carrying every plan item's
payload verbatim. The server does the same after every write; this is the copy
you can take by hand.

gear import READS the overlay's item notes and PROPOSES one interior item per
note; --apply writes them through interior's own POST /api/items. Proposal ids
are derived from the note file names and interior answers 409 for an id it
already has, so running it twice writes nothing the second time. A note is
`owned` unless its front matter says `state: wanted`.

Environment:
  SJEL_INTENT_URL     chat-completions endpoint (default http://127.0.0.1:8091/v1/chat/completions)
  SJEL_INTENT_MODEL   model name (default apple-foundationmodel)
  SJEL_TRIPS_GEAR_DIR gear notes directory (default <overlay>/data/items/vault-notes)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("draft-intent"), Some(text)) => {
            let sentence = if text == "-" {
                read_stdin()
            } else {
                text.to_string()
            };
            if let Err(error) = draft(&sentence) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        (Some("gear"), Some(verb)) if verb == "import" => {
            let flag = args.get(2).map(String::as_str);
            if !matches!(flag, None | Some("--json") | Some("--apply")) {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
            if let Err(error) = gear_import(flag) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        (Some("export-vault"), rest) => {
            let dry_run = matches!(rest.map(String::as_str), Some("--dry-run"));
            if rest.is_some() && !dry_run {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
            if let Err(error) = export_vault(dry_run) {
                eprintln!("trips: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Write every plan into the vault, or say what a write would do.
///
/// An unconfigured vault is an error rather than a quiet zero: the whole point of the
/// command is that a copy exists, and "nothing to do" is indistinguishable from
/// "nothing was saved" once the terminal scrolls.
fn export_vault(dry_run: bool) -> Result<(), String> {
    let config = trips::config::Config::load();
    let vault = config
        .obsidian
        .ok_or("no vault configured: set obsidian.root in <overlay>/config/trips.json")?;
    let root =
        markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;

    let store = trips::store::TripsStore::open(&config.database_path).map_err(|e| e.to_string())?;
    let plans = store.list_every_plan().map_err(|e| e.to_string())?;
    let items: usize = plans.iter().map(|p| p.items.len()).sum();

    if dry_run {
        for projection in trips::projection::render_all(&plans) {
            println!("{}  ({} bytes)", projection.path, projection.body.len());
        }
        println!("{} plan(s), {items} item(s) — nothing written", plans.len());
        return Ok(());
    }

    let report = trips::projection::export_all(&root, &plans).map_err(|e| e.to_string())?;
    println!(
        "{} plan(s), {items} item(s) → {}: {} created, {} updated, {} unchanged",
        plans.len(),
        trips::projection::DIR,
        report.created,
        report.updated,
        report.unchanged
    );
    for path in &report.removed {
        println!("  removed (plan gone or renamed): {path}");
    }
    for path in &report.refused {
        println!("  refused, a human owns this file now: {path}");
    }
    Ok(())
}

/// Read the overlay's gear notes, print the proposals, and write them with `--apply`.
///
/// The apply branch POSTs each proposal to interior's own `POST /api/items` — never SQL,
/// though both capabilities open the same file — because that handler answers 409 for an id
/// that already exists, which makes a re-run idempotent without this file tracking anything.
/// It was refused between 2026-09-05 and 2026-09-07, while `interior_item` had no column to
/// tell a tent from a sofa; B51 shipped the seven columns and the refusal is gone.
fn gear_import(flag: Option<&str>) -> Result<(), String> {
    let directory = trips::config::Config::load().gear_items_dir.ok_or(
        "no gear notes directory: set SJEL_PERSONAL_ROOT, or gear.items_dir in \
         <overlay>/config/trips.json",
    )?;
    let report = trips::gear::scan(&directory)?;

    if flag == Some("--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!(
        "{} note(s) read, {} proposal(s), {} skipped (no item fields)",
        report.scanned,
        report.proposals.len(),
        report.skipped
    );
    for proposal in &report.proposals {
        let gaps = if proposal.incomplete.is_empty() {
            String::new()
        } else {
            format!("  [missing: {}]", proposal.incomplete.join(", "))
        };
        println!("  {}  {}{gaps}", proposal.id, proposal.label);
    }
    if !report.trip_type_vocabulary.is_empty() {
        println!(
            "trip_types in use ({}): {}",
            report.trip_type_vocabulary.len(),
            report.trip_type_vocabulary.join(", ")
        );
    }
    if flag != Some("--apply") {
        println!("\n{}", trips::gear::NOTHING_WRITTEN);
        return Ok(());
    }

    println!(
        "\nWriting to interior at {}",
        trips::interior_client::interior_base_url()
    );
    let (mut created, mut already, mut failed) = (0_usize, 0_usize, 0_usize);
    for proposal in &report.proposals {
        let outcome = trips::interior_client::create_item(
            &proposal.item_payload(),
            &proposal.state,
            "aus den Ausruestungsnotizen importiert (trips gear import)",
        );
        match outcome {
            trips::interior_client::Written::Created => {
                created += 1;
                println!("  created  {}  ({})", proposal.id, proposal.state);
            }
            trips::interior_client::Written::AlreadyThere => {
                already += 1;
                println!("  exists   {}", proposal.id);
            }
            trips::interior_client::Written::Refused(reason) => {
                failed += 1;
                println!("  refused  {}  {reason}", proposal.id);
            }
            trips::interior_client::Written::Unreachable(reason) => {
                // The first unreachable ends the run. Interior does not come back inside one
                // loop, and forty identical failures would bury the one line that matters.
                return Err(format!("interior is not reachable: {reason}"));
            }
        }
    }
    println!("{created} written, {already} already there, {failed} refused");
    // A refusal is interior's judgement about a row, not a transport failure, so the run
    // reports it and exits non-zero — a partial import that reports success is the shape
    // this whole file was written to avoid.
    if failed > 0 {
        return Err(format!("{failed} proposal(s) refused by interior"));
    }
    Ok(())
}

fn read_stdin() -> String {
    let mut buffer = String::new();
    let _ = std::io::stdin().read_to_string(&mut buffer);
    buffer
}

const CLI_MODEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn draft(sentence: &str) -> Result<(), String> {
    let sentence = sentence.trim();
    if sentence.is_empty() {
        return Err("give me a sentence to draft from".into());
    }
    // 60 s rather than the server's 10: a human at a terminal can wait for a cold
    // on-device model, and a heuristic draft is a worse answer than a slow one.
    let drafted = trips::intent::resolve_draft_or_heuristic_with(sentence, |text| {
        trips::intent::query_model_within(text, CLI_MODEL_TIMEOUT).inspect_err(|error| {
            eprintln!("trips: {error}\ntrips: drafting heuristically instead");
        })
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&drafted).map_err(|e| e.to_string())?
    );
    Ok(())
}

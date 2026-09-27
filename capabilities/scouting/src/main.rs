use std::path::PathBuf;

use scouting::adapters::cfp_conferences::CfpConferencesAdapter;
use scouting::adapters::euro_hackathons::EuroHackathonsAdapter;
use scouting::adapters::luma::LumaAdapter;
use scouting::adapters::meetup::MeetupAdapter;
use scouting::adapters::transit_fare::TransitFareAdapter;
use scouting::config::Config;
use scouting::event_route::classify_ranked;
use scouting::merge::{merge, MergedEntry};
use scouting::opportunity::Opportunity;
use scouting::pipeline::{backlog_from_store, fetch_json, run};
use scouting::score::{
    load_opp_embeddings, load_telos_profiles, score, ScoredOpportunity, TelosProfile,
};
use scouting::source::{SearchQuery, SourceAdapter};
use scouting::sources::{print_sources, SourceFactoryError, SourceManifest};
use scouting::store::Store;
use scouting::vault_linker;

/// Handles `--dismiss <id>` / `--save <id>`: opens the real store, sets
/// status, prints a one-line confirmation, and exits. Exits with code 1 (not
/// a panic) for an unknown id or a store-open failure, so this is scriptable.
fn set_status_and_exit(database_path: &std::path::Path, id: &str, status: &str) -> ! {
    let store = match Store::open(database_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "error: could not open store at {}: {e}",
                database_path.display()
            );
            std::process::exit(1);
        }
    };
    match store.set_status(id, status) {
        Ok(true) => {
            println!("{id} -> {status}");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("error: no opportunity found with id '{id}'");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Adapter resolution: try the source registry (config-driven) first, then
// fall back to the built-in API adapters.
// ---------------------------------------------------------------------------

/// What `--adapter <name>` turned out to mean.
///
/// Three-state on purpose. This used to return `Option`, which folded "there is
/// no such source" together with "there is one, and it did not build" — and the
/// caller treated both as *try the built-ins next*. So a source with a broken
/// config printed its own error and then quietly ran EuroHackathons instead.
enum SourceLookup {
    Found(Box<dyn SourceAdapter>),
    /// No declared source carries this id. The caller may try the built-ins.
    NotDeclared,
    /// Declared, and it cannot run. Never falls through: the operator named
    /// this source, and answering with a different one's results is worse than
    /// answering with nothing.
    Failed(String),
}

/// Resolve an adapter by name from the source registry.
fn resolve_source_adapter(id: &str, sources: &[SourceManifest], _no_store: bool) -> SourceLookup {
    let Some(manifest) = sources.iter().find(|s| s.id == id) else {
        return SourceLookup::NotDeclared;
    };
    // Disabled is a decision the operator made, not an absence. Falling through
    // here would run something they never asked for in place of something they
    // deliberately turned off.
    if !manifest.enabled {
        return SourceLookup::Failed(format!(
            "source '{id}' is declared but disabled — set \"enabled\": true in \
             the overlay's scouting.json to run it"
        ));
    }
    match scouting::sources::create_adapter(manifest) {
        Ok(adapter) => SourceLookup::Found(adapter),
        // Only the `Config` variant already names the source it came from.
        // Prefixing unconditionally printed `source 'x': source 'x': …`, which
        // reads like two separate failures.
        Err(e) => SourceLookup::Failed(match &e {
            SourceFactoryError::Config { .. } => e.to_string(),
            _ => format!("source '{id}': {e}"),
        }),
    }
}

/// The built-in adapter names `--adapter` accepts, so the failure can list
/// them. Kept beside `build_api_adapter`'s arms, with a test that fails if one
/// gains a name this misses.
const API_ADAPTERS: [&str; 6] = [
    "euro_hackathons",
    "luma",
    "meetup",
    "cfp",
    "cfp_conferences",
    "transit_fare",
];

/// What to print when a name matches neither a declared source nor a built-in.
///
/// Lists both sides, because the failure alone does not tell the operator which
/// half they got wrong. A disabled source is named as disabled rather than
/// omitted: leaving it out reads as "not configured" and sends them to write an
/// entry that already exists.
fn unknown_adapter_error(name: &str, sources: &[SourceManifest]) -> String {
    let declared = if sources.is_empty() {
        "(none declared)".to_string()
    } else {
        sources
            .iter()
            .map(|s| {
                if s.enabled {
                    s.id.clone()
                } else {
                    format!("{} (disabled)", s.id)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "unknown adapter '{name}'\n  declared sources: {declared}\n  built-in adapters: {}",
        API_ADAPTERS.join(", ")
    )
}

/// Build a built-in API adapter by name.
///
/// `None` means the name is not one of these, which is the caller's problem to
/// report. It used to mean EuroHackathons: the catch-all arm ran it for
/// *anything* unrecognized, so `--adapter meetupp` fetched hackathons and said
/// nothing about it. A typo returning somebody else's results is the worst
/// available answer, because it looks exactly like a working run.
///
/// Still exits rather than returning for a required config that is missing
/// (`transit_fare`): that is a different failure, already reported precisely,
/// and folding it into "unknown adapter" would make the message wrong.
fn build_api_adapter(name: &str, no_store: bool) -> Option<Box<dyn SourceAdapter>> {
    let adapter: Box<dyn SourceAdapter> = match name {
        "euro_hackathons" => {
            // The cache is a developer convenience: a directory of saved
            // responses so a sweep can be re-run offline.
            let cache_dir = PathBuf::from("infra/data/scouting-cache/euro_hackathons");
            if cache_dir.exists() {
                Box::new(EuroHackathonsAdapter::with_cache(cache_dir))
            } else {
                Box::new(EuroHackathonsAdapter::new())
            }
        }
        "luma" => Box::new(LumaAdapter::new()),
        "meetup" => Box::new(MeetupAdapter::new()),
        "cfp" | "cfp_conferences" => Box::new(CfpConferencesAdapter::new()),
        "transit_fare" => {
            let tcfg = transit::config::Config::load();
            let (from_eva, to_eva) = match (
                tcfg.default_from_eva.clone(),
                tcfg.default_to_eva.clone(),
            ) {
                (Some(f), Some(t)) => (f, t),
                _ => {
                    eprintln!(
                        "error: adapter 'transit_fare' needs default_from_eva/default_to_eva set in \
                         transit's overlay config (axon-overlay/config/transit.json) -- no baked-in \
                         station default, by design (see capabilities/transit/src/config.rs)"
                    );
                    std::process::exit(1);
                }
            };
            let transit_store = if no_store {
                None
            } else {
                transit::store::TransitStore::open(&tcfg.database_path).ok()
            };
            Box::new(TransitFareAdapter::new(from_eva, to_eva, transit_store))
        }
        _ => return None,
    };
    Some(adapter)
}

// ---------------------------------------------------------------------------
// Shared run logic — wrapped so we can run a single adapter (existing CLI
// workflow) or multiple sources (new default when sources[] is configured).
// ---------------------------------------------------------------------------

/// Common run for one adapter. Returns the pipeline report.
#[derive(Clone, Copy)]
struct RunOptions<'a> {
    opp_embeddings_path: Option<&'a str>,
    database_path: &'a std::path::Path,
    no_store: bool,
    show_backlog: bool,
    include_dismissed: bool,
    limit: usize,
}

fn run_adapter(
    adapter: &dyn SourceAdapter,
    query: &SearchQuery,
    cfg: &Config,
    options: RunOptions<'_>,
) -> Result<scouting::pipeline::PipelineReport, Box<dyn std::error::Error>> {
    let telos = load_telos_profiles(&cfg.interest_profile_dir.to_string_lossy(), &cfg.sources);
    let events_dir = cfg.events_dir.as_deref();
    let opp_embeddings = options.opp_embeddings_path.map(load_opp_embeddings);

    let mut store: Option<Store> = if options.show_backlog || !options.no_store {
        Store::open(options.database_path).ok()
    } else {
        None
    };

    // For backlog view, we don't run the pipeline — just show the store.
    if options.show_backlog {
        match &store {
            Some(st) => {
                println!(
                    "Axon Scouting — stored backlog{}\n",
                    if options.include_dismissed {
                        " (including dismissed)"
                    } else {
                        ""
                    }
                );
                let rows = backlog_from_store(st, options.limit, options.include_dismissed)?;
                if rows.is_empty() {
                    println!("  store is empty");
                } else {
                    for (i, r) in rows.iter().enumerate() {
                        let vl = r.vault_link.as_deref().unwrap_or("none");
                        let route = classify_ranked(r, cfg.geo.as_ref());
                        let route_label = route
                            .as_ref()
                            .map(|route| route.route.as_str())
                            .unwrap_or("n/a");
                        println!(
                            "  {}. [{:.3}] {} · {} · focus: {} · route: {} · status: {} · vault: {}",
                            i + 1,
                            r.score,
                            r.title,
                            r.city,
                            r.matched_focus,
                            route_label,
                            r.status,
                            vl
                        );
                        if let Some(route) = route {
                            println!("     {}", route.reason);
                        }
                    }
                }
            }
            None => eprintln!(
                "  could not open store at {}",
                options.database_path.display()
            ),
        }
        // Return an empty report — we already printed everything.
        return Ok(scouting::pipeline::PipelineReport {
            scored: vec![],
            new_count: 0,
            vault_links: 0,
            store_total: 0,
        });
    }

    let result = run(
        adapter,
        query,
        &telos,
        opp_embeddings.as_ref(),
        store.as_mut(),
        events_dir,
    );

    if let Ok(ref _report) = result {
        if let Some(st) = store.as_ref() {
            if let Err(e) = st.record_run(adapter.name(), None) {
                eprintln!(
                    "warning: failed to record run for '{}': {e}",
                    adapter.name()
                );
            }
        }
    }

    result
}

/// Run every enabled source through the pipeline, then merge the scored
/// batches into one deduplicated cross-source ranking (merge.rs). This is the
/// default multi-source path; `--no-merge` keeps the per-source blocks.
#[allow(clippy::too_many_arguments)]
fn run_merged_sources(
    enabled_sources: &[&SourceManifest],
    query: &SearchQuery,
    cfg: &Config,
    opp_emb_path: &Option<String>,
    database_path: &std::path::Path,
    no_store: bool,
    include_dismissed: bool,
    limit: usize,
) {
    println!(
        "Axon Scouting — opportunity discovery (merged, {} sources)\n",
        enabled_sources.len()
    );

    let mut per_source: Vec<(String, Vec<ScoredOpportunity>)> = Vec::new();
    let mut new_count = 0;
    let mut vault_links = 0;
    let mut store_total = 0;

    for manifest in enabled_sources {
        let adapter = match scouting::sources::create_adapter(manifest) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("  error creating source '{}': {e}", manifest.id);
                continue;
            }
        };

        println!("  source     : {} ({})", manifest.id, manifest.adapter);
        match run_adapter(
            &*adapter,
            query,
            cfg,
            RunOptions {
                opp_embeddings_path: opp_emb_path.as_deref(),
                database_path,
                no_store,
                show_backlog: false,
                include_dismissed,
                limit,
            },
        ) {
            Ok(report) => {
                new_count += report.new_count;
                vault_links += report.vault_links;
                store_total = report.store_total; // store size, not additive
                per_source.push((manifest.id.clone(), report.scored));
            }
            Err(e) => println!("  pipeline error ({}): {e}", manifest.id),
        }
    }
    println!();

    let merged = merge(per_source);
    let store = Store::open(database_path).ok();
    print_merged(&merged, store.as_ref(), new_count, vault_links, store_total);
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

fn print_run_header(
    adapter: &dyn SourceAdapter,
    cfg: &Config,
    opp_emb_path: &Option<String>,
    database_path: &std::path::Path,
) {
    println!(
        "  adapter    : {} ({})",
        adapter.name(),
        adapter.opportunity_type().as_str()
    );
    println!(
        "  interests  : {} profiles from {}",
        scouting::score::load_telos_profiles(
            &cfg.interest_profile_dir.to_string_lossy(),
            &cfg.sources
        )
        .len(),
        cfg.interest_profile_dir.display()
    );
    if let Some(ref emb) = opp_emb_path.as_ref().and_then(|p| {
        scouting::score::load_opp_embeddings(p)
            .into_iter()
            .next()
            .map(|_| p.clone())
    }) {
        println!("  embeddings : pre-computed e5 vectors from {}", emb);
    } else if let Some(role) = scouting::embed::embedding_role() {
        // Reports what will actually run, not just whether a file was passed.
        // The old branch said "hash-fallback" whenever no pre-computed file
        // existed, which stopped being true the moment the opportunity side
        // got a live backend.
        // "configured", not "live": this line prints before any request is
        // made, so it cannot know whether the backend answers. The per-entry
        // rationale carries what actually produced each vector.
        println!(
            "  embeddings : konfiguriert {} ({})",
            role.cache_key(),
            role.backend.base_url
        );
    } else {
        println!("  embeddings : none (hash-fallback -- no 'embedding' role for this machine)");
    }
    // A path, so nothing to redact: it names a file, not a credential.
    println!("  store      : {}", database_path.display());
    if let Some(ref events_dir) = cfg.events_dir {
        println!("  event link : {} (annotate-only)", events_dir.display());
    }
    println!();
}

fn print_help() {
    println!("Axon Scouting — opportunity discovery\n");
    println!("usage: scout [flags]\n");
    println!(
        "  --adapter <name>         run one source (registry id from --list-sources, or built-in:"
    );
    println!("                           luma, meetup, cfp, transit_fare, euro_hackathons)");
    println!("  --query <text>           search query text");
    println!("  --location <loc>         location filter");
    println!("  --date-from <date>       earliest start date");
    println!("  --limit <n>              max results per source (default 20)");
    println!(
        "  --no-merge               with multiple configured sources: print per-source blocks"
    );
    println!("                           instead of the default merged cross-source ranking");
    println!("  --emit-json              print raw opportunities as JSON, no scoring");
    println!("  --opp-embeddings <path>  pre-computed opportunity embedding vectors");
    println!("  --database-path <path>   store file (default: the shared axon.db)");
    println!("  --no-store               don't persist results");
    println!("  --backlog                show stored backlog instead of fetching");
    println!("  --include-dismissed      include dismissed entries in --backlog");
    println!("  --dismiss <id>           mark an opportunity dismissed and exit");
    println!("  --save <id>              mark an opportunity saved and exit");
    println!("  --list-sources           print configured sources and exit");
    println!("  --luma-calendar <cal-id> run one Luma calendar ad hoc (declare recurring ones");
    println!("                           as a luma-calendar source in scouting.json instead)");
    println!("  --promote-calendar       upsert saved luma events into capabilities/calendar");
    println!("  --timezone <zone>        home timezone for the promotion (overrides config)");
    println!("  --calendar-url <url>     calendar base URL (default http://127.0.0.1:8087)");
    println!("  --dry-run                with --promote-calendar: show what would be written");
    println!("  --from-file <path>       (deprecated) score a JSON dump of opportunities");
    println!("  --help, -h               show this help");
}

/// Resolves the home timezone for the calendar promotion. There is no
/// default on purpose (see `Config::home_timezone`) — an unset zone is an
/// error naming both places it can be set, not a silent guess at UTC.
fn resolve_home_timezone(
    flag: Option<&str>,
    cfg: &Config,
) -> Result<scouting::localtime::HomeTimezone, String> {
    let name = flag
        .map(str::to_string)
        .or_else(|| cfg.home_timezone.clone())
        .ok_or_else(|| {
            "no home timezone set — declare SJEL_HOME_TIMEZONE in the overlay's \
             config/deployment.env (calendar resolves the same value), or pass \
             --timezone <zone> for this run. Luma reports events in UTC and calendar \
             stores local wall time, so promoting without one would write the wrong hour."
                .to_string()
        })?;
    scouting::localtime::HomeTimezone::parse(&name)
}

/// Merged variant of `print_results`: one cross-source ranked list, scores
/// normalized per source (see merge.rs for the method), source ids shown per
/// entry. Raw cosine stays visible inside each rationale line.
fn print_merged(
    entries: &[MergedEntry],
    store: Option<&Store>,
    new_count: usize,
    vault_links: usize,
    store_total: i64,
) {
    let visible: Vec<_> = match store {
        Some(st) => entries
            .iter()
            .filter(|m| {
                st.get_status(&m.scored.opportunity.id)
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some("dismissed")
            })
            .collect(),
        None => entries.iter().collect(),
    };

    if visible.is_empty() {
        if entries.is_empty() {
            println!("  no opportunities found");
        } else {
            println!(
                "  no opportunities found ({} previously dismissed, hidden — see --backlog --include-dismissed)",
                entries.len()
            );
        }
        return;
    }

    println!(
        "  merged ranked backlog ({} entries, {} new, {} vault links, {} in store):\n",
        visible.len(),
        new_count,
        vault_links,
        store_total
    );

    for (i, m) in visible.iter().enumerate() {
        let s = &m.scored;
        let focus = s.matched_focus.as_deref().unwrap_or("none");
        let dates = s.opportunity.starts_at.as_deref().unwrap_or("TBD");
        let loc = s.opportunity.city.as_deref().unwrap_or("Unknown");
        println!(
            "  {}. [{:.3}] {}",
            i + 1,
            m.normalized_score,
            s.opportunity.title
        );
        println!(
            "     {} · {} · focus: {} · sources: {}",
            dates,
            loc,
            focus,
            m.sources.join(", ")
        );
        println!("     {}", s.rationale);
        println!("     {}", s.opportunity.url);
        println!();
    }
}

fn print_results(
    report: &scouting::pipeline::PipelineReport,
    store: Option<&Store>,
    _include_dismissed: bool,
) {
    let visible: Vec<_> = match store {
        Some(st) => report
            .scored
            .iter()
            .filter(|s| {
                st.get_status(&s.opportunity.id).ok().flatten().as_deref() != Some("dismissed")
            })
            .collect(),
        None => report.scored.iter().collect(),
    };

    if visible.is_empty() {
        if report.scored.is_empty() {
            println!("  no opportunities found");
        } else {
            println!(
                "  no opportunities found ({} previously dismissed, hidden — see --backlog --include-dismissed)",
                report.scored.len()
            );
        }
        return;
    }

    println!(
        "  ranked backlog ({} entries, {} new, {} vault links, {} in store):\n",
        visible.len(),
        report.new_count,
        report.vault_links,
        report.store_total
    );

    for (i, s) in visible.iter().enumerate() {
        let focus = s.matched_focus.as_deref().unwrap_or("none");
        let dates = s.opportunity.starts_at.as_deref().unwrap_or("TBD");
        let loc = s.opportunity.city.as_deref().unwrap_or("Unknown");
        println!("  {}. [{:.3}] {}", i + 1, s.score, s.opportunity.title);
        println!("     {} · {} · focus: {}", dates, loc, focus);
        println!("     {}", s.rationale);
        println!("     {}", s.opportunity.url);
        println!();
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let cfg = Config::load();
    let limit: usize = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(20);
    let loc = args
        .iter()
        .position(|a| a == "--location")
        .and_then(|i| args.get(i + 1).cloned());
    let emit_json = args.iter().any(|a| a == "--emit-json");
    let opp_emb_path = args
        .iter()
        .position(|a| a == "--opp-embeddings")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| {
            cfg.opp_embeddings_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
        });
    // `--database-path` replaces `--database-url`: the store is a file now, and
    // the flag names one so a developer can point a run at a scratch database.
    let database_path = args
        .iter()
        .position(|a| a == "--database-path")
        .and_then(|i| args.get(i + 1).cloned())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| cfg.database_path.clone());
    let no_store = args.iter().any(|a| a == "--no-store");
    let show_backlog = args.iter().any(|a| a == "--backlog");
    let list_sources = args.iter().any(|a| a == "--list-sources");
    let from_file = args
        .iter()
        .position(|a| a == "--from-file")
        .and_then(|i| args.get(i + 1).cloned());
    let query_text = args
        .iter()
        .position(|a| a == "--query")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_default();
    let date_from = args
        .iter()
        .position(|a| a == "--date-from")
        .and_then(|i| args.get(i + 1).cloned());
    let adapter_name = args
        .iter()
        .position(|a| a == "--adapter")
        .and_then(|i| args.get(i + 1).cloned());
    let include_dismissed = args.iter().any(|a| a == "--include-dismissed");
    let no_merge = args.iter().any(|a| a == "--no-merge");
    let dismiss_id = args
        .iter()
        .position(|a| a == "--dismiss")
        .and_then(|i| args.get(i + 1).cloned());
    let save_id = args
        .iter()
        .position(|a| a == "--save")
        .and_then(|i| args.get(i + 1).cloned());
    let luma_calendar = args
        .iter()
        .position(|a| a == "--luma-calendar")
        .and_then(|i| args.get(i + 1).cloned());
    let promote_calendar = args.iter().any(|a| a == "--promote-calendar");
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let timezone_flag = args
        .iter()
        .position(|a| a == "--timezone")
        .and_then(|i| args.get(i + 1).cloned());
    let calendar_url = args
        .iter()
        .position(|a| a == "--calendar-url")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| cfg.calendar_base_url.clone());

    // --dismiss/--save
    if let Some(id) = dismiss_id {
        set_status_and_exit(&database_path, &id, "dismissed");
    }
    if let Some(id) = save_id {
        set_status_and_exit(&database_path, &id, "saved");
    }

    // --promote-calendar: saved luma events → calendar entries, then exit.
    if promote_calendar {
        let tz = match resolve_home_timezone(timezone_flag.as_deref(), &cfg) {
            Ok(tz) => tz,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        let store = match Store::open(&database_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: store {}: {e}", database_path.display());
                std::process::exit(1);
            }
        };
        println!("Axon Scouting — promote saved luma events into calendar\n");
        match scouting::calendar_promote::promote_saved_luma(
            &store,
            &calendar_url,
            &tz,
            limit.max(100),
            dry_run,
            cfg.geo.as_ref(),
        ) {
            Ok(report) => {
                scouting::calendar_promote::print_report(&report, &calendar_url, &tz);
                if !report.skipped.is_empty() {
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error: promotion failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --list-sources: print configured sources and exit
    if list_sources {
        println!("Axon Scouting — configured opportunity sources\n");
        print_sources(&cfg.sources);
        return;
    }

    // --from-file: deprecated in favor of the obsidian-markdown source adapter.
    if let Some(ref ff) = from_file {
        eprintln!(
            "note: --from-file is deprecated. Add an obsidian-markdown source to scouting.json's \
             `sources` array instead (see scouting.config.example.json)."
        );
        run_from_file(ff, &cfg, &limit, &opp_emb_path, &database_path, &no_store);
        return;
    }

    // ---------------------------------------------------------------
    // Adapter resolution
    // ---------------------------------------------------------------
    // Strategy:
    //   --adapter <name>  → try source registry first, then API adapters
    //   no --adapter      → if sources[] configured, run all enabled sources
    //                        + API adapter default; otherwise, run euro_hackathons
    // ---------------------------------------------------------------

    let query = SearchQuery {
        query: query_text,
        location: loc,
        date_from,
        limit,
        ..Default::default()
    };

    // Case: --luma-calendar <cal-id> — one Luma calendar, ad hoc. The durable
    // form is a `luma-calendar` entry in scouting.json's sources[]; this is
    // the "try one before declaring it" path.
    if let Some(ref api_id) = luma_calendar {
        let adapter = match LumaAdapter::for_calendar(api_id.clone()) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        if emit_json {
            match fetch_json(&adapter, &query) {
                Ok(opps) => println!(
                    "{}",
                    serde_json::to_string_pretty(&opps).unwrap_or_default()
                ),
                Err(e) => {
                    eprintln!("fetch error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }

        println!("Axon Scouting — opportunity discovery\n");
        print_run_header(&adapter, &cfg, &opp_emb_path, &database_path);

        match run_adapter(
            &adapter,
            &query,
            &cfg,
            RunOptions {
                opp_embeddings_path: opp_emb_path.as_deref(),
                database_path: &database_path,
                no_store,
                show_backlog,
                include_dismissed,
                limit,
            },
        ) {
            Ok(report) => {
                let store = Store::open(&database_path).ok();
                print_results(&report, store.as_ref(), include_dismissed);
            }
            Err(e) => println!("  pipeline error: {e}"),
        }
        return;
    }

    // Case: explicit --adapter flag
    if let Some(ref name) = adapter_name {
        // Warn about unverified API adapters. `luma` came off this list on
        // 2026-07-30 when it was finally run against Luma for real (see its
        // module header and README § Verdict).
        if matches!(name.as_str(), "meetup" | "cfp" | "cfp_conferences") {
            eprintln!(
                "warning: adapter '{name}' is unverified against its live API (fixture-tested only) -- see README Gotchas\n"
            );
        }

        // A declared source wins over a built-in of the same name, and a
        // declared source that cannot run stops here rather than handing the
        // question to the built-in list. One resolved adapter, one run block:
        // the two copies this replaced were where the fallthrough hid.
        let adapter = match resolve_source_adapter(name, &cfg.sources, no_store) {
            SourceLookup::Found(adapter) => adapter,
            SourceLookup::Failed(detail) => {
                eprintln!("error: {detail}");
                std::process::exit(1);
            }
            SourceLookup::NotDeclared => match build_api_adapter(name, no_store) {
                Some(adapter) => adapter,
                None => {
                    eprintln!("error: {}", unknown_adapter_error(name, &cfg.sources));
                    std::process::exit(1);
                }
            },
        };

        if emit_json {
            match fetch_json(&*adapter, &query) {
                Ok(opps) => println!(
                    "{}",
                    serde_json::to_string_pretty(&opps).unwrap_or_default()
                ),
                Err(e) => {
                    eprintln!("fetch error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }

        println!("Axon Scouting — opportunity discovery\n");
        print_run_header(&*adapter, &cfg, &opp_emb_path, &database_path);

        match run_adapter(
            &*adapter,
            &query,
            &cfg,
            RunOptions {
                opp_embeddings_path: opp_emb_path.as_deref(),
                database_path: &database_path,
                no_store,
                show_backlog,
                include_dismissed,
                limit,
            },
        ) {
            Ok(report) => {
                let store = Store::open(&database_path).ok();
                print_results(&report, store.as_ref(), include_dismissed);
            }
            Err(e) => println!("  pipeline error: {e}"),
        }
        return;
    }

    // Case: no --adapter flag — run all enabled sources + default API adapter
    let enabled_sources: Vec<&SourceManifest> = cfg.sources.iter().filter(|s| s.enabled).collect();

    // Multiple sources → merged cross-source ranking is the default (see
    // merge.rs). --no-merge preserves the per-source blocks below exactly.
    // --emit-json and --backlog keep their existing per-source semantics.
    if !no_merge && enabled_sources.len() > 1 && !emit_json && !show_backlog {
        run_merged_sources(
            &enabled_sources,
            &query,
            &cfg,
            &opp_emb_path,
            &database_path,
            no_store,
            include_dismissed,
            limit,
        );
        return;
    }

    let mut ran_any = false;

    // 1. Config-driven sources
    for manifest in &enabled_sources {
        let adapter = match scouting::sources::create_adapter(manifest) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("  error creating source '{}': {e}", manifest.id);
                continue;
            }
        };

        if emit_json {
            match fetch_json(&*adapter, &query) {
                Ok(opps) => println!(
                    "{}",
                    serde_json::to_string_pretty(&opps).unwrap_or_default()
                ),
                Err(_e) => eprintln!("fetch error ({})", manifest.id),
            }
            continue;
        }

        if !ran_any {
            println!("Axon Scouting — opportunity discovery (multi-source)\n");
        }
        ran_any = true;

        println!("  ── source: {} ({}) ──", manifest.id, manifest.adapter);
        print_run_header(&*adapter, &cfg, &opp_emb_path, &database_path);

        match run_adapter(
            &*adapter,
            &query,
            &cfg,
            RunOptions {
                opp_embeddings_path: opp_emb_path.as_deref(),
                database_path: &database_path,
                no_store,
                show_backlog,
                include_dismissed,
                limit,
            },
        ) {
            Ok(report) => {
                let store = Store::open(&database_path).ok();
                print_results(&report, store.as_ref(), include_dismissed);
            }
            Err(e) => println!("  pipeline error: {e}"),
        }
    }

    // 2. Default API adapter (euro_hackathons) — only if no sources are configured
    //    and we haven't run anything yet. If sources are configured, the user
    //    explicitly chose what to run.
    if !ran_any && enabled_sources.is_empty() {
        // A literal this file owns, so `expect` here is a bug in this file
        // rather than anything an operator can trigger — the API_ADAPTERS test
        // below pins the name.
        let adapter = build_api_adapter("euro_hackathons", no_store)
            .expect("euro_hackathons is a built-in adapter name");

        if emit_json {
            match fetch_json(&*adapter, &query) {
                Ok(opps) => println!(
                    "{}",
                    serde_json::to_string_pretty(&opps).unwrap_or_default()
                ),
                Err(e) => eprintln!("fetch error: {e}"),
            }
            return;
        }

        println!("Axon Scouting — opportunity discovery\n");
        print_run_header(&*adapter, &cfg, &opp_emb_path, &database_path);

        match run_adapter(
            &*adapter,
            &query,
            &cfg,
            RunOptions {
                opp_embeddings_path: opp_emb_path.as_deref(),
                database_path: &database_path,
                no_store,
                show_backlog,
                include_dismissed,
                limit,
            },
        ) {
            Ok(report) => {
                let store = Store::open(&database_path).ok();
                print_results(&report, store.as_ref(), include_dismissed);
            }
            Err(e) => println!("  pipeline error: {e}"),
        }
    } else if !ran_any {
        println!("  no enabled sources found — check scouting.json");
    }
}

// ---------------------------------------------------------------------------
// Deprecated --from-file path (kept for backwards compat)
// ---------------------------------------------------------------------------

fn run_from_file(
    path: &str,
    cfg: &Config,
    limit: &usize,
    opp_emb_path: &Option<String>,
    database_path: &std::path::Path,
    no_store: &bool,
) {
    let telos: Vec<TelosProfile> =
        load_telos_profiles(&cfg.interest_profile_dir.to_string_lossy(), &cfg.sources);
    let events_dir = cfg.events_dir.as_deref();

    let body = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    let opps: Vec<Opportunity> = serde_json::from_str(&body).unwrap_or_else(|e| {
        eprintln!("cannot parse JSON: {e}");
        std::process::exit(1);
    });

    let opp_embeddings = opp_emb_path.as_ref().map(|p| load_opp_embeddings(p));
    let mut store: Option<Store> = if *no_store {
        None
    } else {
        Store::open(database_path).ok()
    };

    println!("Axon Scouting — past-event retro calibration (deprecated --from-file)\n");
    println!("  source     : {path} ({} opportunities)", opps.len());
    println!(
        "  interests  : {} profiles from {}",
        telos.len(),
        cfg.interest_profile_dir.display()
    );
    if let Some(ref emb) = opp_embeddings {
        println!("  embeddings : {} pre-computed e5 vectors", emb.len());
    }
    if store.is_some() {
        println!("  store      : {}", database_path.display());
    }
    if let Some(events_dir) = events_dir {
        println!("  event link : {} (annotate-only)", events_dir.display());
    }
    println!();

    let mut scored = score(&opps, &telos, opp_embeddings.as_ref());
    let mut new_count = 0;
    let mut vault_links = 0;

    for s in &mut scored {
        let vault_link =
            events_dir.and_then(|dir| vault_linker::link_to_vault(&s.opportunity, dir));
        if let Some(ref vl) = vault_link {
            vault_links += 1;
            s.rationale = format!("{}\n     vault link: {vl}", s.rationale);
        }
        if let Some(st) = store.as_mut() {
            let is_new = st
                .upsert(
                    &s.opportunity,
                    s.score,
                    s.matched_focus.as_deref(),
                    &s.rationale,
                    vault_link.as_deref(),
                )
                .unwrap_or(false);
            if is_new {
                new_count += 1;
            }
        }
    }

    let store_total = match &store {
        Some(st) => st.count().unwrap_or(0),
        None => 0,
    };
    let show_limit = (*limit).min(scored.len());

    println!(
        "  ranked backlog ({} of {} scored, {} new, {} vault links, {} in store):\n",
        show_limit,
        scored.len(),
        new_count,
        vault_links,
        store_total
    );

    for (i, s) in scored.iter().take(show_limit).enumerate() {
        let focus = s.matched_focus.as_deref().unwrap_or("none");
        let dates = s.opportunity.starts_at.as_deref().unwrap_or("TBD");
        let loc = s.opportunity.city.as_deref().unwrap_or("Unknown");
        let raw_cat = s
            .opportunity
            .raw
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("  {}. [{:.3}] {}", i + 1, s.score, s.opportunity.title);
        println!(
            "     {} · {} · focus: {} · category: {}",
            dates, loc, focus, raw_cat
        );
        println!("     {}", s.rationale);
        println!("     {}", s.opportunity.url);
        println!();
    }

    if scored.len() > show_limit {
        println!(
            "  ... {} more (raise --limit to see all)",
            scored.len() - show_limit
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scouting::sources::SourceEntry;

    fn manifests(json: serde_json::Value) -> Vec<SourceManifest> {
        serde_json::from_value::<Vec<SourceEntry>>(json)
            .expect("source entries")
            .iter()
            .map(SourceEntry::resolve)
            .collect()
    }

    fn one_enabled_and_one_disabled() -> Vec<SourceManifest> {
        manifests(serde_json::json!([
            { "id": "events-radar", "adapter": "rss", "url": "https://example.test/a" },
            { "id": "old-radar", "adapter": "rss", "url": "https://example.test/b", "enabled": false }
        ]))
    }

    /// The defect this module's catch-all used to have: any unrecognized name
    /// built EuroHackathons, so a typo fetched hackathons and reported success.
    /// A run that answers the wrong question while looking healthy is worse
    /// than one that stops.
    #[test]
    fn an_unknown_adapter_name_builds_nothing() {
        assert!(build_api_adapter("meetupp", true).is_none());
        assert!(build_api_adapter("", true).is_none());
        assert!(build_api_adapter("splash-hub", true).is_none());
    }

    #[test]
    fn every_built_in_name_this_lists_actually_builds() {
        for name in API_ADAPTERS {
            // transit_fare exits rather than returning when its overlay config
            // is absent, which is a different failure with its own message.
            if name == "transit_fare" {
                continue;
            }
            assert!(
                build_api_adapter(name, true).is_some(),
                "API_ADAPTERS lists '{name}' but build_api_adapter does not build it"
            );
        }
    }

    /// The default no-flag path calls this by literal.
    #[test]
    fn the_default_adapter_is_a_name_the_builder_knows() {
        assert!(API_ADAPTERS.contains(&"euro_hackathons"));
        assert!(build_api_adapter("euro_hackathons", true).is_some());
    }

    #[test]
    fn a_name_no_source_declares_is_left_to_the_built_ins() {
        assert!(matches!(
            resolve_source_adapter("luma", &one_enabled_and_one_disabled(), true),
            SourceLookup::NotDeclared
        ));
    }

    #[test]
    fn a_declared_enabled_source_resolves_to_its_own_adapter() {
        match resolve_source_adapter("events-radar", &one_enabled_and_one_disabled(), true) {
            SourceLookup::Found(adapter) => assert_eq!(adapter.name(), "events-radar"),
            _ => panic!("a declared, enabled source must resolve"),
        }
    }

    /// Disabled is a decision, not an absence. Falling through would run a
    /// built-in in place of the thing the operator deliberately turned off.
    #[test]
    fn a_disabled_source_stops_rather_than_falling_through() {
        match resolve_source_adapter("old-radar", &one_enabled_and_one_disabled(), true) {
            SourceLookup::Failed(detail) => {
                assert!(detail.contains("old-radar"), "got: {detail}");
                assert!(detail.contains("disabled"), "got: {detail}");
            }
            _ => panic!("a disabled source must not resolve, and must not fall through"),
        }
    }

    /// The worse half of the old bug: this case already printed its error, and
    /// then ran EuroHackathons anyway.
    #[test]
    fn a_declared_source_that_cannot_build_stops_rather_than_falling_through() {
        let broken = manifests(serde_json::json!([
            { "id": "broken-radar", "adapter": "luma-calendar" }
        ]));
        match resolve_source_adapter("broken-radar", &broken, true) {
            SourceLookup::Failed(detail) => {
                assert!(detail.contains("broken-radar"), "got: {detail}");
                assert_eq!(
                    detail.matches("broken-radar").count(),
                    1,
                    "the source is named once; twice reads like two separate failures: {detail}"
                );
            }
            _ => panic!("a source whose config cannot build must not fall through"),
        }
    }

    #[test]
    fn the_unknown_adapter_error_names_both_halves_of_what_it_could_have_been() {
        let message = unknown_adapter_error("meetupp", &one_enabled_and_one_disabled());
        assert!(
            message.contains("meetupp"),
            "the error quotes what was typed"
        );
        assert!(
            message.contains("events-radar"),
            "and lists declared sources"
        );
        assert!(
            message.contains("old-radar (disabled)"),
            "a disabled source is named as disabled, not omitted -- omitting it reads as \
             'not configured' and sends the operator to write an entry that already exists"
        );
        assert!(
            message.contains("euro_hackathons"),
            "and lists the built-ins"
        );
    }

    #[test]
    fn with_nothing_declared_the_error_says_so_rather_than_showing_an_empty_list() {
        let message = unknown_adapter_error("whatever", &[]);
        assert!(message.contains("(none declared)"), "got: {message}");
    }
}

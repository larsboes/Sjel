use std::path::PathBuf;

use transit::config::Config;
use transit::extractor;
use transit::hafas::HafasClient;

fn get_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// CLI flag wins; falls back to the overlay config value; errors with a
/// clear message (not a silent personal default) if neither is set.
fn require(
    flag_value: Option<String>,
    config_value: Option<String>,
    flag: &str,
    config_field: &str,
) -> Result<String, String> {
    flag_value.or(config_value).ok_or_else(|| {
        format!(
            "missing {flag} and no {config_field} configured -- pass {flag} <value> or set {config_field} in transit.config.json (see transit.config.example.json)"
        )
    })
}

fn print_usage() {
    eprintln!("Sjel Transit -- HAFAS journey search, split-ticket solver, ticket extraction\n");
    eprintln!("Usage:");
    eprintln!("  transit suggest --query <station name>");
    eprintln!("  transit search  --from <EVA> --to <EVA> --time <ISO datetime>");
    eprintln!("  transit split   --from <EVA> --to <EVA> --time <ISO datetime>");
    eprintln!("  transit import  <file>");
    eprintln!("  transit plan    --from <EVA> --destinations <city,city,...> --date-from <YYYY-MM-DD> --date-to <YYYY-MM-DD>");
    eprintln!(
        "                 [--intent <text>] [--time HH:MM] [--step-days N] [--max-queries N]"
    );
    eprintln!("                 [--candidates-per-dest N] [--dry-run] [--show <session-id>]");
    eprintln!(
        "  transit plan    --from <EVA> --destinations <city,city,...> --dates <YYYY-MM-DD,...>"
    );
    eprintln!("                 (same options; --dates searches exactly those days instead of");
    eprintln!(
        "                  sampling a window -- mutually exclusive with --date-from/--date-to)"
    );
    eprintln!();
    eprintln!("--from/--to/--time fall back to default_from_eva/default_to_eva/default_time");
    eprintln!("in the overlay config if set (see transit.config.example.json); otherwise all");
    eprintln!("three must be passed explicitly -- there is no baked-in station default.");
    eprintln!();
    eprintln!("plan: fuzzy/triggered trip-search session (correlation driving query #2). Resolves");
    eprintln!("soft destination names to EVA candidates, samples departure dates across the");
    eprintln!("window, fans a fresh HAFAS search out over (candidate x date), records every");
    eprintln!("found journey to a session row, and prints the ranked summary -- cheapest first.");
}

// ── Phase 3: date math for the trip-search session ──────────────────────
//
// No `chrono` dependency (the crate deliberately stays sync + minimal, see
// Cargo.toml's header comment). The session sampler only needs proleptic-
// Gregorian day arithmetic over a window, so the classic days-from-civil
// algorithm (Howard Hinnant) is enough -- parse `YYYY-MM-DD` into a day
// count, step, format back. ~25 lines beats a transitive dep tree for this.

/// `YYYY-MM-DD` -> days since 1970-01-01 (proleptic Gregorian). Returns
/// `None` on a malformed/unparseable date rather than panicking -- a bad
/// `--date-from`/`--date-to` is a user error to print + exit, not a crash.
fn parse_date_days(s: &str) -> Option<i64> {
    let mut parts = s.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// days-since-1970 -> `YYYY-MM-DD`. Inverse of `parse_date_days`.
fn fmt_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's `days_from_civil` -- no validation beyond what
/// `parse_date_days` already does; assumes a well-formed y/m/d. The first
/// step (`y -= m <= 2`) shifts the year boundary to March so that the
/// 5-month `doy` pattern's leap day is always the *last* day of the year,
/// not a special case inside it.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil` (`civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Evenly samples departure dates across `[date_from, date_to]` (inclusive).
/// `step_days` is the stride hint; the actual stride is stretched so the last
/// sample lands on or before `date_to`, and the final day of the window is
/// always included -- the cheap fares a "maybe mid-September" query wants
/// are often near month-end. `max_points` is the cap driven by `--max-queries`
/// divided across candidates (see `run_plan`).
fn sample_dates(date_from: i64, date_to: i64, step_days: i64, max_points: usize) -> Vec<i64> {
    if date_to <= date_from || step_days <= 0 || max_points == 0 {
        return vec![date_from];
    }
    let mut out = vec![date_from];
    let span = date_to - date_from;
    let by_step = (span / step_days) as usize;
    let n = by_step.min(max_points.saturating_sub(1));
    if n == 0 {
        if date_to > date_from {
            out.push(date_to);
        }
        return out;
    }
    for i in 1..=n {
        out.push(date_from + (span * i as i64) / n as i64);
    }
    if *out.last().unwrap() != date_to {
        out.push(date_to);
    }
    out
}

/// Parses `--dates 2026-08-14,2026-08-15,...` into sorted, deduped day counts.
///
/// This is the constrained half of the sampler: instead of guessing which days
/// in a window are worth a fare search, the caller states them. The intended
/// producer is `capabilities/calendar`, whose feasible-windows endpoint derives
/// exactly this list from the operator's real availability -- transit stays
/// unaware of that capability and just honours the days it is handed (see the
/// README's fuzzy-trip-search section).
fn parse_dates(csv: &str) -> Result<Vec<i64>, String> {
    let mut days = Vec::new();
    for token in csv.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let day = parse_date_days(token)
            .ok_or_else(|| format!("--dates entry must be YYYY-MM-DD, got \"{token}\""))?;
        days.push(day);
    }
    if days.is_empty() {
        return Err("--dates needs at least one YYYY-MM-DD day".into());
    }
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

/// Thins an explicit day list down to `max_points`, keeping the first and last
/// and spreading the rest evenly -- the same shrink-to-fit contract
/// `sample_dates` gives a window, so `--max-queries` caps both paths
/// identically and a month of feasible days never silently fans out into a
/// month of HAFAS calls.
fn pick_dates(days: &[i64], max_points: usize) -> Vec<i64> {
    let n = max_points.max(1);
    if days.len() <= n {
        return days.to_vec();
    }
    if n == 1 {
        return vec![days[0]];
    }
    // days.len() > n means the stride is at least one index, so these land on
    // n distinct days, first and last included.
    let last = days.len() - 1;
    (0..n).map(|i| days[(last * i) / (n - 1)]).collect()
}

/// Resolves a comma-separated list of soft destination names ("Valencia,
/// Copenhagen") into concrete EVA candidates via `HafasClient::suggest_stations`.
/// Takes up to `per_dest` candidates per name (default 1 = the main station).
/// A name resolving to nothing is *not* fatal -- the candidate set just
/// shrinks -- but it prints a warning so the user knows a destination
/// dropped silently.
fn resolve_candidates(
    client: &HafasClient,
    destinations_csv: &str,
    per_dest: usize,
) -> Result<Vec<transit::store::CandidateDest>, String> {
    let mut out = Vec::new();
    for raw in destinations_csv.split(',') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match client.suggest_stations(name) {
            Ok(stations) => {
                if stations.is_empty() {
                    eprintln!("warning: \"{name}\" resolved to no station -- dropped from the candidate set");
                    continue;
                }
                for s in stations.into_iter().take(per_dest.max(1)) {
                    out.push(transit::store::CandidateDest {
                        eva: s.id,
                        name: s.name,
                    });
                }
            }
            Err(e) => return Err(format!("could not resolve \"{name}\": {e}")),
        }
    }
    if out.is_empty() {
        return Err(
            "no destinations resolved -- pass at least one resolvable city in --destinations"
                .into(),
        );
    }
    Ok(out)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cfg = Config::load();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "suggest" => {
            let Some(query) = get_flag(&args, "--query") else {
                eprintln!("error: --query is required");
                std::process::exit(1);
            };
            let client = HafasClient::new();
            match client.suggest_stations(&query) {
                Ok(stations) => println!(
                    "{}",
                    serde_json::to_string_pretty(&stations).unwrap_or_default()
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "search" | "split" => {
            let from = match require(
                get_flag(&args, "--from"),
                cfg.default_from_eva.clone(),
                "--from",
                "default_from_eva",
            ) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let to = match require(
                get_flag(&args, "--to"),
                cfg.default_to_eva.clone(),
                "--to",
                "default_to_eva",
            ) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let time = match require(
                get_flag(&args, "--time"),
                cfg.default_time.clone(),
                "--time",
                "default_time",
            ) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };

            let client = HafasClient::new();
            if cmd == "search" {
                eprintln!("Searching direct connections from {from} to {to} at {time}...");
                match client.search_connections(&from, &to, &time, &Default::default()) {
                    Ok(journeys) => println!(
                        "{}",
                        serde_json::to_string_pretty(&journeys).unwrap_or_default()
                    ),
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!(
                    "Calculating cheapest split-ticket connection from {from} to {to} at {time}..."
                );
                match client.search_split_tickets(&from, &to, &time, &Default::default()) {
                    Ok(result) => println!(
                        "{}",
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    ),
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        "plan" => run_plan(&args, &cfg),
        "import" => {
            let Some(path) = args.get(2).cloned() else {
                eprintln!("error: transit import <file> requires a file path");
                std::process::exit(1);
            };
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: cannot read {path}: {e}");
                    std::process::exit(1);
                }
            };
            let file_name = PathBuf::from(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path)
                .to_string();
            match extractor::extract_from_bytes(&bytes, &file_name) {
                Ok(extracted) => println!(
                    "{}",
                    serde_json::to_string_pretty(&extracted).unwrap_or_default()
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => print_usage(),
    }
}

/// `transit plan` -- the fuzzy/triggered trip-search session
/// (`capabilities/store/README.md` driving query #2: "in September I feel
/// like a trip"). See `print_usage`
/// for the flags. Three modes share this entry:
///   - `--show <session-id>`: re-list an existing session's ranked trips
///     without touching the network (read-only).
///   - `--dry-run`: resolve candidates + sample dates, print the planned
///     session shape, exit without searching. Lets you sanity-check a fuzzy
///     intent before paying for N HAFAS calls.
///   - default: upsert the session, fan the search out, record, print the
///     ranked summary.
fn run_plan(args: &[String], cfg: &Config) {
    // Read-only re-list -- no network, no DB writes beyond the open.
    if let Some(show_id) = get_flag(args, "--show") {
        let store = match open_store(cfg) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        let session = match store.get_session(&show_id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                eprintln!("error: no session with id {show_id}");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("error: could not read session: {e}");
                std::process::exit(1);
            }
        };
        let trips = match store.list_session_trips(&show_id) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: could not list session trips: {e}");
                std::process::exit(1);
            }
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&session_summary(&session, &trips)).unwrap_or_default()
        );
        return;
    }

    // Required: origin + destinations + a date window.
    let from = match require(
        get_flag(args, "--from"),
        cfg.default_from_eva.clone(),
        "--from",
        "default_from_eva",
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let Some(dests_csv) = get_flag(args, "--destinations") else {
        eprintln!("error: --destinations <city,city,...> is required (e.g. --destinations \"Valencia,Copenhagen\")");
        std::process::exit(1);
    };
    // Two ways to say *when*: a window the sampler thins, or an explicit day
    // list somebody else already thought about (calendar's feasible windows).
    // Mutually exclusive on purpose -- a silent precedence rule between "the
    // whole of September" and "these four days" would hide which one ran.
    let dates_flag = get_flag(args, "--dates");
    if dates_flag.is_some()
        && (get_flag(args, "--date-from").is_some() || get_flag(args, "--date-to").is_some())
    {
        eprintln!("error: --dates and --date-from/--date-to are mutually exclusive -- pass the explicit days or the window, not both");
        std::process::exit(1);
    }
    let explicit_dates = match &dates_flag {
        Some(csv) => match parse_dates(csv) {
            Ok(days) => Some(days),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let (date_from, date_to, date_from_raw, date_to_raw) = match &explicit_dates {
        // The session row still records a span; it is the one the given days
        // actually cover.
        Some(days) => {
            let (first, last) = (days[0], days[days.len() - 1]);
            (first, last, fmt_date(first), fmt_date(last))
        }
        None => {
            let Some(date_from_raw) = get_flag(args, "--date-from") else {
                eprintln!("error: --date-from <YYYY-MM-DD> is required (or pass --dates <YYYY-MM-DD,...>)");
                std::process::exit(1);
            };
            let Some(date_to_raw) = get_flag(args, "--date-to") else {
                eprintln!(
                    "error: --date-to <YYYY-MM-DD> is required (or pass --dates <YYYY-MM-DD,...>)"
                );
                std::process::exit(1);
            };
            let Some(date_from) = parse_date_days(&date_from_raw) else {
                eprintln!("error: --date-from must be YYYY-MM-DD, got \"{date_from_raw}\"");
                std::process::exit(1);
            };
            let Some(date_to) = parse_date_days(&date_to_raw) else {
                eprintln!("error: --date-to must be YYYY-MM-DD, got \"{date_to_raw}\"");
                std::process::exit(1);
            };
            if date_to < date_from {
                eprintln!(
                    "error: --date-to ({date_to_raw}) is before --date-from ({date_from_raw})"
                );
                std::process::exit(1);
            }
            (date_from, date_to, date_from_raw, date_to_raw)
        }
    };

    let intent = get_flag(args, "--intent").unwrap_or_else(|| dests_csv.clone());
    let time_of_day = get_flag(args, "--time").unwrap_or_else(|| "08:00".into());
    let step_days: i64 = get_flag(args, "--step-days")
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let max_queries: usize = get_flag(args, "--max-queries")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let per_dest: usize = get_flag(args, "--candidates-per-dest")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let client = HafasClient::new();
    let candidates = match resolve_candidates(&client, &dests_csv, per_dest) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    // Cap the (candidate x date) search count before sampling so a wide
    // window never silently hammers bahn.de. The date count shrinks to fit,
    // whether the days came from the sampler or from --dates.
    let max_dates = max_queries / candidates.len().max(1);
    let sampled = match &explicit_dates {
        Some(days) => pick_dates(days, max_dates.max(1)),
        None => sample_dates(date_from, date_to, step_days, max_dates.max(1)),
    };
    let total_queries = candidates.len() * sampled.len();
    if total_queries > max_queries {
        eprintln!(
            "error: planned {total_queries} HAFAS queries ({} candidates x {} dates) exceeds --max-queries {max_queries} -- widen --step-days, narrow --destinations, or raise --max-queries",
            candidates.len(),
            sampled.len()
        );
        std::process::exit(1);
    }

    let session_id = transit::store::stable_session_id(
        &from,
        &candidates,
        &date_from_raw,
        &date_to_raw,
        &intent,
    );
    eprintln!("plan: intent \"{intent}\"");
    eprintln!(
        "plan: origin EVA {from} -> {} candidate destination(s):",
        candidates.len()
    );
    for c in &candidates {
        eprintln!("    {}  {}", c.eva, c.name);
    }
    match &explicit_dates {
        Some(days) => eprintln!(
            "plan: {} given day(s) across {date_from_raw}..{date_to_raw}, searching {}:",
            days.len(),
            sampled.len()
        ),
        None => eprintln!(
            "plan: date window {date_from_raw}..{date_to_raw}, sampling {} departure date(s):",
            sampled.len()
        ),
    }
    for d in &sampled {
        eprintln!("    {}", fmt_date(*d));
    }
    eprintln!("plan: session id {session_id}");

    if dry_run {
        eprintln!("plan: --dry-run set, no search performed");
        let dry = serde_json::json!({
            "session_id": session_id,
            "intent": intent,
            "origin_eva": from,
            "candidates": candidates,
            "date_start": date_from_raw,
            "date_end": date_to_raw,
            // Names which of the two "when" shapes ran, so a caller feeding in
            // calendar's feasible days can verify the constraint took effect
            // instead of inferring it from the sampled list.
            "date_source": if explicit_dates.is_some() { "explicit" } else { "window" },
            "sampled_dates": sampled.iter().map(|d| fmt_date(*d)).collect::<Vec<_>>(),
            "planned_queries": total_queries,
            "dry_run": true,
        });
        println!("{}", serde_json::to_string_pretty(&dry).unwrap_or_default());
        return;
    }

    let store = match open_store(cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not open transit store: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = store.upsert_session(
        &session_id,
        &from,
        &intent,
        &candidates,
        &date_from_raw,
        &date_to_raw,
    ) {
        eprintln!("error: could not record session: {e}");
        std::process::exit(1);
    }

    let mut found = 0usize;
    let mut errors = 0usize;
    for cand in &candidates {
        for d in &sampled {
            let departure = format!("{}T{}:00", fmt_date(*d), time_of_day);
            // Same 250ms inter-request cadence the split-ticket solver uses
            // (hafas.rs) -- a polite pace against an undocumented endpoint.
            if found + errors > 0 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            match client.search_connections(&from, &cand.eva, &departure, &Default::default()) {
                Ok(journeys) => {
                    for j in &journeys {
                        match store.record_journey(
                            j,
                            &from,
                            &cand.eva,
                            "session",
                            Some(&session_id),
                        ) {
                            Ok(_) => found += 1,
                            Err(e) => {
                                eprintln!("warning: could not record journey {}: {e}", j.id);
                                errors += 1;
                            }
                        }
                    }
                    if journeys.is_empty() {
                        eprintln!(
                            "    {} {} -> no journeys found at {}",
                            cand.eva, cand.name, departure
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "warning: search {} -> {} at {} failed: {e}",
                        from, cand.eva, departure
                    );
                    errors += 1;
                }
            }
        }
    }
    eprintln!("plan: recorded {found} journey/journeys across the session ({errors} error/s)");

    let session = store
        .get_session(&session_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| transit::store::SessionRow {
            id: session_id.clone(),
            origin_eva: from.clone(),
            intent: intent.clone(),
            candidates: candidates.clone(),
            date_start: date_from_raw.clone(),
            date_end: date_to_raw.clone(),
            status: "new".into(),
            created_at: String::new(),
        });
    let trips = store.list_session_trips(&session_id).unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&session_summary(&session, &trips)).unwrap_or_default()
    );
}

fn open_store(cfg: &Config) -> Result<transit::store::TransitStore, String> {
    transit::store::TransitStore::open(&cfg.database_path).map_err(|e| e.to_string())
}

/// Builds the JSON summary printed by `plan` and `plan --show`: the session
/// shape plus its ranked trips (cheapest-first, the order
/// `list_session_trips` already returns). Each trip is flattened to the
/// handful of fields a "I feel like a trip, what's cheap?" decision actually
/// needs -- full structured legs stay queryable in `transit.trips`/`trip_legs`
/// for anything that wants the detail.
fn session_summary(
    session: &transit::store::SessionRow,
    trips: &[(transit::store::TripRow, Vec<transit::store::TripLegRow>)],
) -> serde_json::Value {
    let trip_summaries: Vec<serde_json::Value> = trips
        .iter()
        .map(|(t, legs)| {
            let first_dep = legs.first().map(|l| l.departure_time.clone()).unwrap_or_else(|| t.created_at.clone());
            serde_json::json!({
                "trip_id": t.id,
                "destination": legs.last().map(|l| l.destination_name.clone()).unwrap_or_else(|| t.destination_eva.clone()),
                "price": t.total_price,
                "duration_minutes": t.total_duration_minutes,
                "departure": first_dep,
                "status": t.status,
                "legs": legs.len(),
            })
        })
        .collect();
    serde_json::json!({
        "session_id": session.id,
        "intent": session.intent,
        "origin_eva": session.origin_eva,
        "candidates": session.candidates,
        "date_start": session.date_start,
        "date_end": session.date_end,
        "status": session.status,
        "trips": trip_summaries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_date_days_round_trips_and_rejects_garbage() {
        assert_eq!(
            parse_date_days("2026-09-01"),
            Some(days_from_civil(2026, 9, 1))
        );
        assert_eq!(
            fmt_date(parse_date_days("2026-09-01").unwrap()),
            "2026-09-01"
        );
        // Leap day survives the round trip.
        assert_eq!(
            fmt_date(parse_date_days("2024-02-29").unwrap()),
            "2024-02-29"
        );
        // Malformed inputs -> None, never panic.
        assert_eq!(parse_date_days("not-a-date"), None);
        // Non-padded components parse leniently (integer parsing doesn't
        // care about zero-padding) -- friendlier for a fuzzy CLI than rigid
        // ISO enforcement. The separator discipline catches real garbage:
        assert_eq!(
            parse_date_days("2026/09/01"),
            None,
            "wrong separator must be rejected"
        );
        assert_eq!(parse_date_days("2026-13-01"), None, "month out of range");
        assert_eq!(parse_date_days("2026-09-32"), None, "day out of range");
        assert_eq!(parse_date_days("2026-09-01-extra"), None);
    }

    #[test]
    fn sample_dates_always_includes_end_and_start() {
        // Sept 1 .. Sept 30, step 7 -- the canonical "in September" query.
        let df = parse_date_days("2026-09-01").unwrap();
        let dt = parse_date_days("2026-09-30").unwrap();
        let dates = sample_dates(df, dt, 7, 10);
        assert_eq!(
            dates.first().copied(),
            Some(df),
            "window start must be sampled"
        );
        assert_eq!(
            dates.last().copied(),
            Some(dt),
            "window end must be sampled even if step doesn't land on it"
        );
        // Monotonic non-decreasing, no duplicates that matter for price sampling.
        assert!(dates.windows(2).all(|w| w[0] <= w[1]));
        assert!(dates.len() <= 10, "should respect max_points cap");
    }

    #[test]
    fn sample_dates_short_window_returns_start_and_end_only() {
        // A 3-day window with a 7-day step can't fit any mid sample -- the
        // helper falls back to start + end rather than dropping the end.
        let df = parse_date_days("2026-09-01").unwrap();
        let dt = parse_date_days("2026-09-03").unwrap();
        let dates = sample_dates(df, dt, 7, 10);
        assert_eq!(dates, vec![df, dt]);
    }

    #[test]
    fn sample_dates_respects_max_points() {
        let df = parse_date_days("2026-09-01").unwrap();
        let dt = parse_date_days("2026-09-30").unwrap();
        let dates = sample_dates(df, dt, 1, 4);
        assert!(
            dates.len() <= 4,
            "max_points caps the count even for a fine step"
        );
        assert!(dates.len() >= 2, "start + end always present");
    }

    #[test]
    fn parse_dates_sorts_dedupes_and_rejects_garbage() {
        let days = parse_dates("2026-08-16, 2026-08-14,2026-08-16").unwrap();
        assert_eq!(
            days.iter().map(|d| fmt_date(*d)).collect::<Vec<_>>(),
            ["2026-08-14", "2026-08-16"],
            "a day list arrives sorted and deduped, whatever order it was written in"
        );
        // Trailing separators are the shape a shell pipeline produces.
        assert_eq!(parse_dates("2026-08-14,").unwrap().len(), 1);
        assert!(parse_dates("2026-08-14,tuesday").is_err());
        assert!(
            parse_dates("").is_err(),
            "an empty list is a mistake, not a no-op"
        );
    }

    #[test]
    fn pick_dates_thins_to_the_query_budget_keeping_both_ends() {
        let days: Vec<i64> = (0..30)
            .map(|i| parse_date_days("2026-08-01").unwrap() + i)
            .collect();
        let picked = pick_dates(&days, 4);
        assert_eq!(picked.len(), 4);
        assert_eq!(picked.first(), days.first());
        assert_eq!(picked.last(), days.last());
        assert!(picked.windows(2).all(|w| w[0] < w[1]), "must stay ordered");

        // Under budget, every given day is searched -- the point of --dates.
        let short = &days[..3];
        assert_eq!(pick_dates(short, 10), short.to_vec());
        assert_eq!(pick_dates(&days, 1), vec![days[0]]);
    }

    #[test]
    fn days_round_trip_is_stable_for_known_dates() {
        // Sanity: 1970-01-01 is day 0, 2000-03-01 is a known anchor (Hinnant).
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        let days = days_from_civil(2026, 9, 1);
        assert_eq!(civil_from_days(days), (2026, 9, 1));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}

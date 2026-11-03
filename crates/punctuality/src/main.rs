//! punctuality — ingest published DB stop history, read the resulting statistics.

use punctuality::config::Config;
use punctuality::dataset::{self, FIRST_FULL_COVERAGE_MONTH};
use punctuality::ingest::{self, CellKey};
use punctuality::stats::Cell;
use punctuality::store::{MonthRecord, Store};
use std::collections::{HashMap, HashSet};

const USAGE: &str = "\
Usage:
  punctuality ingest [--from YYYY-MM] [--to YYYY-MM]   download + incrementally aggregate monthly releases
  punctuality stats <station|eva> [--type ICE] [--min-n N]
  punctuality stations <needle>                        eva lookup by name
  punctuality ride --type ICE --number 611 --date YYYY-MM-DD [--eva 8000044]
                                                       one train's actual stops that day

Defaults: --from 2025-12 (first month covering every station, not just the largest ~100),
--to the newest published month, --min-n 30.";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ingest") => ingest_cmd(&args),
        Some("stats") => stats_cmd(&args),
        Some("stations") => stations_cmd(&args),
        Some("ride") => ride_cmd(&args),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("punctuality: {e}");
        std::process::exit(1);
    }
}

fn ingest_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let from = flag(args, "--from").unwrap_or_else(|| FIRST_FULL_COVERAGE_MONTH.to_string());
    let to = flag(args, "--to");

    let client = sjel_http::client(
        sjel_http::Purpose::new("punctuality-dataset"),
        std::time::Duration::from_secs(1800),
    )?;
    let months = dataset::select(dataset::list_months(&client)?, &from, to.as_deref())?;
    let store = Store::open(&cfg.database_path)?;
    let previous = store.month_manifest()?;
    let previous: HashMap<String, Option<String>> = previous
        .into_iter()
        .map(|month| (month.month, month.source_oid))
        .collect();
    let selected: HashSet<&str> = months.iter().map(|month| month.id.as_str()).collect();

    // An existing deployment has an aggregate but no source ledger. Rebuild once to
    // bootstrap the ledger; every later run can merge only new months.
    let bootstrap = previous.is_empty() && store.coverage()?.is_some();
    let removed = previous
        .keys()
        .any(|month| !selected.contains(month.as_str()));
    let changed = months
        .iter()
        .any(|month| previous.get(&month.id).is_some_and(|old| old != &month.oid));
    let new_months: Vec<_> = months
        .iter()
        .filter(|month| !previous.contains_key(&month.id))
        .collect();
    let rebuild = bootstrap || removed || changed;
    let work: Vec<_> = if rebuild {
        months.iter().collect()
    } else {
        new_months.clone()
    };

    eprintln!(
        "punctuality: {} month(s) {}..={}, {} to process ({}) cache {}",
        months.len(),
        months[0].id,
        months[months.len() - 1].id,
        work.len(),
        if rebuild { "rebuild" } else { "incremental" },
        cfg.raw_dir.display()
    );

    let mut cells: HashMap<CellKey, Cell> = HashMap::new();
    let mut stations: HashMap<String, String> = HashMap::new();
    let (mut rows, mut skipped) = (0u64, 0u64);
    let mut processed = Vec::with_capacity(work.len());

    for month in work {
        let refresh = previous.get(&month.id).is_some_and(|old| old != &month.oid);
        let path = dataset::ensure_local(&client, month, &cfg.raw_dir, refresh)?;
        let counts = ingest::fold_file(&path, &mut cells, &mut stations)?;
        rows += counts.rows;
        skipped += counts.skipped;
        processed.push(MonthRecord {
            month: month.id.clone(),
            source_oid: month.oid.clone(),
            rows_read: counts.rows as i64,
            rows_skipped: counts.skipped as i64,
        });
        eprintln!(
            "  {}  {:>11} rows  {:>8} skipped  {:>8} cells so far",
            month.id,
            counts.rows,
            counts.skipped,
            cells.len()
        );
    }

    if rebuild {
        store.replace_stats_and_months(&cells, &stations, &processed)?;
    } else if !processed.is_empty() {
        store.merge_stats_and_months(&cells, &stations, &processed)?;
    }

    let total_cells = store.coverage()?.map(|coverage| coverage.2).unwrap_or(0);
    store.record_run(
        &months[0].id,
        &months[months.len() - 1].id,
        months.len() as i32,
        rows as i64,
        skipped as i64,
        if rebuild {
            cells.len() as i32
        } else {
            total_cells
        },
    )?;

    println!(
        "{} cells from {} rows ({} skipped), processed {} of {} month(s), {} stations",
        if rebuild {
            cells.len() as i32
        } else {
            total_cells
        },
        rows,
        skipped,
        processed.len(),
        months.len(),
        stations.len()
    );
    Ok(())
}

fn stats_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let target = args.get(1).ok_or("stats needs a station name or eva")?;
    let train_type = flag(args, "--type");
    let min_n: i64 = flag(args, "--min-n").as_deref().unwrap_or("30").parse()?;

    let cfg = Config::load();
    let store = Store::open(&cfg.database_path)?;

    // A name is resolved through the stations table; digits are taken as an eva.
    let eva = if target.chars().all(|c| c.is_ascii_digit()) {
        target.clone()
    } else {
        let hits = store.find_stations(target)?;
        match hits.len() {
            0 => return Err(format!("no station matching '{target}'").into()),
            _ => {
                if hits.len() > 1 {
                    eprintln!(
                        "punctuality: '{target}' matched {} stations, using {}",
                        hits.len(),
                        hits[0].1
                    );
                }
                hits[0].0.clone()
            }
        }
    };

    let rows = store.station_stats(&eva, train_type.as_deref(), min_n)?;
    if rows.is_empty() {
        println!("no cells with n >= {min_n} for {eva}. Ingested anything yet?");
        return Ok(());
    }
    println!(
        "{} ({})    n >= {}",
        rows[0].station_name.clone().unwrap_or_else(|| "?".into()),
        rows[0].eva,
        min_n
    );
    println!(
        "{:<6} {:>3} {:<3} {:>7} {:>7} {:>5} {:>5} {:>8} {:>8}",
        "typ", "std", "we", "n", "mittel", "p50", "p90", ">=6min", "ausfall"
    );
    for r in &rows {
        println!(
            "{:<6} {:>3} {:<3} {:>7} {:>7.1} {:>5} {:>5} {:>7.1}% {:>7.1}%",
            r.train_type,
            r.hour,
            if r.weekend { "we" } else { "" },
            r.n,
            r.mean_delay,
            r.p50,
            r.p90,
            r.share_late_6 * 100.0,
            r.cancel_rate * 100.0
        );
    }
    Ok(())
}

fn stations_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let needle = args.get(1).ok_or("stations needs a search string")?;
    let cfg = Config::load();
    let store = Store::open(&cfg.database_path)?;
    for (eva, name) in store.find_stations(needle)? {
        println!("{eva}  {name}");
    }
    Ok(())
}

/// One train's actual stops on one day, straight from the cached monthly file.
///
/// Reads the columns `ingest` throws away. Deliberately CLI-only and
/// deliberately not stored: this is a lookup against files already on disk, and
/// giving it a table would create a second copy of DB's own published data.
fn ride_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let train_type = flag(args, "--type").ok_or("ride needs --type (e.g. ICE)")?;
    let number = flag(args, "--number").ok_or("ride needs --number (e.g. 611)")?;
    let date = flag(args, "--date").ok_or("ride needs --date YYYY-MM-DD")?;
    let eva = flag(args, "--eva");

    let answer =
        punctuality::ride::find(&cfg.raw_dir, &train_type, &number, &date, eva.as_deref())?;
    if answer.stops.is_empty() && answer.unavailable.is_none() {
        eprintln!(
            "punctuality: no stops for {train_type} {number} on {date}. Cached months: {}",
            punctuality::ride::cached_months(&cfg.raw_dir).join(", ")
        );
    }
    println!("{}", serde_json::to_string_pretty(&answer)?);
    Ok(())
}

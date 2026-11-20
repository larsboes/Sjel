//! Batch commands for the punctuality aggregate.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
};

use crate::{
    config::Config,
    dataset::{self, FIRST_FULL_COVERAGE_MONTH},
    ingest::{self, CellKey},
    stats::Cell,
    store::{MonthRecord, Store},
};

pub fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Run a command using the owning store.
///
/// # Errors
/// Returns an error when arguments, provider data or a database operation are invalid.
pub fn run(
    args: &[String],
    cfg: &Config,
    store: &Store,
) -> Result<String, Box<dyn std::error::Error>> {
    match args.first().map(String::as_str) {
        Some("ingest") => ingest_cmd(args, cfg, store),
        Some("stats") => stats_cmd(args, store),
        Some("stations") => stations_cmd(args, store),
        _ => Err("unknown punctuality store command".into()),
    }
}

fn ingest_cmd(
    args: &[String],
    cfg: &Config,
    store: &Store,
) -> Result<String, Box<dyn std::error::Error>> {
    let from = flag(args, "--from").unwrap_or_else(|| FIRST_FULL_COVERAGE_MONTH.to_string());
    let to = flag(args, "--to");

    let client = sjel_http::client(
        sjel_http::Purpose::new("punctuality-dataset"),
        std::time::Duration::from_secs(1800),
    )?;
    let months = dataset::select(dataset::list_months(&client)?, &from, to.as_deref())?;
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

    Ok(format!(
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
    ))
}

fn stats_cmd(args: &[String], store: &Store) -> Result<String, Box<dyn std::error::Error>> {
    let target = args.get(1).ok_or("stats needs a station name or eva")?;
    let train_type = flag(args, "--type");
    let min_n: i64 = flag(args, "--min-n").as_deref().unwrap_or("30").parse()?;

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
        return Ok(format!(
            "no cells with n >= {min_n} for {eva}. Ingested anything yet?"
        ));
    }
    let mut output = String::new();
    writeln!(
        output,
        "{} ({})    n >= {}",
        rows[0].station_name.clone().unwrap_or_else(|| "?".into()),
        rows[0].eva,
        min_n
    )?;
    writeln!(
        output,
        "{:<6} {:>3} {:<3} {:>7} {:>7} {:>5} {:>5} {:>8} {:>8}",
        "typ", "std", "we", "n", "mittel", "p50", "p90", ">=6min", "ausfall"
    )?;
    for r in &rows {
        writeln!(
            output,
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
        )?;
    }
    Ok(output)
}

fn stations_cmd(args: &[String], store: &Store) -> Result<String, Box<dyn std::error::Error>> {
    let needle = args.get(1).ok_or("stations needs a search string")?;
    let mut output = String::new();
    for (eva, name) in store.find_stations(needle)? {
        writeln!(output, "{eva}  {name}")?;
    }
    Ok(output)
}

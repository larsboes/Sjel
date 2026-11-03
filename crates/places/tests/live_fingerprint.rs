//! Read-only proof that the shared candidate fingerprint still reproduces the
//! identities the live store was built with.
//!
//! `libs/candidate-fingerprint` replaced two byte-identical copies of the hash
//! on 2026-08-28. The unit tests in that crate and in
//! `capabilities/places/src/backfill.rs` pin the algorithm against golden
//! values, but they do it with synthetic rows. This one re-derives the real
//! ones and checks them against what is actually stored, because the failure
//! this guards against is silent: a fingerprint that changed would not raise an
//! error, it would simply stop matching, and every venue link would quietly
//! become unresolvable.
//!
//! The data is private. No path, account name, merchant or amount is written
//! down here — the test reads `SJEL_PERSONAL_ROOT` for the overlay and reports
//! why it did nothing when that is unset, which is what happens in CI and on
//! any machine that does not hold the files. It opens the store with
//! `mode=ro`, reads counts, and prints them.

use places::backfill::{
    amex_fingerprint, amex_profile_from, normalize_date_dmy_slashes, normalize_text,
    parse_amount_cents, AmexProfile,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The three artifacts the proof needs, or `None` with a reason printed.
fn overlay() -> Option<(AmexProfile, PathBuf, PathBuf)> {
    let root = sjel_config::env_var_os("SJEL_PERSONAL_ROOT")?;
    let root = PathBuf::from(root);
    let config = root.join("config/finance.json");
    let raw = root.join("data/finance/import/raw");
    let database = root.join("data/axon/axon.db");
    for path in [&config, &raw, &database] {
        if !path.exists() {
            eprintln!("SJEL_PERSONAL_ROOT is set but the overlay is incomplete; skipping");
            return None;
        }
    }
    let text = std::fs::read_to_string(&config).expect("the overlay config must be readable");
    let value: serde_json::Value =
        serde_json::from_str(&text).expect("the overlay config must be JSON");
    let profile = amex_profile_from(&value).expect("the overlay must declare an Amex mapping");
    Some((profile, raw, database))
}

/// Every fingerprint the live links hang on is reproduced by the shared
/// function, from the same raw rows, byte for byte.
#[test]
fn the_shared_fingerprint_reproduces_every_live_link() {
    let Some((profile, raw, database)) = overlay() else {
        eprintln!("set SJEL_PERSONAL_ROOT to run the live fingerprint proof; skipping");
        return;
    };

    // Read-only by URI, so the proof cannot alter the store it is reading.
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let uri = format!("file:{}?mode=ro", database.display());
    let conn = rusqlite::Connection::open_with_flags(uri, flags)
        .expect("the live store must open read-only");

    let candidates: HashSet<String> = conn
        .prepare("SELECT fingerprint FROM finance_transaction_candidates WHERE source_account = ?1")
        .and_then(|mut statement| {
            statement
                .query_map([&profile.source_account], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect())
        })
        .expect("the candidate table must be readable");
    let linked: HashSet<String> = conn
        .prepare("SELECT source_id FROM places_transaction_places WHERE source = 'amex-backfill'")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect())
        })
        .expect("the link table must be readable");

    // The same walk `backfill amex` does: per file, per row, with the
    // occurrence ordinal counted inside one file.
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&raw)
        .expect("the raw export directory must be readable")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("activity") && name.ends_with(".csv"))
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the raw Amex exports must be present");

    let mut rows = 0_usize;
    let mut derived: HashSet<String> = HashSet::new();
    let mut repeats = 0_usize;
    for path in &paths {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(profile.delimiter)
            .from_path(path)
            .expect("a raw export must parse as CSV");
        let headers = reader
            .headers()
            .expect("a raw export must have headers")
            .clone();
        let at = |name: &str| {
            headers
                .iter()
                .position(|header| header.trim().trim_start_matches('\u{feff}') == name)
                .expect("the declared column must be present")
        };
        let date_at = at(&profile.date_column);
        let amount_at = at(&profile.amount_column);
        let description_at = at(&profile.description_column);

        let mut occurrences: HashMap<String, usize> = HashMap::new();
        for record in reader.records() {
            let record = record.expect("a raw row must parse");
            rows += 1;
            let get = |index: usize| record.get(index).unwrap_or_default();
            let Some(booked_at) = normalize_date_dmy_slashes(get(date_at)) else {
                continue;
            };
            let Some(mut amount_cents) =
                parse_amount_cents(get(amount_at), profile.decimal_separator)
            else {
                continue;
            };
            if profile.invert {
                amount_cents = -amount_cents;
            }
            let description = normalize_text(get(description_at));
            let key = format!("{booked_at}\u{1f}{amount_cents}\u{1f}{description}");
            let occurrence = {
                let counter = occurrences.entry(key).or_insert(0);
                let current = *counter;
                *counter += 1;
                current
            };
            if occurrence > 0 {
                repeats += 1;
            }
            derived.insert(amex_fingerprint(
                &booked_at,
                amount_cents,
                &description,
                &profile.source_account,
                occurrence,
            ));
        }
    }

    let unresolvable: Vec<&String> = linked.difference(&derived).collect();
    let unknown = derived.difference(&candidates).count();

    println!("raw export files:            {}", paths.len());
    println!("raw rows read:               {rows}");
    println!("fingerprints re-derived:     {}", derived.len());
    println!("live amex candidates:        {}", candidates.len());
    println!("live amex-backfill links:    {}", linked.len());
    println!(
        "links the shared hash finds: {}",
        linked.len() - unresolvable.len()
    );
    println!("rows using the repeat rule:  {repeats}");

    assert!(
        !linked.is_empty(),
        "this store is expected to hold amex-backfill links; an empty set proves nothing"
    );
    // The one that matters. A single miss here is a venue that stopped
    // resolving, so the count is asserted rather than sampled.
    assert!(
        unresolvable.is_empty(),
        "{} live link(s) are no longer reproduced by the shared fingerprint",
        unresolvable.len()
    );
    assert_eq!(
        unknown, 0,
        "every re-derived fingerprint must be one finance actually minted"
    );
}

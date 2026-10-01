//! One-shot backfills (README "Backfills" section). Subcommands on the places
//! binary, not routes. Each is idempotent — stable ids and ON CONFLICT DO
//! NOTHING — and prints counts instead of guessing.
//!
//! `amex` re-derives the finance candidate fingerprint from the raw export
//! rows, because that fingerprint is the journal-stable `source_id` the
//! projection carries and the one identity a location link can safely hang on
//! (README D2).
//!
//! The algorithm was a deliberate copy of `capabilities/finance/src/import.rs`
//! until 2026-08-28. It is now `libs/candidate-fingerprint`, which both call:
//! two copies of a hash that 263 live links depend on is a divergence waiting
//! to happen, and the divergence would be silent — every row would simply stop
//! matching and be reported unmatched.

use crate::geocode::{GeocodeQuery, Geocoder, StructuredQuery};
use crate::layers::{normalize_eva, parse_station_ref};
use candidate_fingerprint::CandidateKey;
use rusqlite::{params, OptionalExtension};
use sjel_store::QueryAll;

use crate::store::{stable_id, validate_prefix, Fallible, Place, PlaceVisit, PlacesStore};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Address columns of the raw American Express export. Constants here rather
/// than config because finance's import mapping deliberately has no fields for
/// them — the import pipeline discards these columns, which is why the raw
/// files are the only surviving venue source (README D1).
const AMEX_ADDRESS_COLUMN: &str = "Adresse";
const AMEX_CITY_COLUMN: &str = "Stadt";
const AMEX_POSTAL_COLUMN: &str = "PLZ";
const AMEX_COUNTRY_COLUMN: &str = "Land";

/// Where transit's suggest surface answers (`capabilities/transit/service.toml`
/// port 3000, `GET /api/suggest`). Overridable for tests and odd deployments.
pub fn transit_url() -> String {
    sjel_config::env_var("SJEL_PLACES_TRANSIT_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:3000".to_string())
}

// ---------------------------------------------------------------------------
// Fingerprint recomputation (the finance candidate identity)
// ---------------------------------------------------------------------------

/// finance's `normalize_text`: whitespace runs collapse to single spaces.
pub fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `DD/MM/YYYY` → `YYYY-MM-DD`, the one date format the Amex mapping declares
/// (`day_month_year_slashes` in the overlay's finance.json).
pub fn normalize_date_dmy_slashes(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[2] != b'/' || bytes[5] != b'/' {
        return None;
    }
    let day: u32 = value.get(..2)?.parse().ok()?;
    let month: u32 = value.get(3..5)?.parse().ok()?;
    let year: u32 = value.get(6..10)?.parse().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// finance's `parse_decimal_cents` for the subset the Amex export uses:
/// grouping separator stripped, one decimal mark, at most two fraction digits,
/// optional sign (leading or trailing).
pub fn parse_amount_cents(value: &str, decimal_separator: char) -> Option<i64> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping, "");
    if value.matches(decimal_separator).count() > 1 {
        return None;
    }
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value.as_str());
    if unsigned.is_empty()
        || !unsigned
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == decimal_separator as u8)
    {
        return None;
    }
    let (whole, fraction) = unsigned
        .split_once(decimal_separator)
        .unwrap_or((unsigned, ""));
    if fraction.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction_cents = fraction
        .bytes()
        .fold(0_i64, |cents, digit| cents * 10 + i64::from(digit - b'0'))
        * if fraction.len() == 1 { 10 } else { 1 };
    let cents = whole.checked_mul(100)?.checked_add(fraction_cents)?;
    if negative {
        cents.checked_neg()
    } else {
        Some(cents)
    }
}

/// The candidate fingerprint for one normalized Amex row. `occurrence` is the
/// zero-based repeat count of the same base tuple within one file — finance's
/// preserved-repetition rule, without which the second identical coffee in one
/// export would never match its candidate.
///
/// Parity with finance's fingerprint rests on two assumptions about the Amex
/// export, stated here as `source_reference: None` and `currency: "EUR"` even
/// though the overlay mapping declares reference_column and currency_column:
/// every measured row has an empty reference cell and an EUR currency cell. A
/// future row that breaks either assumption hashes differently and is reported
/// as unmatched — never guessed (the raw-file phase's honest-failure rule).
///
/// The hash itself is `candidate_fingerprint`, the same code finance mints
/// with, so this function is now only the Amex case of it rather than a second
/// copy of the algorithm.
pub fn amex_fingerprint(
    booked_at: &str,
    amount_cents: i64,
    description: &str,
    source_account: &str,
    occurrence: usize,
) -> String {
    CandidateKey {
        booked_at,
        amount_cents,
        currency: "EUR",
        description,
        source_reference: None,
        source_account,
    }
    .repeated_fingerprint(occurrence)
}

// ---------------------------------------------------------------------------
// Overlay profile lookup
// ---------------------------------------------------------------------------

/// The pieces of the overlay's American Express CSV mapping this backfill
/// needs. Read from the same `finance.json` finance itself loads, so the
/// fingerprint inputs (source account, sign convention, separators) cannot
/// drift from the ones the candidates were staged with — and no private
/// account name is baked into public code.
#[derive(Debug, Clone, PartialEq)]
pub struct AmexProfile {
    pub source_account: String,
    pub invert: bool,
    pub delimiter: u8,
    pub decimal_separator: char,
    pub date_column: String,
    pub amount_column: String,
    pub description_column: String,
}

pub fn amex_profile_from(config: &Value) -> Fallible<AmexProfile> {
    let profiles = config
        .get("csv_mappings")
        .and_then(Value::as_array)
        .ok_or("finance.json has no csv_mappings")?;
    let profile = profiles
        .iter()
        .find(|profile| {
            profile
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                .is_some_and(|label| label.contains("american express") || label.contains("amex"))
        })
        .ok_or("no American Express profile in the overlay's finance.json csv_mappings")?;
    let mapping = profile
        .get("mapping")
        .ok_or("Amex profile has no mapping")?;
    let text = |key: &str| mapping.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(AmexProfile {
        source_account: text("source_account").ok_or("Amex mapping has no source_account")?,
        invert: text("amount_sign").as_deref() == Some("invert"),
        delimiter: text("delimiter")
            .and_then(|d| d.bytes().next())
            .unwrap_or(b','),
        decimal_separator: text("decimal_separator")
            .and_then(|d| d.chars().next())
            .unwrap_or(','),
        date_column: text("date_column").unwrap_or_else(|| "Datum".into()),
        amount_column: text("amount_column").unwrap_or_else(|| "Betrag".into()),
        description_column: text("description_column").unwrap_or_else(|| "Beschreibung".into()),
    })
}

fn load_amex_profile() -> Fallible<AmexProfile> {
    let path = sjel_config::overlay_config("finance.json")
        .ok_or("SJEL_PERSONAL_ROOT is not set; the raw exports live in the private overlay")?;
    let body = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    amex_profile_from(&serde_json::from_str(&body)?)
}

fn raw_import_dir() -> Fallible<PathBuf> {
    let dir = sjel_config::overlay_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set; the raw exports live in the private overlay")?
        .join("data/finance/import/raw");
    if !dir.is_dir() {
        return Err(format!("raw import directory {} does not exist", dir.display()).into());
    }
    Ok(dir)
}

fn column(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|header| header.trim().trim_start_matches('\u{feff}') == name)
}

/// The address block of one raw row: street line, city line, postal code,
/// country. `Adresse` is a quoted multi-line value (street on the first line,
/// city on the last); the separate `Stadt` column wins when it is filled.
fn split_address(adresse: &str, stadt: &str) -> (Option<String>, Option<String>) {
    let lines: Vec<String> = adresse
        .lines()
        .map(normalize_text)
        .filter(|line| !line.is_empty())
        .collect();
    let street = lines.first().cloned();
    let stadt = normalize_text(stadt);
    let city = if !stadt.is_empty() {
        Some(stadt)
    } else if lines.len() > 1 {
        lines.last().cloned()
    } else {
        None
    };
    (street, city)
}

// ---------------------------------------------------------------------------
// backfill amex
// ---------------------------------------------------------------------------

/// Counters for one Amex source (the stored candidate columns, or the raw
/// export files).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AmexStats {
    /// Venue-precision links written.
    pub linked: usize,
    /// Rows the venue path could not serve — a structured miss, or no street
    /// at all — that still earned a city-precision link (D1: the city string
    /// comes from the raw data, so nothing is guessed).
    pub city_fallback: usize,
    /// Rows the provider could resolve at neither precision.
    pub not_found: usize,
    /// Rows with neither a street nor a city.
    pub no_address: usize,
    pub skipped_linked: usize,
    pub cache_hits: usize,
}

/// The address block of one Amex row, wherever it was read from: the stored
/// candidate columns (the finance contract) or the raw export.
#[derive(Debug, Clone, Default)]
pub struct RawAddress {
    pub street: Option<String>,
    pub city: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// The venue-first, city-second link path both Amex sources share. Place text
/// only reaches the provider (README D3): street, postal code, city, country —
/// never the description, amount or date. Venue precision requires a street
/// (D1): a street-less row goes straight to the city path, so city-level
/// geometry is never pinned as a venue. When the structured venue attempt
/// misses too, the city line alone (raw data, not a guess) earns a
/// city-precision link with source `amex-city-fallback`.
fn link_address(
    store: &PlacesStore,
    geocoder: &Geocoder,
    source_id: &str,
    address: RawAddress,
    today: &str,
    stats: &mut AmexStats,
) -> Fallible<()> {
    if address.street.is_none() && address.city.is_none() {
        stats.no_address += 1;
        return Ok(());
    }
    if address.street.is_some() {
        let query = GeocodeQuery::Structured(StructuredQuery {
            street: address.street,
            postalcode: address.postal_code,
            city: address.city.clone(),
            country: address.country,
        });
        let outcome = geocoder.geocode(&query, Some("venue"), today)?;
        if outcome.cached {
            stats.cache_hits += 1;
        }
        if let Some(place) = outcome.place {
            if store.link_transaction(
                source_id,
                &place.id,
                "venue",
                9000,
                "amex-backfill",
                today,
            )? {
                stats.linked += 1;
            } else {
                stats.skipped_linked += 1;
            }
            return Ok(());
        }
    }
    let Some(city) = address.city.filter(|city| !city.trim().is_empty()) else {
        stats.not_found += 1;
        return Ok(());
    };
    // No kind_override: the guard below tests the kind DERIVED from the
    // provider response, so a non-city top hit is counted as not found,
    // never force-registered and linked as a city (D1).
    let outcome = geocoder.geocode(&GeocodeQuery::Free(city), None, today)?;
    if outcome.cached {
        stats.cache_hits += 1;
    }
    match outcome.place {
        Some(place) if place.kind == "city" => {
            if store.link_transaction(
                source_id,
                &place.id,
                "city",
                6000,
                "amex-city-fallback",
                today,
            )? {
                stats.city_fallback += 1;
            } else {
                stats.skipped_linked += 1;
            }
        }
        _ => stats.not_found += 1,
    }
    Ok(())
}

/// Phase one of `backfill amex`: candidates whose raw location columns finance
/// stored at import time (`location_street` holds the verbatim Adresse value,
/// embedded newline and all — street on the first line, city on the second).
/// Returns `None` until finance's migration adds the columns; otherwise the
/// counters plus the fingerprints this phase covered, which the raw-file phase
/// skips. The prefix is a parameter so a test can point the same SQL at a
/// scratch copy of the table; production passes `finance`.
pub fn amex_from_candidates(
    store: &PlacesStore,
    geocoder: &Geocoder,
    finance_prefix: &str,
    source_account: &str,
    already_linked: &HashSet<String>,
    today: &str,
) -> Fallible<Option<(AmexStats, HashSet<String>)>> {
    validate_prefix(finance_prefix)?;
    type Located = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let located: Option<Vec<Located>> = {
        let conn = store.conn()?;
        // `information_schema.columns` has no SQLite counterpart; `pragma_table_info`
        // is the table-valued function that answers the same question, and it returns
        // no rows at all for a table that does not exist -- which is the other half of
        // what this probe has to tolerate.
        let columns_present: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'location_street'",
            params![&format!("{finance_prefix}_transaction_candidates")],
            |row| row.get(0),
        )?;
        if columns_present == 0 {
            None
        } else {
            Some(conn.query_all(
                &format!(
                    "SELECT fingerprint, location_street, location_city,
                            location_postal_code, location_country
                     FROM {finance_prefix}_transaction_candidates
                     WHERE source_account = ?1 AND location_street IS NOT NULL
                     ORDER BY fingerprint"
                ),
                params![&source_account],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?)
        }
    };
    let Some(located) = located else {
        return Ok(None);
    };

    let mut stats = AmexStats::default();
    let mut covered: HashSet<String> = HashSet::new();
    for (fingerprint, street_block, city, postal_code, country) in located {
        covered.insert(fingerprint.clone());
        if already_linked.contains(&fingerprint) {
            stats.skipped_linked += 1;
            continue;
        }
        let (street, city) = split_address(&street_block, city.as_deref().unwrap_or(""));
        let clean = |value: Option<String>| {
            value
                .as_deref()
                .map(normalize_text)
                .filter(|value| !value.is_empty())
        };
        link_address(
            store,
            geocoder,
            &fingerprint,
            RawAddress {
                street,
                city,
                postal_code: clean(postal_code),
                country: clean(country),
            },
            today,
            &mut stats,
        )?;
    }
    Ok(Some((stats, covered)))
}

pub fn amex(store: &PlacesStore, today: &str) -> Fallible<()> {
    let profile = load_amex_profile()?;
    let dir = raw_import_dir()?;
    let geocoder = Geocoder::new(store);

    let candidate_fingerprints: HashSet<String> = {
        let conn = store.conn()?;
        conn.query_all(
            "SELECT fingerprint FROM finance_transaction_candidates WHERE source_account = ?1",
            params![&profile.source_account],
            |row| row.get::<_, String>(0),
        )?
        .into_iter()
        .collect()
    };
    let already_linked = store.linked_source_ids()?;

    // Primary source: the location columns finance stores at import time.
    // The raw files below cover only the candidates imported before finance
    // kept those columns (all historical rows today).
    let candidate_phase = amex_from_candidates(
        store,
        &geocoder,
        "finance",
        &profile.source_account,
        &already_linked,
        today,
    )?;
    let (candidate_stats, covered) = match candidate_phase {
        Some((stats, covered)) => (Some(stats), covered),
        None => (None, HashSet::new()),
    };

    let mut files = 0_usize;
    let mut rows = 0_usize;
    let mut matched = 0_usize;
    let mut unmatched = 0_usize;
    let mut covered_by_candidates = 0_usize;
    let mut raw_stats = AmexStats::default();

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("activity") && name.ends_with(".csv"))
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no activity*.csv files in {}", dir.display()).into());
    }

    for path in &paths {
        files += 1;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(profile.delimiter)
            .from_path(path)?;
        let headers = reader.headers()?.clone();
        let need = |name: &str| {
            column(&headers, name)
                .ok_or_else(|| format!("{}: column {name:?} is absent", path.display()))
        };
        let date_at = need(&profile.date_column)?;
        let amount_at = need(&profile.amount_column)?;
        let description_at = need(&profile.description_column)?;
        let address_at = need(AMEX_ADDRESS_COLUMN)?;
        let city_at = need(AMEX_CITY_COLUMN)?;
        let postal_at = need(AMEX_POSTAL_COLUMN)?;
        let country_at = need(AMEX_COUNTRY_COLUMN)?;

        // Per file, like finance's per-import staging: the ordinal counts
        // repeats of the same base tuple inside one export.
        let mut occurrences: HashMap<String, usize> = HashMap::new();

        for record in reader.records() {
            let record = record?;
            rows += 1;
            let get = |at: usize| record.get(at).unwrap_or_default();
            let Some(booked_at) = normalize_date_dmy_slashes(get(date_at)) else {
                unmatched += 1;
                continue;
            };
            let Some(mut amount_cents) =
                parse_amount_cents(get(amount_at), profile.decimal_separator)
            else {
                unmatched += 1;
                continue;
            };
            if profile.invert {
                let Some(inverted) = amount_cents.checked_neg() else {
                    unmatched += 1;
                    continue;
                };
                amount_cents = inverted;
            }
            let description = normalize_text(get(description_at));
            let base_key = format!("{booked_at}\u{1f}{amount_cents}\u{1f}{description}");
            let occurrence = {
                let counter = occurrences.entry(base_key).or_insert(0);
                let current = *counter;
                *counter += 1;
                current
            };
            let source_id = amex_fingerprint(
                &booked_at,
                amount_cents,
                &description,
                &profile.source_account,
                occurrence,
            );
            if !candidate_fingerprints.contains(&source_id) {
                unmatched += 1;
                continue;
            }
            matched += 1;
            if covered.contains(&source_id) {
                covered_by_candidates += 1;
                continue;
            }
            if already_linked.contains(&source_id) {
                raw_stats.skipped_linked += 1;
                continue;
            }
            let (street, city) = split_address(get(address_at), get(city_at));
            let postalcode = normalize_text(get(postal_at));
            let country = normalize_text(get(country_at));
            link_address(
                store,
                &geocoder,
                &source_id,
                RawAddress {
                    street,
                    city,
                    postal_code: (!postalcode.is_empty()).then_some(postalcode),
                    country: (!country.is_empty()).then_some(country),
                },
                today,
                &mut raw_stats,
            )?;
        }
    }

    // The source split the report promises: candidates-with-stored-location
    // first, raw files second.
    match &candidate_stats {
        Some(stats) => println!(
            "backfill amex (candidates): {} with stored location, {} venue links written, \
             {} city-fallback links, {} not geocodable, {} without address, \
             {} already linked, {} geocode cache hits",
            covered.len(),
            stats.linked,
            stats.city_fallback,
            stats.not_found,
            stats.no_address,
            stats.skipped_linked,
            stats.cache_hits,
        ),
        None => println!(
            "backfill amex (candidates): finance has no location columns yet; raw files only"
        ),
    }
    println!(
        "backfill amex (raw files): {files} files, {rows} rows, {matched} matched candidates, \
         {covered_by_candidates} covered by stored locations, \
         {unmatched} unmatched (reported, never guessed), {} without address, \
         {} not geocodable, {} venue links written, {} city-fallback links, \
         {} already linked, {} geocode cache hits",
        raw_stats.no_address,
        raw_stats.not_found,
        raw_stats.linked,
        raw_stats.city_fallback,
        raw_stats.skipped_linked,
        raw_stats.cache_hits,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill cities
// ---------------------------------------------------------------------------

/// A usable city token: letters (plus space, dot, hyphen) and at least three
/// characters. Filters the numeric junk that ends PayPal descriptions and the
/// pseudo-city lines some Amex rows carry.
fn plausible_city(token: &str) -> bool {
    token.chars().count() >= 3
        && token
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '.' | '-'))
}

/// The longest known city that ends the description, compared in uppercase —
/// `"MARKT 12345678 MUSTERSTADT"` matches `MUSTERSTADT`, `"CAFE BAD MUSTERSTADT"`
/// matches `BAD MUSTERSTADT`. No match means skip: conservative by contract
/// (README D1).
pub fn trailing_city<'a>(description: &str, cities: &'a HashSet<String>) -> Option<&'a str> {
    let upper = normalize_text(description).to_uppercase();
    cities
        .iter()
        .filter(|city| upper == **city || upper.ends_with(&format!(" {city}")))
        .max_by_key(|city| city.len())
        .map(String::as_str)
}

fn known_cities(store: &PlacesStore) -> Fallible<HashSet<String>> {
    let prefix = store.prefix();
    let conn = store.conn()?;
    let mut cities: HashSet<String> = conn
        .query_all(
            &format!(
                "SELECT DISTINCT city FROM {prefix}_places WHERE city IS NOT NULL
                 UNION
                 SELECT name FROM {prefix}_places WHERE kind = 'city'"
            ),
            [],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|city| normalize_text(city).to_uppercase())
        .filter(|city| plausible_city(city))
        .collect();

    // Plus the city lines of the raw Amex exports, when the overlay is here.
    if let Ok(profile) = load_amex_profile() {
        if let Ok(dir) = raw_import_dir() {
            for entry in std::fs::read_dir(&dir)?.filter_map(|entry| entry.ok()) {
                let path = entry.path();
                let is_activity = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("activity") && name.ends_with(".csv"));
                if !is_activity {
                    continue;
                }
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(profile.delimiter)
                    .from_path(&path)?;
                let headers = reader.headers()?.clone();
                let (Some(address_at), Some(city_at)) = (
                    column(&headers, AMEX_ADDRESS_COLUMN),
                    column(&headers, AMEX_CITY_COLUMN),
                ) else {
                    continue;
                };
                for record in reader.records().filter_map(|record| record.ok()) {
                    let (_, city) = split_address(
                        record.get(address_at).unwrap_or_default(),
                        record.get(city_at).unwrap_or_default(),
                    );
                    if let Some(city) = city {
                        let upper = city.to_uppercase();
                        if plausible_city(&upper) {
                            cities.insert(upper);
                        }
                    }
                }
            }
        }
    }
    Ok(cities)
}

pub fn cities(store: &PlacesStore, today: &str) -> Fallible<()> {
    let cities = known_cities(store)?;
    let geocoder = Geocoder::new(store);
    let prefix = store.prefix();

    let unlinked: Vec<(String, String)> = {
        let conn = store.conn()?;
        conn.query_all(
            &format!(
                "SELECT p.source_id, p.description
                 FROM finance_transaction_projection p
                 WHERE p.kind = 'expense' AND p.currency = 'EUR'
                   AND p.source_id IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM {prefix}_transaction_places tp
                       WHERE tp.source_id = p.source_id
                   )
                 ORDER BY p.booked_at, p.source_id"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };

    let mut skipped = 0_usize;
    let mut linked = 0_usize;
    let mut not_found = 0_usize;
    let mut wrong_kind = 0_usize;
    for (source_id, description) in &unlinked {
        let Some(city) = trailing_city(description, &cities) else {
            skipped += 1;
            continue;
        };
        // No kind_override: a `city` override would stamp any first-seen OSM
        // object kind='city' and make the guard below vacuous. Deriving the
        // kind from the response is what lets `wrong_kind` actually fire (D1).
        let outcome = geocoder.geocode(&GeocodeQuery::Free(city.to_string()), None, today)?;
        match outcome.place {
            Some(place) if place.kind == "city" => {
                if store.link_transaction(
                    source_id,
                    &place.id,
                    "city",
                    6000,
                    "city-backfill",
                    today,
                )? {
                    linked += 1;
                }
            }
            Some(_) => wrong_kind += 1,
            None => not_found += 1,
        }
    }

    println!(
        "backfill cities: {} unlinked transactions, {} known city names, \
         {linked} city links written, {skipped} without a recognisable city token (skipped), \
         {not_found} city names the geocoder could not resolve, {wrong_kind} resolved to a non-city",
        unlinked.len(),
        cities.len(),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill stations
// ---------------------------------------------------------------------------

pub fn stations(store: &PlacesStore, today: &str) -> Fallible<()> {
    // Every station reference transit persists, from both tables. Legs carry
    // full HAFAS ids whose X=/Y= already embed the coordinate; trips carry
    // bare EVA codes.
    let refs: Vec<String> = {
        let conn = store.conn()?;
        conn.query_all(
            "SELECT origin_eva FROM transit_trips
             UNION SELECT destination_eva FROM transit_trips
             UNION SELECT origin_eva FROM transit_trip_legs
             UNION SELECT destination_eva FROM transit_trip_legs",
            [],
            |row| row.get::<_, String>(0),
        )?
    };

    // Group by EVA, preferring a reference that carries its own coordinates.
    let mut by_eva: BTreeMap<String, crate::layers::StationRef> = BTreeMap::new();
    for raw in &refs {
        let Some(parsed) = parse_station_ref(raw) else {
            continue;
        };
        let entry = by_eva
            .entry(parsed.eva.clone())
            .or_insert_with(|| parsed.clone());
        if entry.latitude.is_none() && parsed.latitude.is_some() {
            *entry = parsed;
        }
    }

    let mut from_refs = 0_usize;
    let mut via_suggest = 0_usize;
    let mut existing = 0_usize;
    let mut unresolved: Vec<String> = Vec::new();

    let client = sjel_http::client(
        sjel_http::Purpose::new("places-backfill"),
        std::time::Duration::from_secs(15),
    )?;
    let suggest_base = transit_url();

    for (eva, station) in &by_eva {
        let external_ref = format!("eva:{eva}");
        if store.place_by_external_ref(&external_ref)?.is_some() {
            existing += 1;
            continue;
        }
        let (name, latitude, longitude) =
            if let (Some(lat), Some(lon)) = (station.latitude, station.longitude) {
                let name = station.name.clone().unwrap_or_else(|| format!("EVA {eva}"));
                from_refs += 1;
                (name, lat, lon)
            } else {
                // Bare EVA: name it from punctuality's station list (zero-padded
                // there) when listed, then ask transit's own suggest surface for
                // coordinates — by that name, or by the bare EVA itself. The EVA
                // query is what resolves the HAFAS meta-stations (city-level
                // aggregate codes) that punctuality's station list lacks (ISA
                // PLC-6; the specific codes are live data and stay in the
                // overlay evidence note).
                let known_name: Option<String> = {
                    let conn = store.conn()?;
                    // `lpad($1, 8, '0')` has no SQLite counterpart. The padded
                    // spelling is computed here instead, by the same helper the
                    // rest of this module normalizes EVAs with, so the two forms
                    // cannot drift apart in two places.
                    conn.query_row(
                        "SELECT station_name FROM punctuality_stations
                         WHERE eva = ?1 OR eva = ?2",
                        params![&eva, &normalize_eva(eva)],
                        |row| row.get(0),
                    )
                    .optional()?
                };
                let query = known_name.clone().unwrap_or_else(|| eva.clone());
                let url = format!("{suggest_base}/api/suggest");
                let request = sjel_server::InboundAuth::with_loopback_auth(
                    client.get(&url).query(&[("q", query.as_str())]),
                    &url,
                );
                let resolved = request
                    .send()
                    .ok()
                    .filter(|response| response.status().is_success())
                    .and_then(|response| response.json::<Value>().ok())
                    .and_then(|body| {
                        body.as_array()?.iter().find_map(|station| {
                            let id = station.get("id").and_then(Value::as_str)?;
                            if normalize_eva(id) != *eva {
                                return None;
                            }
                            Some((
                                station
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                station.get("latitude")?.as_f64()?,
                                station.get("longitude")?.as_f64()?,
                            ))
                        })
                    });
                let Some((suggest_name, lat, lon)) = resolved else {
                    unresolved.push(eva.clone());
                    continue;
                };
                let name = known_name
                    .or(suggest_name)
                    .unwrap_or_else(|| format!("EVA {eva}"));
                via_suggest += 1;
                (name, lat, lon)
            };
        store.upsert_place(
            &Place {
                id: stable_id("place", &external_ref),
                name,
                kind: "station".into(),
                address: None,
                city: None,
                country_code: None,
                latitude: Some(latitude),
                longitude: Some(longitude),
                source: "transit-backfill".into(),
                external_ref: Some(external_ref.clone()),
            },
            today,
        )?;
    }

    println!(
        "backfill stations: {} distinct EVA codes, {from_refs} from embedded HAFAS coordinates, \
         {via_suggest} via transit suggest ({suggest_base}), {existing} already registered, \
         {} unresolved{}",
        by_eva.len(),
        unresolved.len(),
        if unresolved.is_empty() {
            String::new()
        } else {
            format!(" ({})", unresolved.join(", "))
        }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill travelers (PRD 8.2: register proposals from trips' travelers)
// ---------------------------------------------------------------------------

/// Counts only. Person names and plan titles stay in the database — never in
/// logs or printed output (README D4: the register is C2).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TravelersReport {
    /// Non-archived plans read.
    pub plans: usize,
    /// Plans with travelers but no destination that carries a coordinate —
    /// reported, never guessed.
    pub without_located_destination: usize,
    /// Traveler-name × plan pairs seen.
    pub pairs: usize,
    pub proposals_written: usize,
    pub proposals_existing: usize,
    pub places_created: usize,
}

/// The traveler names of one plan, as trips serializes them: a JSON array of
/// strings (`trips.plans.travelers`).
pub fn traveler_names(travelers: &str) -> Vec<String> {
    serde_json::from_str::<Value>(travelers)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The first destination `PlaceRef` that carries coordinates
/// (`capabilities/trips/src/store.rs` serializes `id, name, kind, latitude,
/// longitude`). Returns `(name, registry kind, latitude, longitude)`; `None`
/// when no destination can be drawn.
pub fn first_located_destination(destinations: &str) -> Option<(String, String, f64, f64)> {
    let refs = serde_json::from_str::<Value>(destinations).ok()?;
    refs.as_array()?.iter().find_map(|place_ref| {
        let latitude = place_ref.get("latitude").and_then(Value::as_f64)?;
        let longitude = place_ref.get("longitude").and_then(Value::as_f64)?;
        let name = normalize_text(place_ref.get("name").and_then(Value::as_str)?);
        if name.is_empty() {
            return None;
        }
        // trips' PlaceKind, mapped onto the registry's kinds; `airport` lands
        // as station, and trips' own default is station.
        let kind = match place_ref.get("kind").and_then(Value::as_str) {
            Some("city") => "city",
            Some("venue") => "venue",
            Some("address") => "address",
            _ => "station",
        };
        Some((name, kind.to_string(), latitude, longitude))
    })
}

/// Derive register proposals from `trips.plans` travelers (PRD 8.2). One
/// proposal per traveler × plan, id stable on that pair, state hardcoded to
/// `proposed` by `propose_person_place` — derivation never writes a confirmed
/// row (ISA PLC-7). Archived plans are skipped, the same reading of `status`
/// as `travel_layer`. The prefix is a parameter so a test can point the
/// same SQL at a scratch copy of the table; production passes `trips`.
pub fn travelers_from(
    store: &PlacesStore,
    trips_prefix: &str,
    today: &str,
) -> Fallible<TravelersReport> {
    validate_prefix(trips_prefix)?;
    let plans: Vec<(String, String, String, String, String)> = {
        let conn = store.conn()?;
        conn.query_all(
            &format!(
                "SELECT id, travelers, destinations, date_start, date_end
                 FROM {trips_prefix}_plans
                 WHERE status != 'archived'
                 ORDER BY date_start, id"
            ),
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?
    };

    let mut report = TravelersReport::default();
    for (plan_id, travelers, destinations, date_start, date_end) in plans {
        report.plans += 1;
        let names = traveler_names(&travelers);
        if names.is_empty() {
            continue;
        }
        let Some((name, kind, latitude, longitude)) = first_located_destination(&destinations)
        else {
            report.without_located_destination += 1;
            continue;
        };
        // One registry row per destination name and kind, whatever plan named
        // it — the existing upsert-by-stable-identity path.
        let external_ref = format!("trips-dest:{kind}:{}", name.to_lowercase());
        let place = Place {
            id: stable_id("place", &external_ref),
            name,
            kind,
            address: None,
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "trips-travelers".into(),
            external_ref: Some(external_ref),
        };
        if store.upsert_place(&place, today)? {
            report.places_created += 1;
        }
        for person in &names {
            report.pairs += 1;
            let id = stable_id("pp", &format!("trips-travelers:{person}:{plan_id}"));
            if store.propose_person_place(
                &id,
                person,
                &place.id,
                Some(&date_start),
                Some(&date_end),
                5000,
                "trips-travelers",
                today,
            )? {
                report.proposals_written += 1;
            } else {
                report.proposals_existing += 1;
            }
        }
    }
    Ok(report)
}

pub fn travelers(store: &PlacesStore, today: &str) -> Fallible<()> {
    let report = travelers_from(store, "trips", today)?;
    println!(
        "backfill travelers: {} plans, {} traveler-plan pairs, {} proposals written, \
         {} already present, {} destination places created, \
         {} plans without a located destination (skipped)",
        report.plans,
        report.pairs,
        report.proposals_written,
        report.proposals_existing,
        report.places_created,
        report.without_located_destination,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill vault
// ---------------------------------------------------------------------------

/// Minimal frontmatter reader for the two shapes the vault uses. Returns the
/// scalar map and the list map of the block between the first `---` pair.
pub fn parse_frontmatter(body: &str) -> (BTreeMap<String, String>, BTreeMap<String, Vec<String>>) {
    let mut scalars = BTreeMap::new();
    let mut lists: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (scalars, lists);
    }
    let mut current_list: Option<String> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(item) = line.strip_prefix("  - ") {
            if let Some(key) = &current_list {
                lists
                    .entry(key.clone())
                    .or_default()
                    .push(item.trim().trim_matches('"').to_string());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').to_string();
            if value.is_empty() {
                current_list = Some(key);
            } else {
                current_list = None;
                scalars.insert(key, value);
            }
        }
    }
    (scalars, lists)
}

/// Coordinates from frontmatter, in both spellings the vault holds today:
/// a two-item list of quoted strings (`- "40.75"` / `- "-74.00"`, latitude
/// first) or an inline `lat,lon` scalar. `[]` and absence are `None`.
pub fn frontmatter_coordinates(
    scalars: &BTreeMap<String, String>,
    lists: &BTreeMap<String, Vec<String>>,
) -> Option<(f64, f64)> {
    if let Some(items) = lists.get("coordinates") {
        if items.len() == 2 {
            return Some((items[0].parse().ok()?, items[1].parse().ok()?));
        }
    }
    let inline = scalars.get("coordinates")?;
    let inline = inline.trim();
    if inline.is_empty() || inline == "[]" {
        return None;
    }
    let (lat, lon) = inline.split_once(',')?;
    Some((lat.trim().parse().ok()?, lon.trim().parse().ok()?))
}

/// The `home` frontmatter key: the city a person lives in, as plain text (`home: Bonn`).
///
/// Added 2026-09-25 because a city is what a person note's author knows, and a coordinate
/// pair is not: 5 of 89 notes carried `coordinates` that day. `coordinates` still wins when
/// both are set, because it is the more precise fact. An empty value, `[]` or a quoted
/// empty string is no home.
pub fn frontmatter_home(scalars: &BTreeMap<String, String>) -> Option<String> {
    let raw = scalars
        .get("home")?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'');
    let home = raw.trim();
    (!home.is_empty() && home != "[]").then(|| home.to_string())
}

/// Great-circle distance in kilometres — haversine in code, the no-PostGIS
/// decision (README "Deliberately not built", following
/// `dashboard/src/lib/travel/travel-candidates.ts`).
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let radius_km = 6371.0;
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * radius_km * h.sqrt().asin()
}

fn note_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// A registry row for a coordinate-only proposal: ask the provider what the
/// locality there is called (REVERSE geocode — the query carries the two
/// numbers only, never the person's name or note text, README D3); when the
/// provider knows nothing there, keep a bare coordinate row rather than guess
/// (D1). Returns the place id and whether reverse geocoding named it.
pub fn resolve_unnamed_place(
    store: &PlacesStore,
    geocoder: &Geocoder,
    coordinates: (f64, f64),
    today: &str,
) -> Fallible<(String, bool)> {
    let outcome = geocoder.reverse(coordinates.0, coordinates.1, today)?;
    if let Some(place) = outcome.place {
        return Ok((place.id, true));
    }
    let label = format!("{:.4},{:.4}", coordinates.0, coordinates.1);
    let place = Place {
        id: stable_id("place", &format!("coord:{label}")),
        name: label.clone(),
        kind: "address".into(),
        address: None,
        city: None,
        country_code: None,
        latitude: Some(coordinates.0),
        longitude: Some(coordinates.1),
        source: "vault-frontmatter".into(),
        external_ref: Some(format!("coord:{label}")),
    };
    store.upsert_place(&place, today)?;
    Ok((place.id, false))
}

fn people_dir() -> Fallible<PathBuf> {
    if let Ok(dir) = sjel_config::env_var("SJEL_PLACES_PEOPLE_DIR") {
        return Ok(sjel_config::expand_tilde(&dir));
    }
    // The vault root the finance capability already declares in the overlay
    // (finance.json `obsidian.root`) — one declaration of where the vault is.
    let path =
        sjel_config::overlay_config("finance.json").ok_or("SJEL_PERSONAL_ROOT is not set")?;
    let body = std::fs::read_to_string(&path)?;
    let config: Value = serde_json::from_str(&body)?;
    let root = config
        .get("obsidian")
        .and_then(|obsidian| obsidian.get("root"))
        .and_then(Value::as_str)
        .ok_or("finance.json declares no obsidian.root; set SJEL_PLACES_PEOPLE_DIR instead")?;
    Ok(sjel_config::expand_tilde(root).join("Atlas/People"))
}

pub fn vault(store: &PlacesStore, today: &str) -> Fallible<()> {
    // Exported place notes in the overlay.
    let places_dir = sjel_config::overlay_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set")?
        .join("data/places/vault-notes");
    let mut place_notes = 0_usize;
    let mut imported = 0_usize;
    let mut place_existing = 0_usize;
    let mut skipped = 0_usize;
    if places_dir.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&places_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        paths.sort();
        for path in paths {
            place_notes += 1;
            let Some(name) = note_stem(&path) else {
                skipped += 1;
                continue;
            };
            let body = std::fs::read_to_string(&path)?;
            let (scalars, lists) = parse_frontmatter(&body);
            let Some((latitude, longitude)) = frontmatter_coordinates(&scalars, &lists) else {
                skipped += 1;
                continue;
            };
            let city = scalars.get("city").cloned().filter(|city| !city.is_empty());
            // A note that names a containing city is a venue inside it; a note
            // without one (Vienna) is the city itself.
            let kind = if city.is_some() { "venue" } else { "city" };
            let place = Place {
                id: stable_id("place", &format!("vault:{name}")),
                name: name.clone(),
                kind: kind.into(),
                address: None,
                city: city.or_else(|| (kind == "city").then(|| name.clone())),
                country_code: None,
                latitude: Some(latitude),
                longitude: Some(longitude),
                source: "vault".into(),
                external_ref: Some(format!("vault:{name}")),
            };
            if store.upsert_place(&place, today)? {
                imported += 1;
            } else {
                place_existing += 1;
            }
        }
    }

    // Person notes with populated coordinates become register PROPOSALS,
    // never confirmed rows (README D4, ISA PLC-7).
    let people = people_dir()?;
    let geocoder = Geocoder::new(store);
    let mut person_notes = 0_usize;
    let mut proposals = 0_usize;
    let mut proposals_existing = 0_usize;
    let mut reverse_named = 0_usize;
    let mut home_notes = 0_usize;
    if people.is_dir() {
        let known = store.places_with_coordinates()?;
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&people)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(person) = note_stem(&path) else {
                continue;
            };
            let body = std::fs::read_to_string(&path)?;
            let (scalars, lists) = parse_frontmatter(&body);
            let Some(coordinates) = frontmatter_coordinates(&scalars, &lists) else {
                // No coordinate: a `home` city is the second way in. A registered city
                // of that name is reused; otherwise the city text is geocoded. Only the
                // city text reaches the provider, never the person's name (README D3).
                let Some(home) = frontmatter_home(&scalars) else {
                    continue;
                };
                let wanted = home.to_lowercase();
                let registered = known
                    .iter()
                    .find(|place| place.kind == "city" && place.name.to_lowercase() == wanted)
                    .map(|place| place.id.clone());
                let place_id = match registered {
                    Some(id) => id,
                    None => {
                        let outcome = geocoder.geocode(
                            &GeocodeQuery::Free(home.clone()),
                            Some("city"),
                            today,
                        )?;
                        match outcome.place {
                            Some(place) => place.id,
                            None => {
                                eprintln!(
                                    "backfill vault: home {home:?} of one person note did not geocode; skipped"
                                );
                                continue;
                            }
                        }
                    }
                };
                home_notes += 1;
                let id = stable_id("pp", &format!("vault-home:{person}:{wanted}"));
                if store.propose_person_place(
                    &id,
                    &person,
                    &place_id,
                    None,
                    None,
                    5000,
                    "vault-home",
                    today,
                )? {
                    proposals += 1;
                } else {
                    proposals_existing += 1;
                }
                continue;
            };
            person_notes += 1;
            // Reuse the nearest registered place within 30 km; otherwise name
            // the coordinate by reverse geocode. The only thing that reaches
            // the provider is the coordinate pair — never the person's name or
            // any note text (README D3).
            let nearest = known
                .iter()
                .filter_map(|place| {
                    let position = (place.latitude?, place.longitude?);
                    let distance = haversine_km(coordinates, position);
                    (distance <= 30.0).then_some((distance, place))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, place)| place.id.clone());
            let place_id = match nearest {
                Some(id) => id,
                None => {
                    let (id, named) = resolve_unnamed_place(store, &geocoder, coordinates, today)?;
                    if named {
                        reverse_named += 1;
                    }
                    id
                }
            };
            // Id derived from the note's own content (person + coordinate),
            // never from the resolved place id: which registered place is
            // nearest shifts as other backfills grow the registry, and an id
            // that shifted with it would write a second proposal for the same
            // unchanged note on the next run — breaking the README's "each is
            // idempotent" promise. An edited coordinate is a changed fact and
            // correctly earns a fresh proposal.
            let id = stable_id(
                "pp",
                &format!(
                    "vault-frontmatter:{person}:{:.4},{:.4}",
                    coordinates.0, coordinates.1
                ),
            );
            if store.propose_person_place(
                &id,
                &person,
                &place_id,
                None,
                None,
                5000,
                "vault-frontmatter",
                today,
            )? {
                proposals += 1;
            } else {
                proposals_existing += 1;
            }
        }
    }

    println!(
        "backfill vault: {place_notes} place notes, {imported} places imported, \
         {place_existing} already present, {skipped} without coordinates (skipped), \
         {person_notes} person notes with coordinates, {home_notes} with a home city only, \
         {proposals} proposals written, \
         {proposals_existing} proposals already present, \
         {reverse_named} coordinate-only places named by reverse geocode"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// backfill takeout
// ---------------------------------------------------------------------------

/// Google Takeout exports staged in the overlay, one directory per export:
/// `data/places/import/raw/takeout-<yyyymmdd>/` — the same staging convention
/// finance uses for its raw card exports. Two files are read: `Labelled
/// places.json` and `Reviews.json`, both GeoJSON FeatureCollections.
/// `Commute routes.json` is deliberately NOT read: raw route traces sit
/// against this README's "No GPS trace" ruling, so the file is counted and
/// reported as held rather than silently skipped.
#[derive(Debug, Default)]
pub struct TakeoutReport {
    pub dirs: usize,
    pub labelled: usize,
    pub reviews: usize,
    pub places_created: usize,
    pub places_existing: usize,
    pub proposals_written: usize,
    pub proposals_existing: usize,
    pub commute_files_held: usize,
    pub features_skipped: usize,
    /// Visits written from a review, and reviews already recorded.
    ///
    /// Counted separately from places because the two are different claims: a
    /// place exists, a visit says the operator was there. A re-import adds no
    /// second visit, so `visits_existing` is the expected outcome on a re-run.
    pub visits_written: usize,
    pub visits_existing: usize,
    /// Timeline files read, and place visits found inside them.
    pub timeline_files: usize,
    pub timeline_visits: usize,
    /// Files present in the export that this importer does not read, by name.
    ///
    /// Named rather than counted: the whole complaint that produced this field is
    /// that a Takeout can hold more than the importer reads and nothing said so.
    pub files_held: Vec<String>,
}

/// The person a Maps label names, or `None` for the user's own anchors. Google
/// fixes the literal labels `Home` and `Work`; every other label is one the
/// user typed, and in this corpus those name other people's addresses
/// ("Marinas Home", "Bryan"). A trailing ` Home`/` Work` qualifier is
/// dropped; the remainder stays VERBATIM — no possessive stripping, because
/// "Lars Home" must not become "Lar". A wrong person string costs a dismissed
/// proposal at review (PRD §8.2); a silently mangled name would cost a wrong
/// register row.
pub fn takeout_person_label(label: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.eq_ignore_ascii_case("home") || trimmed.eq_ignore_ascii_case("work") {
        return None;
    }
    let person = trimmed
        .strip_suffix(" Home")
        .or_else(|| trimmed.strip_suffix(" Work"))
        .unwrap_or(trimmed)
        .trim();
    (!person.is_empty()).then(|| person.to_string())
}

/// `[lon, lat]` per the GeoJSON spec — the reverse of `Place`'s field order.
fn geojson_point(feature: &Value) -> Option<(f64, f64)> {
    let coordinates = feature.get("geometry")?.get("coordinates")?.as_array()?;
    let longitude = coordinates.first()?.as_f64()?;
    let latitude = coordinates.get(1)?.as_f64()?;
    Some((latitude, longitude))
}

fn geojson_features(body: &str) -> Fallible<Vec<Value>> {
    let value: Value = serde_json::from_str(body)?;
    Ok(value
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn takeout_labelled(
    store: &PlacesStore,
    body: &str,
    today: &str,
    report: &mut TakeoutReport,
) -> Fallible<()> {
    for feature in geojson_features(body)? {
        let properties = feature.get("properties").cloned().unwrap_or(Value::Null);
        let Some(name) = properties.get("name").and_then(Value::as_str) else {
            report.features_skipped += 1;
            continue;
        };
        let Some((latitude, longitude)) = geojson_point(&feature) else {
            report.features_skipped += 1;
            continue;
        };
        report.labelled += 1;
        let external_ref = format!("takeout:labelled:{name}");
        let place = Place {
            id: stable_id("place", &external_ref),
            name: name.to_string(),
            kind: "address".into(),
            address: properties
                .get("address")
                .and_then(Value::as_str)
                .map(str::to_string),
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "takeout".into(),
            external_ref: Some(external_ref.clone()),
        };
        if store.upsert_place(&place, today)? {
            report.places_created += 1;
        } else {
            report.places_existing += 1;
        }
        if let Some(person) = takeout_person_label(name) {
            // 9000 bp: the user typed this label into Maps themselves — it is
            // curation, not derivation — but confirmation stays with the human
            // (PLC-7), so the row still enters as `proposed`.
            let id = stable_id("pp", &format!("takeout-labelled:{person}:{name}"));
            if store
                .propose_person_place(&id, &person, &place.id, None, None, 9000, "takeout", today)?
            {
                report.proposals_written += 1;
            } else {
                report.proposals_existing += 1;
            }
        }
    }
    Ok(())
}

fn takeout_reviews(
    store: &PlacesStore,
    body: &str,
    today: &str,
    report: &mut TakeoutReport,
) -> Fallible<()> {
    for feature in geojson_features(body)? {
        let properties = feature.get("properties").cloned().unwrap_or(Value::Null);
        let location = properties.get("location").cloned().unwrap_or(Value::Null);
        let Some(name) = location.get("name").and_then(Value::as_str) else {
            report.features_skipped += 1;
            continue;
        };
        let Some((latitude, longitude)) = geojson_point(&feature) else {
            report.features_skipped += 1;
            continue;
        };
        report.reviews += 1;
        // The maps URL carries Google's stable place id and is the one
        // identity two exports of the same review agree on. The review text
        // and rating stay in the export — they are not place attributes.
        let external_ref = properties
            .get("google_maps_url")
            .and_then(Value::as_str)
            .map(|url| format!("takeout:review:{url}"))
            .unwrap_or_else(|| format!("takeout:review:{name}:{latitude:.4},{longitude:.4}"));
        let place = Place {
            id: stable_id("place", &external_ref),
            name: name.to_string(),
            kind: "venue".into(),
            address: location
                .get("address")
                .and_then(Value::as_str)
                .map(str::to_string),
            city: None,
            country_code: location
                .get("country_code")
                .and_then(Value::as_str)
                .map(str::to_string),
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "takeout".into(),
            // Cloned because the visit below is keyed by the same identity: the
            // Google Maps URL is what makes a re-import a no-op instead of a second
            // visit.
            external_ref: Some(external_ref.clone()),
        };
        if store.upsert_place(&place, today)? {
            report.places_created += 1;
        } else {
            report.places_existing += 1;
        }

        // The date and the rating are the OPERATOR's record, not the place's, and
        // they were dropped here until 2026-09-23 on the reasoning that they are
        // not place attributes -- which is correct, and is why they go in their own
        // table. Dropping them threw away the only personal signal a review export
        // carries: that a place was been to, when, and what it was worth.
        let visit = PlaceVisit {
            id: stable_id("visit", &external_ref),
            place_id: place.id.clone(),
            visited_on: properties
                .get("date")
                .and_then(Value::as_str)
                .map(str::to_string),
            rating: properties
                .get("five_star_rating_published")
                .and_then(Value::as_i64),
            source: "takeout-review".into(),
            external_ref: Some(external_ref.clone()),
        };
        if store.put_visit(&visit, today)? {
            report.visits_written += 1;
        } else {
            report.visits_existing += 1;
        }
    }
    Ok(())
}

/// Find a named file anywhere within `depth` levels of the export root.
///
/// Google has moved these files between exports: `Reviews.json` used to sit at the
/// export root and now sits under `Maps (your places)/`. Reading only the root
/// meant an export whose layout had changed imported nothing and reported success,
/// which is the failure this function exists to end. Bounded rather than
/// unbounded so a huge Timeline tree cannot turn one import into a full walk.
fn find_export_file(root: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    children.sort();
    children
        .into_iter()
        .find_map(|child| find_export_file(&child, name, depth - 1))
}

/// Every `<year>_<MONTH>.json` under any `Semantic Location History` directory.
///
/// The documented layout is `Semantic Location History/<year>/<year>_<MONTH>.json`
/// (locationhistoryformat.com, read 2026-09-23). Found by walking rather than by
/// constructing the path, because the year folders are whatever Google wrote.
fn timeline_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, under_semantic: bool, found: &mut Vec<PathBuf>, depth: usize) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                let semantic = under_semantic
                    || path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("Semantic Location History"));
                walk(&path, semantic, found, depth - 1);
            } else if under_semantic {
                let is_month = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".json") && name.contains('_'));
                if is_month {
                    found.push(path);
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(root, false, &mut found, 6);
    found.sort();
    found
}

/// One `Semantic Location History/<year>/<year>_<MONTH>.json`.
///
/// **Built to the documented schema, not to a live file.** No Timeline export has
/// been staged on this machine, so every field name here comes from
/// locationhistoryformat.com's reference, read 2026-09-23, and none of it has been
/// seen in the wild. The first real export is what proves it -- and the test below
/// pins the shape so a divergence fails loudly rather than importing nothing.
///
/// The shape, exactly as documented:
///
/// ```text
/// { "timelineObjects": [
///     { "placeVisit": {
///         "location": { "latitudeE7": i64, "longitudeE7": i64,
///                       "name": str?, "address": str?, "placeId": str? },
///         "duration": { "startTimestamp": iso, "endTimestamp": iso } } },
///     { "activitySegment": { ... } } ] }
/// ```
///
/// Timestamps are ISO strings, NOT the `startTimestampMs` integers that a summary
/// of this format will tell you. `activitySegment` is skipped: a journey between
/// two places is not a place, and the two Berlin trips are already recorded as
/// plans.
fn takeout_timeline(
    store: &PlacesStore,
    body: &str,
    today: &str,
    report: &mut TakeoutReport,
) -> Fallible<()> {
    let value: Value = serde_json::from_str(body)?;
    let Some(objects) = value.get("timelineObjects").and_then(Value::as_array) else {
        // A month file that parses but carries no `timelineObjects` is a layout
        // this does not know, not an empty month. Counted, never silently zero.
        report.timeline_files += 1;
        return Ok(());
    };
    report.timeline_files += 1;

    for object in objects {
        let Some(visit) = object.get("placeVisit") else {
            continue; // activitySegment, or a key this version does not know
        };
        let location = visit.get("location").cloned().unwrap_or(Value::Null);
        // E7 is degrees times 10^7, rounded. Absent coordinates make a visit
        // unplaceable, and an unplaceable visit is not a row.
        let Some(latitude) = location.get("latitudeE7").and_then(Value::as_i64) else {
            continue;
        };
        let Some(longitude) = location.get("longitudeE7").and_then(Value::as_i64) else {
            continue;
        };
        let latitude = latitude as f64 / 10_000_000.0;
        let longitude = longitude as f64 / 10_000_000.0;

        // A Timeline visit is often a coordinate with no name -- Google only names
        // places it recognises. The address's first line is the next best label,
        // and a bare coordinate is the last: a named row is worth more, and an
        // unnamed one is still the fact that they were there.
        let address = location
            .get("address")
            .and_then(Value::as_str)
            .map(str::to_string);
        let name = location
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                address
                    .as_deref()
                    .and_then(|address| address.lines().next())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("{latitude:.4},{longitude:.4}"));

        // Google's own place id when present, coordinates otherwise: the id is
        // stable across exports, a coordinate is stable enough.
        let external_ref = location
            .get("placeId")
            .and_then(Value::as_str)
            .map(|id| format!("takeout:timeline:{id}"))
            .unwrap_or_else(|| format!("takeout:timeline:{latitude:.5},{longitude:.5}"));

        let place = Place {
            id: stable_id("place", &external_ref),
            name,
            kind: "venue".into(),
            address,
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "takeout-timeline".into(),
            external_ref: Some(external_ref.clone()),
        };
        if store.upsert_place(&place, today)? {
            report.places_created += 1;
        } else {
            report.places_existing += 1;
        }

        // No rating: a Timeline visit records that they were there, and says
        // nothing about whether they liked it. A review says both, which is why
        // the two sources are kept apart in `source`.
        let visit = PlaceVisit {
            id: stable_id("visit", &external_ref),
            place_id: place.id.clone(),
            visited_on: visit
                .get("duration")
                .and_then(|duration| duration.get("startTimestamp"))
                .and_then(Value::as_str)
                .map(str::to_string),
            rating: None,
            source: "takeout-timeline".into(),
            external_ref: Some(external_ref),
        };
        if store.put_visit(&visit, today)? {
            report.visits_written += 1;
        } else {
            report.visits_existing += 1;
        }
        report.timeline_visits += 1;
    }
    Ok(())
}

pub fn takeout_from(store: &PlacesStore, dir: &Path, today: &str) -> Fallible<TakeoutReport> {
    let mut report = TakeoutReport {
        dirs: 1,
        ..TakeoutReport::default()
    };
    if let Some(labelled) = find_export_file(dir, "Labelled places.json", 3) {
        takeout_labelled(
            store,
            &std::fs::read_to_string(&labelled)?,
            today,
            &mut report,
        )?;
    }
    if let Some(reviews) = find_export_file(dir, "Reviews.json", 3) {
        takeout_reviews(
            store,
            &std::fs::read_to_string(&reviews)?,
            today,
            &mut report,
        )?;
    }

    // The Timeline: where the operator actually went, and when. This is the half
    // that makes a place history useful for planning rather than a list of places
    // that merely exist.
    for path in timeline_files(dir) {
        let body = std::fs::read_to_string(&path)?;
        takeout_timeline(store, &body, today, &mut report)?;
    }

    // Everything else the export holds, named so a reader can see what is not
    // being imported rather than assuming nothing is there.
    for entry in std::fs::read_dir(dir)?.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if matches!(
            name.as_str(),
            "Labelled places.json" | "Reviews.json" | "README.md"
        ) {
            continue;
        }
        report.files_held.push(name);
    }
    report.files_held.sort();
    if dir.join("Commute routes.json").is_file() {
        report.commute_files_held += 1;
    }
    Ok(report)
}

pub fn takeout(store: &PlacesStore, today: &str) -> Fallible<()> {
    let root = sjel_config::overlay_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set; Takeout exports live in the private overlay")?
        .join("data/places/import/raw");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .map_err(|_| format!("raw import directory {} does not exist", root.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("takeout-"))
        })
        .collect();
    if dirs.is_empty() {
        return Err(format!("no takeout-* directory under {}", root.display()).into());
    }
    dirs.sort();
    let mut total = TakeoutReport::default();
    for dir in &dirs {
        let report = takeout_from(store, dir, today)?;
        total.dirs += report.dirs;
        total.labelled += report.labelled;
        total.reviews += report.reviews;
        total.places_created += report.places_created;
        total.places_existing += report.places_existing;
        total.proposals_written += report.proposals_written;
        total.proposals_existing += report.proposals_existing;
        total.commute_files_held += report.commute_files_held;
        total.features_skipped += report.features_skipped;
        total.visits_written += report.visits_written;
        total.visits_existing += report.visits_existing;
        total.timeline_files += report.timeline_files;
        total.timeline_visits += report.timeline_visits;
        total.files_held.extend(report.files_held);
    }
    println!(
        "backfill takeout: {} export dir(s), {} labelled places, {} reviews, \
         {} timeline file(s) yielding {} place visit(s), {} places created, {} \
         already present, {} visits written, {} already recorded, {} register \
         proposals written, {} already present, {} feature(s) without name or point \
         (skipped)",
        total.dirs,
        total.labelled,
        total.reviews,
        total.timeline_files,
        total.timeline_visits,
        total.places_created,
        total.places_existing,
        total.visits_written,
        total.visits_existing,
        total.proposals_written,
        total.proposals_existing,
        total.features_skipped,
    );
    // What the export held and this did not read. Named, because the complaint
    // that produced this line was that a Takeout can carry more than the importer
    // reads and the run reported success either way.
    if !total.files_held.is_empty() {
        total.files_held.sort();
        total.files_held.dedup();
        println!(
            "  not read ({} file(s)): {}",
            total.files_held.len(),
            total.files_held.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_home_city_is_read_plain_or_quoted_and_empty_is_none() {
        let with = |value: &str| BTreeMap::from([("home".to_string(), value.to_string())]);
        assert_eq!(frontmatter_home(&with("Bonn")), Some("Bonn".into()));
        assert_eq!(
            frontmatter_home(&with(" \"Berlin\" ")),
            Some("Berlin".into())
        );
        assert_eq!(frontmatter_home(&with("'Köln'")), Some("Köln".into()));
        for empty in ["", "  ", "[]", "\"\""] {
            assert_eq!(frontmatter_home(&with(empty)), None, "{empty:?}");
        }
        assert_eq!(frontmatter_home(&BTreeMap::new()), None);
    }

    #[test]
    fn the_recomputed_fingerprint_matches_finances_algorithm() {
        // Cross-checked against an independent SHA-256 implementation over the
        // exact tuple capabilities/finance/src/import.rs hashes: parts
        // separated by 0xff, reference empty, EUR implied.
        assert_eq!(
            amex_fingerprint(
                "2026-08-02",
                -1234,
                "SYNTHETIC MARKET BERLIN",
                "liabilities:card:amex",
                0,
            ),
            "b2ad3eca9d16b36389049659015c60ebd58e63712d904ff426cddd752a7baf57"
        );
        // The preserved-repetition rule: the second identical row in one file
        // hashes ("csv-occurrence", base, "1").
        assert_eq!(
            amex_fingerprint(
                "2026-08-02",
                -1234,
                "SYNTHETIC MARKET BERLIN",
                "liabilities:card:amex",
                1,
            ),
            "478fb1067bf98ea5ada39b358998c6dc36f0c73e0662489d147c2be026c4f901"
        );
    }

    #[test]
    fn a_synthetic_csv_row_normalizes_like_finances_import() {
        // The raw export shape: DD/MM/YYYY date, comma decimals, padded
        // description whitespace, quoted multi-line address.
        assert_eq!(
            normalize_date_dmy_slashes("03/08/2026").as_deref(),
            Some("2026-08-03")
        );
        assert_eq!(normalize_date_dmy_slashes("31/02/2026"), None);
        assert_eq!(normalize_date_dmy_slashes("2026-08-03"), None);
        assert_eq!(parse_amount_cents("44,00", ','), Some(4400));
        assert_eq!(parse_amount_cents("1.234,56", ','), Some(123_456));
        assert_eq!(parse_amount_cents("18,13", ','), Some(1813));
        assert_eq!(parse_amount_cents("not-a-number", ','), None);
        assert_eq!(
            normalize_text("SYNTHETIC MARKET        BERLIN"),
            "SYNTHETIC MARKET BERLIN"
        );
        let (street, city) = split_address("EXAMPLE STR. 1\nBERLIN", "");
        assert_eq!(street.as_deref(), Some("EXAMPLE STR. 1"));
        assert_eq!(city.as_deref(), Some("BERLIN"));
        let (street, city) = split_address("EXAMPLE STR. 1\nIGNORED", "MUNICH");
        assert_eq!(street.as_deref(), Some("EXAMPLE STR. 1"));
        assert_eq!(city.as_deref(), Some("MUNICH"));
        let (street, city) = split_address("ONLY ONE LINE", "");
        assert_eq!(street.as_deref(), Some("ONLY ONE LINE"));
        assert_eq!(city, None);
    }

    #[test]
    fn the_amex_profile_is_read_from_the_finance_config_shape() {
        let config = json!({
            "csv_mappings": [
                { "label": "Some bank", "mapping": { "date_column": "Buchung" } },
                {
                    "label": "American Express CSV export",
                    "mapping": {
                        "delimiter": ",",
                        "decimal_separator": ",",
                        "date_column": "Datum",
                        "amount_column": "Betrag",
                        "description_column": "Beschreibung",
                        "source_account": "liabilities:card:synthetic",
                        "amount_sign": "invert"
                    }
                }
            ]
        });
        let profile = amex_profile_from(&config).unwrap();
        assert_eq!(profile.source_account, "liabilities:card:synthetic");
        assert!(profile.invert);
        assert_eq!(profile.delimiter, b',');
        assert!(amex_profile_from(&json!({ "csv_mappings": [] })).is_err());
    }

    #[test]
    fn city_tokens_are_matched_conservatively_from_the_end() {
        // Synthetic descriptors only (the Musterstadt convention) — never
        // verbatim live transaction text in this public file.
        let cities: HashSet<String> = ["MUSTERSTADT", "BAD MUSTERSTADT", "BEISPIELHAUSEN"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            trailing_city("MARKT 12345678 MUSTERSTADT", &cities),
            Some("MUSTERSTADT")
        );
        // The longest match wins, and whitespace runs collapse first.
        assert_eq!(
            trailing_city("CAFE   BAD MUSTERSTADT", &cities),
            Some("BAD MUSTERSTADT")
        );
        assert_eq!(
            trailing_city("PAYPAL *SYNTHETIC GAME 4029357733", &cities),
            None
        );
        // A city as word prefix, not trailing token, never matches.
        assert_eq!(trailing_city("MUSTERSTADTER KINDL", &cities), None);
        assert!(plausible_city("BAD MUSTERSTADT"));
        assert!(!plausible_city("4029357733"));
        assert!(!plausible_city("A0"));
    }

    /// The documented shape, exactly: locationhistoryformat.com's reference, read
    /// 2026-09-23. Pinned here so a divergence fails loudly instead of importing
    /// nothing and reporting success.
    const TIMELINE_MONTH: &str = r#"{
      "timelineObjects": [
        { "activitySegment": {
            "startLocation": { "latitudeE7": 525110000, "longitudeE7": 134260000 },
            "endLocation": { "latitudeE7": 525050000, "longitudeE7": 134410000 },
            "duration": { "startTimestamp": "2025-12-19T14:22:00Z",
                          "endTimestamp": "2025-12-19T14:51:00Z" },
            "activityType": "IN_TRAIN" } },
        { "placeVisit": {
            "location": { "latitudeE7": 525025000, "longitudeE7": 134430000,
                          "placeId": "ChIJdarkmatter", "name": "DARK MATTER",
                          "address": "Koe penicker Str. 70\n10179 Berlin" },
            "duration": { "startTimestamp": "2025-12-21T19:05:00Z",
                          "endTimestamp": "2025-12-21T21:40:00Z" },
            "placeConfidence": "HIGH_CONFIDENCE",
            "visitConfidence": 92 } },
        { "placeVisit": {
            "location": { "latitudeE7": 525500000, "longitudeE7": 135000000,
                          "address": "Somewhere Without A Name\nBerlin" },
            "duration": { "startTimestamp": "2025-12-20T11:00:00Z",
                          "endTimestamp": "2025-12-20T12:00:00Z" } } }
      ]
    }"#;

    fn scratch(name: &str) -> (PlacesStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("places-timeline-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (PlacesStore::open(&dir.join("axon.db")).unwrap(), dir)
    }

    #[test]
    fn a_timeline_month_yields_one_visit_per_place_visit_and_skips_journeys() {
        // A journey between two places is not a place, and the two Berlin trips are
        // already recorded as plans. The activitySegment must not become a row.
        let (store, dir) = scratch("parse");
        let mut report = TakeoutReport::default();
        takeout_timeline(&store, TIMELINE_MONTH, "2026-09-23", &mut report).unwrap();

        assert_eq!(report.timeline_files, 1);
        assert_eq!(
            report.timeline_visits, 2,
            "two placeVisit objects, one journey skipped"
        );
        assert_eq!(report.places_created, 2);
        assert_eq!(report.visits_written, 2);

        let rows = store.visits().unwrap();
        assert_eq!(rows.len(), 2);
        let dark = rows
            .iter()
            .find(|row| row.place_name == "DARK MATTER")
            .expect("the named visit is there by name");
        assert_eq!(
            dark.visited_on.as_deref(),
            Some("2025-12-21T19:05:00Z"),
            "the ISO startTimestamp is the date, not a millisecond integer"
        );
        assert_eq!(
            dark.rating, None,
            "a Timeline visit says nothing about liking it"
        );
        assert!(
            (dark.latitude.unwrap() - 52.5025).abs() < 1e-6,
            "E7 is degrees times 10^7"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_visit_with_no_name_is_named_by_its_address_first_line() {
        // Google only names places it recognises, and most Timeline visits are a
        // coordinate. A row named by its address is worth more than a bare pair.
        let (store, dir) = scratch("unnamed");
        let mut report = TakeoutReport::default();
        takeout_timeline(&store, TIMELINE_MONTH, "2026-09-23", &mut report).unwrap();
        let rows = store.visits().unwrap();
        assert!(
            rows.iter()
                .any(|row| row.place_name == "Somewhere Without A Name"),
            "the address's first line became the name: {:?}",
            rows.iter().map(|r| &r.place_name).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_timeline_import_is_idempotent_by_google_place_id() {
        let (store, dir) = scratch("idempotent");
        let mut first = TakeoutReport::default();
        takeout_timeline(&store, TIMELINE_MONTH, "2026-09-23", &mut first).unwrap();
        let mut again = TakeoutReport::default();
        takeout_timeline(&store, TIMELINE_MONTH, "2026-09-23", &mut again).unwrap();
        assert_eq!(
            again.places_created, 0,
            "the same export twice adds no place"
        );
        assert_eq!(again.visits_written, 0, "and no second visit");
        assert_eq!(again.visits_existing, 2);
        assert_eq!(store.visits().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_month_file_without_timeline_objects_is_counted_and_yields_nothing() {
        // A layout this does not know, not an empty month. The distinction matters
        // because "imported 0" must not read as "there was nothing there".
        let (store, dir) = scratch("unknown");
        let mut report = TakeoutReport::default();
        takeout_timeline(
            &store,
            r#"{"somethingElse": []}"#,
            "2026-09-23",
            &mut report,
        )
        .unwrap();
        assert_eq!(report.timeline_files, 1, "the file was seen");
        assert_eq!(report.timeline_visits, 0);
        assert!(store.visits().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_google_moved_is_still_found() {
        // `Reviews.json` sits at the export root in old exports and under
        // `Maps (your places)/` in new ones. Reading only the root meant a
        // relayout imported nothing and reported success.
        let dir = std::env::temp_dir().join("places-timeline-layout");
        let _ = std::fs::remove_dir_all(&dir);
        let nested = dir.join("Maps (your places)").join("Saved");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Reviews.json"), "{}").unwrap();

        assert!(find_export_file(&dir, "Reviews.json", 3).is_some());
        assert!(find_export_file(&dir, "Nowhere.json", 3).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_semantic_location_history_tree_is_walked_by_shape_not_by_path() {
        let dir = std::env::temp_dir().join("places-timeline-walk");
        let _ = std::fs::remove_dir_all(&dir);
        let months = dir
            .join("Location History")
            .join("Semantic Location History")
            .join("2025");
        std::fs::create_dir_all(&months).unwrap();
        std::fs::write(months.join("2025_DECEMBER.json"), "{}").unwrap();
        std::fs::write(months.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("Reviews.json"), "{}").unwrap();

        let found = timeline_files(&dir);
        assert_eq!(found.len(), 1, "one month file, and not the stray txt");
        assert!(found[0].ends_with("2025_DECEMBER.json"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn traveler_names_and_destinations_parse_the_trips_shapes() {
        assert_eq!(
            traveler_names(r#"["Synthetic Person", "  ", "Second Person"]"#),
            vec!["Synthetic Person".to_string(), "Second Person".to_string()]
        );
        assert!(traveler_names("[]").is_empty());
        assert!(traveler_names("not json").is_empty());

        // The first destination with coordinates wins; kinds map onto the
        // registry's vocabulary (airport → station, trips' default → station).
        let destinations = r#"[
            {"id": "d0", "name": "Nowhere", "kind": "city"},
            {"id": "d1", "name": "  Musterstadt ", "kind": "city",
             "latitude": 50.0, "longitude": 7.0},
            {"id": "d2", "name": "Later", "kind": "venue",
             "latitude": 51.0, "longitude": 8.0}
        ]"#;
        assert_eq!(
            first_located_destination(destinations),
            Some(("Musterstadt".into(), "city".into(), 50.0, 7.0))
        );
        let airport = r#"[{"id": "d3", "name": "Synthetic Airport", "kind": "airport",
                           "latitude": 50.1, "longitude": 7.1}]"#;
        assert_eq!(
            first_located_destination(airport).map(|d| d.1),
            Some("station".into())
        );
        assert_eq!(first_located_destination("[]"), None);
        assert_eq!(
            first_located_destination(r#"[{"id": "d4", "name": "No coords"}]"#),
            None
        );
    }

    #[test]
    fn frontmatter_coordinates_parse_both_vault_spellings() {
        let list_form = "---\ntype: museum\ncity: New York\ncoordinates:\n  - \"40.7033\"\n  - \"-73.9894\"\nicon: pin\n---\nbody\n";
        let (scalars, lists) = parse_frontmatter(list_form);
        assert_eq!(
            frontmatter_coordinates(&scalars, &lists),
            Some((40.7033, -73.9894))
        );
        assert_eq!(scalars.get("city").map(String::as_str), Some("New York"));

        let inline_form = "---\nicon: user-round\ncoordinates: 49.8920,8.6480\n---\n";
        let (scalars, lists) = parse_frontmatter(inline_form);
        assert_eq!(
            frontmatter_coordinates(&scalars, &lists),
            Some((49.892, 8.648))
        );

        let empty = "---\ncoordinates: []\n---\n";
        let (scalars, lists) = parse_frontmatter(empty);
        assert_eq!(frontmatter_coordinates(&scalars, &lists), None);

        let absent = "no frontmatter at all";
        let (scalars, lists) = parse_frontmatter(absent);
        assert_eq!(frontmatter_coordinates(&scalars, &lists), None);
    }

    #[test]
    fn haversine_matches_a_known_distance() {
        // Vienna Hauptbahnhof-ish to Vienna centre: a few kilometres.
        let d = haversine_km((48.1849, 16.3673), (48.2085, 16.3721));
        assert!(d > 2.0 && d < 4.0, "got {d}");
        // Vienna to Darmstadt: several hundred kilometres, never "nearby".
        let far = haversine_km((48.1849, 16.3673), (49.8920, 8.6480));
        assert!(far > 500.0, "got {far}");
    }

    #[test]
    fn google_fixed_labels_are_the_users_own_anchors() {
        assert_eq!(takeout_person_label("Home"), None);
        assert_eq!(takeout_person_label("Work"), None);
        assert_eq!(takeout_person_label("home"), None);
    }

    #[test]
    fn a_typed_label_names_a_person_verbatim() {
        // The qualifier drops; the name is NOT de-possessivized — "Marinas"
        // stays "Marinas" (review fixes identity), "Lars Home" must not
        // become "Lar".
        assert_eq!(takeout_person_label("Marinas Home"), Some("Marinas".into()));
        assert_eq!(takeout_person_label("Bryan Work"), Some("Bryan".into()));
        assert_eq!(takeout_person_label("Bryan"), Some("Bryan".into()));
        assert_eq!(takeout_person_label("Lars Home"), Some("Lars".into()));
    }

    #[test]
    fn geojson_points_arrive_lon_lat_and_leave_lat_lon() {
        let feature = json!({
            "geometry": { "coordinates": [16.3673, 48.1849], "type": "Point" }
        });
        assert_eq!(geojson_point(&feature), Some((48.1849, 16.3673)));
        assert_eq!(geojson_point(&json!({"geometry": {}})), None);
    }
}

/// Database-backed backfill tests. Synthetic fixtures only (the Musterstadt
/// convention); the neighbour tables the backfills read
/// (`finance_transaction_candidates`, `trips_plans`) are scratch copies under a test
/// prefix in the test's own file, which is what the prefix parameters exist for.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::geocode::db_tests::{stub, stub_with};
    use crate::store::db_tests::open_test_store;
    use rusqlite::Connection;
    use std::sync::atomic::Ordering;

    /// The scratch prefix the neighbour fixtures live under. A prefix rather than the
    /// real `finance`/`trips` here, because these tests state a *narrower* shape than
    /// those capabilities actually migrate -- the columns each query projects, no more.
    const SCRATCH: &str = "scratch";

    const SOURCE_ACCOUNT: &str = "liabilities:card:synthetic";
    const VENUE_ITEM: &str = r#"[{"osm_type":"node","osm_id":7001,"lat":"50.0002","lon":"7.0002","name":"Synthetic Market","display_name":"Synthetic Market, Musterstadt","addresstype":"shop","address":{"town":"Musterstadt","country_code":"de"}}]"#;
    const CITY_ITEM: &str = r#"[{"osm_type":"relation","osm_id":7002,"lat":"50.0","lon":"7.0","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}]"#;

    fn transaction_places(store: &PlacesStore) -> Vec<(String, String, String)> {
        let conn = store.conn().unwrap();
        conn.query_all(
            &format!(
                "SELECT source_id, precision, source FROM {}_transaction_places
                 ORDER BY source_id",
                store.prefix()
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn candidate_locations_wait_for_the_finance_migration() {
        let (store, path) = open_test_store("cands_absent");
        // The candidates table as it existed before finance's ALTER TABLE added
        // the location columns.
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE {SCRATCH}_transaction_candidates (
                     fingerprint TEXT PRIMARY KEY,
                     source_account TEXT NOT NULL
                 )"
            ))
            .unwrap();
        // Port 1 refuses connections: proving no geocode is even attempted.
        let geocoder = Geocoder::with_url(&store, "http://127.0.0.1:1/search".into());
        let phase = amex_from_candidates(
            &store,
            &geocoder,
            SCRATCH,
            SOURCE_ACCOUNT,
            &HashSet::new(),
            "2026-08-25",
        )
        .unwrap();
        assert!(phase.is_none(), "absent columns mean no candidate phase");
    }

    /// `pragma_table_info` returns no rows for a table that is not there, which is
    /// the case the `information_schema` probe answered with a zero count. A missing
    /// neighbour table must read as "no candidate phase", never as an error.
    #[test]
    fn an_absent_candidates_table_reads_as_no_candidate_phase() {
        let (store, _path) = open_test_store("cands_missing");
        let geocoder = Geocoder::with_url(&store, "http://127.0.0.1:1/search".into());
        let phase = amex_from_candidates(
            &store,
            &geocoder,
            SCRATCH,
            SOURCE_ACCOUNT,
            &HashSet::new(),
            "2026-08-25",
        )
        .unwrap();
        assert!(phase.is_none(), "a missing table is not an error here");
    }

    #[test]
    fn stored_candidate_locations_link_venues_and_fall_back_to_the_city() {
        let (store, path) = open_test_store("cands");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE {SCRATCH}_transaction_candidates (
                     fingerprint TEXT PRIMARY KEY,
                     source_account TEXT NOT NULL,
                     location_street TEXT,
                     location_postal_code TEXT,
                     location_city TEXT,
                     location_country TEXT
                 )"
            ))
            .unwrap();
            // The contract shape: Adresse verbatim with its embedded newline
            // (street on line 1, city on line 2), PLZ and Land beside it.
            // fp-cityonly is a whitespace-only Adresse with a filled Stadt: no
            // street exists, so the venue path must never see it (D1).
            //
            // SQLite has no `E'...'` escape literal; the newline is a bound
            // parameter's own character instead.
            let street = "Beispielstr. 1\nMusterstadt";
            let unresolvable = "Unresolvable Weg 9\nMusterstadt";
            let sql = format!(
                "INSERT INTO {SCRATCH}_transaction_candidates VALUES
                 ('fp-venue',    ?1, ?2, '12345', NULL, 'Deutschland'),
                 ('fp-cityfb',   ?1, ?3, '12345', NULL, 'Deutschland'),
                 ('fp-cityonly', ?1, ?4, '12345', 'Musterstadt', 'Deutschland'),
                 ('fp-linked',   ?1, ?2, '12345', NULL, 'Deutschland'),
                 ('fp-noloc',    ?1, NULL, NULL, NULL, NULL),
                 ('fp-other', 'liabilities:card:other', ?2, NULL, NULL, NULL)"
            );
            conn.execute(&sql, params![SOURCE_ACCOUNT, street, unresolvable, "\n"])
                .unwrap();
        }
        // fp-linked already carries a link (a previous run's), so this run
        // must leave it alone.
        let existing = Place {
            id: stable_id("place", "test:prelinked"),
            name: "Prelinked Venue".into(),
            kind: "venue".into(),
            address: None,
            city: Some("Musterstadt".into()),
            country_code: Some("DE".into()),
            latitude: Some(50.0),
            longitude: Some(7.0),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&existing, "2026-08-25").unwrap();
        store
            .link_transaction(
                "fp-linked",
                &existing.id,
                "venue",
                9000,
                "test",
                "2026-08-25",
            )
            .unwrap();

        // The stub resolves the good street, misses the bad one, and knows the
        // city, exercising both halves of the venue-then-city path.
        let (url, hits) = stub_with(|request: &str| {
            if request.contains("street=Beispielstr") {
                VENUE_ITEM.to_string()
            } else if request.contains("street=") {
                "[]".to_string()
            } else {
                CITY_ITEM.to_string()
            }
        });
        let geocoder = Geocoder::with_url(&store, url);

        let (stats, covered) = amex_from_candidates(
            &store,
            &geocoder,
            SCRATCH,
            SOURCE_ACCOUNT,
            &store.linked_source_ids().unwrap(),
            "2026-08-25",
        )
        .unwrap()
        .expect("the location columns exist");

        // fp-noloc has no location_street and fp-other is another account:
        // neither is a candidate of this phase.
        assert_eq!(covered.len(), 4);
        assert!(covered.contains("fp-venue") && covered.contains("fp-cityfb"));
        assert_eq!(stats.linked, 1);
        assert_eq!(
            stats.city_fallback, 2,
            "the venue miss and the street-less row each earn a city link"
        );
        assert_eq!(stats.skipped_linked, 1);
        assert_eq!(stats.not_found, 0);
        assert_eq!(
            transaction_places(&store),
            vec![
                (
                    "fp-cityfb".to_string(),
                    "city".to_string(),
                    "amex-city-fallback".to_string()
                ),
                (
                    "fp-cityonly".to_string(),
                    "city".to_string(),
                    "amex-city-fallback".to_string()
                ),
                (
                    "fp-linked".to_string(),
                    "venue".to_string(),
                    "test".to_string()
                ),
                (
                    "fp-venue".to_string(),
                    "venue".to_string(),
                    "amex-backfill".to_string()
                ),
            ]
        );
        let first_run_hits = hits.load(Ordering::SeqCst);
        assert_eq!(
            first_run_hits, 3,
            "venue hit + venue miss + city hit; the street-less row's city \
             query is served from the cache and never reaches the venue path"
        );

        // Idempotent: a re-run links nothing new and never re-asks the
        // provider (every row is now linked, every query cached).
        let (stats, _) = amex_from_candidates(
            &store,
            &geocoder,
            SCRATCH,
            SOURCE_ACCOUNT,
            &store.linked_source_ids().unwrap(),
            "2026-08-25",
        )
        .unwrap()
        .unwrap();
        assert_eq!(stats.linked, 0);
        assert_eq!(stats.city_fallback, 0);
        assert_eq!(stats.skipped_linked, 4);
        assert_eq!(hits.load(Ordering::SeqCst), first_run_hits);
    }

    #[test]
    fn travelers_become_proposals_and_never_confirmed_rows() {
        let (store, path) = open_test_store("travelers");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE {SCRATCH}_plans (
                     id TEXT PRIMARY KEY,
                     travelers TEXT NOT NULL,
                     destinations TEXT NOT NULL,
                     date_start TEXT NOT NULL,
                     date_end TEXT NOT NULL,
                     status TEXT NOT NULL
                 );
                 INSERT INTO {SCRATCH}_plans VALUES
                 ('plan-1', '[\"Synthetic Person\", \"Second Person\"]',
                  '[{{\"id\": \"d1\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                      \"latitude\": 50.0, \"longitude\": 7.0}}]',
                  '2026-09-01', '2026-09-05', 'saved'),
                 ('plan-2', '[\"Synthetic Person\"]',
                  '[{{\"id\": \"d2\", \"name\": \"Nowhere\", \"kind\": \"city\"}}]',
                  '2026-10-01', '2026-10-03', 'saved'),
                 ('plan-3', '[\"Synthetic Person\"]',
                  '[{{\"id\": \"d3\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                      \"latitude\": 50.0, \"longitude\": 7.0}}]',
                  '2026-11-01', '2026-11-03', 'archived'),
                 ('plan-4', '[]',
                  '[{{\"id\": \"d4\", \"name\": \"Musterstadt\", \"kind\": \"city\",
                      \"latitude\": 50.0, \"longitude\": 7.0}}]',
                  '2026-12-01', '2026-12-03', 'saved')"
            ))
            .unwrap();
        }

        let report = travelers_from(&store, SCRATCH, "2026-08-25").unwrap();
        assert_eq!(report.plans, 3, "the archived plan is not read");
        assert_eq!(report.without_located_destination, 1);
        assert_eq!(report.pairs, 2);
        assert_eq!(report.proposals_written, 2);
        assert_eq!(report.places_created, 1);

        let proposed = store.person_places_in_state("proposed").unwrap();
        assert_eq!(proposed.len(), 2);
        for row in &proposed {
            assert_eq!(row.state, "proposed");
            assert_eq!(row.source, "trips-travelers");
            assert_eq!(row.date_start.as_deref(), Some("2026-09-01"));
            assert_eq!(row.date_end.as_deref(), Some("2026-09-05"));
            assert_eq!(row.confidence_bp, 5000);
        }
        // PLC-7: derivation must never produce a confirmed row.
        assert!(store
            .person_places_in_state("confirmed")
            .unwrap()
            .is_empty());

        // Idempotent by (person, plan): a re-run writes nothing new.
        let again = travelers_from(&store, SCRATCH, "2026-08-25").unwrap();
        assert_eq!(again.proposals_written, 0);
        assert_eq!(again.proposals_existing, 2);
        assert_eq!(again.places_created, 0);
    }

    #[test]
    fn an_unnamed_coordinate_is_named_by_reverse_geocode_or_kept_bare() {
        let (store, _path) = open_test_store("revname");
        let (url, _hits) = stub(
            r#"{"osm_type":"relation","osm_id":7003,"lat":"50.10","lon":"7.10","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}"#,
        );
        let geocoder = Geocoder::with_url(&store, url);
        let (place_id, named) =
            resolve_unnamed_place(&store, &geocoder, (50.1234, 7.1234), "2026-08-25").unwrap();
        assert!(named);
        let place = store.place(&place_id).unwrap().unwrap();
        assert_eq!(place.name, "Musterstadt");
        assert_eq!(place.kind, "city");

        // When the provider knows nothing there, the row stays a bare
        // coordinate — reported precision, never a guessed name (D1).
        let (url, _hits) = stub(r#"{"error":"Unable to geocode"}"#);
        let geocoder = Geocoder::with_url(&store, url);
        let (place_id, named) =
            resolve_unnamed_place(&store, &geocoder, (10.9876, 9.8765), "2026-08-25").unwrap();
        assert!(!named);
        let place = store.place(&place_id).unwrap().unwrap();
        assert_eq!(place.name, "10.9876,9.8765");
        assert_eq!(place.kind, "address");
    }

    #[test]
    fn takeout_labels_become_places_and_person_labels_only_propose() {
        let (store, path) = open_test_store("takeout");
        let dir = path.parent().unwrap().join("takeout-19700101");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Labelled places.json"),
            r#"{"type":"FeatureCollection","features":[
                {"geometry":{"coordinates":[7.0,50.0],"type":"Point"},
                 "properties":{"address":"Musterweg 1, Musterstadt","name":"Home"}},
                {"geometry":{"coordinates":[7.1,50.1],"type":"Point"},
                 "properties":{"address":"Beispielgasse 2, Musterstadt","name":"Maxims Home"}}
            ]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("Reviews.json"),
            r#"{"type":"FeatureCollection","features":[
                {"geometry":{"coordinates":[7.2,50.2],"type":"Point"},
                 "properties":{"google_maps_url":"http://maps.example/?cid=42",
                     "location":{"address":"Marktplatz 3","country_code":"DE","name":"Synthetic Market"},
                     "date":"2025-10-31T19:02:16Z","five_star_rating_published":5}}
            ]}"#,
        )
        .unwrap();
        // A commute file is counted as held, never parsed.
        std::fs::write(dir.join("Commute routes.json"), "{}").unwrap();

        let report = takeout_from(&store, &dir, "2026-09-02").unwrap();
        assert_eq!(report.labelled, 2);
        assert_eq!(report.reviews, 1);
        assert_eq!(report.places_created, 3);
        assert_eq!(report.proposals_written, 1);
        assert_eq!(report.commute_files_held, 1);

        // PLC-7: the person label produced a `proposed` row for the person
        // verbatim; nothing entered `confirmed`.
        let proposed = store.person_places_in_state("proposed").unwrap();
        assert_eq!(proposed.len(), 1);
        assert_eq!(proposed[0].person, "Maxims");
        assert!(store
            .person_places_in_state("confirmed")
            .unwrap()
            .is_empty());
        let venue = store
            .place_by_external_ref("takeout:review:http://maps.example/?cid=42")
            .unwrap()
            .unwrap();
        assert_eq!(venue.kind, "venue");
        assert_eq!(venue.country_code.as_deref(), Some("DE"));

        // Second run: same stable ids, nothing duplicated.
        let again = takeout_from(&store, &dir, "2026-09-02").unwrap();
        assert_eq!(again.places_created, 0);
        assert_eq!(again.places_existing, 3);
        assert_eq!(again.proposals_written, 0);
        assert_eq!(again.proposals_existing, 1);
    }
}

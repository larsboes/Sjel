//! Broker-neutral investment activity preview and reviewed aggregate snapshot.
//!
//! Raw rows stop at `preview_csv`. Confirmation re-runs that function and writes
//! only the aggregate holdings below; source identifiers, mappings and CSV bytes
//! never enter the canonical snapshot or the disposable database projection.

use crate::import::{normalize_date, ImportError, ImportResult};
use candidate_fingerprint::digest;
use csv::StringRecord;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldingsCoverage {
    #[default]
    Complete,
    Partial,
}

impl HoldingsCoverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "complete" => Some(Self::Complete),
            "partial" => Some(Self::Partial),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentCsvMapping {
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    #[serde(default = "default_decimal_separator")]
    pub decimal_separator: char,
    pub date_column: String,
    pub instrument_column: String,
    pub quantity_column: String,
    #[serde(default)]
    pub activity_type_column: Option<String>,
    #[serde(default)]
    pub position_activity_values: Vec<String>,
    #[serde(default)]
    pub non_position_activity_values: Vec<String>,
    #[serde(default)]
    pub reference_column: Option<String>,
    #[serde(default)]
    pub price_column: Option<String>,
    #[serde(default)]
    pub currency_column: Option<String>,
    #[serde(default = "default_currency")]
    pub default_currency: String,
    #[serde(default)]
    pub instrument_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    #[serde(
        serialize_with = "serialize_i64_as_string",
        deserialize_with = "deserialize_i64_from_string"
    )]
    pub mantissa: i64,
    pub scale: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub instrument: String,
    pub quantity: Quantity,
    pub latest_unit_price: Option<Quantity>,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentPreview {
    pub snapshot_id: String,
    pub activity_count: usize,
    pub duplicate_rows: usize,
    pub ignored_non_position_rows: usize,
    pub closed_positions: usize,
    pub holdings: Vec<Holding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedHoldingsSnapshot {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub reviewed_at: String,
    #[serde(default)]
    pub coverage: HoldingsCoverage,
    pub holdings: Vec<Holding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ReviewedHoldingsSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedHoldingsSource {
    pub source_key: String,
    pub snapshot_id: String,
    pub reviewed_at: String,
    #[serde(default)]
    pub coverage: HoldingsCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortfolioValuation {
    pub currency: String,
    pub value: DecimalValue,
    pub priced_holdings: usize,
    pub unpriced_holdings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecimalValue {
    #[serde(serialize_with = "serialize_i128_as_string")]
    pub mantissa: i128,
    pub scale: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReviewedHoldingsCollection {
    schema_version: u32,
    sources: BTreeMap<String, ReviewedHoldingsSnapshot>,
}

#[derive(Debug)]
struct Activity {
    date: String,
    instrument: String,
    quantity: Quantity,
    unit_price: Option<Quantity>,
    currency: String,
}

#[derive(Debug)]
struct Position {
    quantity: Quantity,
    latest_price: Option<(String, Quantity)>,
    currency: String,
}

pub fn portfolio_valuations(
    snapshot: &ReviewedHoldingsSnapshot,
) -> ImportResult<Vec<PortfolioValuation>> {
    let mut values = BTreeMap::<String, PortfolioValuation>::new();
    for holding in &snapshot.holdings {
        let valuation =
            values
                .entry(holding.currency.clone())
                .or_insert_with(|| PortfolioValuation {
                    currency: holding.currency.clone(),
                    value: DecimalValue {
                        mantissa: 0,
                        scale: 0,
                    },
                    priced_holdings: 0,
                    unpriced_holdings: 0,
                });
        match &holding.latest_unit_price {
            Some(price) => {
                let position_value = multiply(&holding.quantity, price)?;
                valuation.value = if valuation.value.mantissa == 0 {
                    position_value
                } else {
                    add_values(&valuation.value, &position_value)?
                };
                valuation.priced_holdings += 1;
            }
            None => valuation.unpriced_holdings += 1,
        }
    }
    Ok(values.into_values().collect())
}

pub fn preview_csv(
    bytes: &[u8],
    mapping: &InvestmentCsvMapping,
) -> ImportResult<InvestmentPreview> {
    validate_mapping(mapping)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(mapping.delimiter as u8)
        .flexible(false)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| ImportError("CSV header could not be read".into()))?
        .clone();
    let date = column(&headers, &mapping.date_column)?;
    let instrument = column(&headers, &mapping.instrument_column)?;
    let quantity = column(&headers, &mapping.quantity_column)?;
    let activity_type = optional_column(&headers, mapping.activity_type_column.as_deref())?;
    let reference = optional_column(&headers, mapping.reference_column.as_deref())?;
    let price = optional_column(&headers, mapping.price_column.as_deref())?;
    let currency = optional_column(&headers, mapping.currency_column.as_deref())?;

    let mut activities = Vec::new();
    let mut seen = HashSet::new();
    let mut duplicate_rows = 0;
    let mut ignored_non_position_rows = 0;
    for record in reader.records() {
        let record = record.map_err(|_| ImportError("CSV row does not match its header".into()))?;
        let raw_instrument = record.get(instrument).unwrap_or_default().trim();
        let raw_quantity = record.get(quantity).unwrap_or_default().trim();
        if raw_quantity.is_empty() {
            ignored_non_position_rows += 1;
            continue;
        }
        let quantity = parse_quantity(raw_quantity, mapping.decimal_separator)?;
        if quantity.mantissa == 0 {
            ignored_non_position_rows += 1;
            continue;
        }
        if let Some(index) = activity_type {
            let value = record.get(index).unwrap_or_default().trim();
            if mapping
                .non_position_activity_values
                .iter()
                .any(|candidate| candidate == value)
            {
                ignored_non_position_rows += 1;
                continue;
            }
            if !mapping
                .position_activity_values
                .iter()
                .any(|candidate| candidate == value)
            {
                return Err(ImportError(
                    "a nonzero quantity activity type is not classified".into(),
                ));
            }
        }
        if raw_instrument.is_empty() {
            return Err(ImportError(
                "a nonzero quantity requires an instrument".into(),
            ));
        }
        let instrument = mapping
            .instrument_aliases
            .get(raw_instrument)
            .map(String::as_str)
            .unwrap_or(raw_instrument)
            .trim()
            .to_string();
        validate_instrument(&instrument)?;
        let date = normalize_date(record.get(date).unwrap_or_default())?;
        let source_reference = reference
            .and_then(|index| record.get(index))
            .map(str::trim)
            .unwrap_or_default();
        let unit_price = price
            .and_then(|index| record.get(index))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| parse_decimal(value, mapping.decimal_separator, "price"))
            .transpose()?;
        let currency = currency
            .and_then(|index| record.get(index))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&mapping.default_currency)
            .to_ascii_uppercase();
        validate_currency(&currency)?;
        // The same 0xff-terminated primitive the candidate identity uses, over
        // this ledger's own tuple. Shared so the digest cannot drift; the tuple
        // is stated here because it is this file's, not the candidate's.
        let id = digest(&[
            &date,
            &instrument,
            &quantity.mantissa.to_string(),
            &quantity.scale.to_string(),
            source_reference,
            &unit_price
                .as_ref()
                .map(|value| value.mantissa.to_string())
                .unwrap_or_default(),
            &unit_price
                .as_ref()
                .map(|value| value.scale.to_string())
                .unwrap_or_default(),
            &currency,
        ]);
        if !seen.insert(id.clone()) {
            duplicate_rows += 1;
            continue;
        }
        activities.push(Activity {
            date,
            instrument,
            quantity,
            unit_price,
            currency,
        });
    }

    let mut positions: BTreeMap<String, Position> = BTreeMap::new();
    for activity in &activities {
        let position = positions
            .entry(activity.instrument.clone())
            .or_insert_with(|| Position {
                quantity: Quantity {
                    mantissa: 0,
                    scale: activity.quantity.scale,
                },
                latest_price: None,
                currency: activity.currency.clone(),
            });
        if position.currency != activity.currency {
            return Err(ImportError(
                "one instrument cannot use multiple currencies".into(),
            ));
        }
        position.quantity = add(&position.quantity, &activity.quantity)?;
        if let Some(price) = &activity.unit_price {
            let replace = position
                .latest_price
                .as_ref()
                .is_none_or(|(date, _)| activity.date >= *date);
            if replace {
                position.latest_price = Some((activity.date.clone(), price.clone()));
            }
        }
    }

    let closed_positions = positions
        .values()
        .filter(|position| position.quantity.mantissa == 0)
        .count();
    let holdings: Vec<Holding> = positions
        .into_iter()
        .filter(|(_, position)| position.quantity.mantissa != 0)
        .map(|(instrument, position)| Holding {
            instrument,
            quantity: position.quantity,
            latest_unit_price: position.latest_price.map(|(_, price)| price),
            currency: position.currency,
        })
        .collect();
    let snapshot_id = holdings_identity(&holdings);
    Ok(InvestmentPreview {
        snapshot_id,
        activity_count: activities.len(),
        duplicate_rows,
        ignored_non_position_rows,
        closed_positions,
        holdings,
    })
}

pub fn reviewed_snapshot(
    preview: &InvestmentPreview,
    mapping: &InvestmentCsvMapping,
    reviewed_at: &str,
    coverage: HoldingsCoverage,
) -> ImportResult<ReviewedHoldingsSnapshot> {
    let approved_aliases: HashSet<_> = mapping.instrument_aliases.values().collect();
    if preview
        .holdings
        .iter()
        .any(|holding| !approved_aliases.contains(&holding.instrument))
    {
        return Err(ImportError(
            "every confirmed instrument requires an explicit private alias".into(),
        ));
    }
    Ok(ReviewedHoldingsSnapshot {
        schema_version: 1,
        snapshot_id: preview.snapshot_id.clone(),
        reviewed_at: reviewed_at.to_string(),
        coverage,
        holdings: preview.holdings.clone(),
        sources: Vec::new(),
    })
}

pub fn read_reviewed_snapshot(path: &Path) -> ImportResult<Option<ReviewedHoldingsSnapshot>> {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ImportError("holdings snapshot could not be read".into())),
    };
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| ImportError("holdings snapshot has an unsupported shape".into()))?;
    if value
        .get("sources")
        .is_some_and(serde_json::Value::is_object)
    {
        let collection: ReviewedHoldingsCollection = serde_json::from_value(value)
            .map_err(|_| ImportError("holdings snapshot has an unsupported shape".into()))?;
        return aggregate_collection(&collection).map(Some);
    }
    let snapshot: ReviewedHoldingsSnapshot = serde_json::from_value(value)
        .map_err(|_| ImportError("holdings snapshot has an unsupported shape".into()))?;
    validate_source_snapshot(&snapshot)?;
    Ok(Some(snapshot))
}

/// Atomically replace one source inside the canonical private collection.
/// Reconfirming the same source identity on the same date is a no-op. A later
/// review advances source freshness even when the quantities are unchanged.
pub fn write_reviewed_snapshot(
    path: &Path,
    source_key: &str,
    snapshot: &ReviewedHoldingsSnapshot,
) -> ImportResult<bool> {
    validate_source_key(source_key)?;
    validate_source_snapshot(snapshot)?;
    let mut collection = read_collection(path)?.unwrap_or(ReviewedHoldingsCollection {
        schema_version: 2,
        sources: BTreeMap::new(),
    });
    if collection.sources.get(source_key).is_some_and(|existing| {
        existing.snapshot_id == snapshot.snapshot_id
            && existing.reviewed_at == snapshot.reviewed_at
            && existing.coverage == snapshot.coverage
    }) {
        return Ok(false);
    }
    collection
        .sources
        .insert(source_key.to_string(), snapshot.clone());
    aggregate_collection(&collection)?;
    let parent = path
        .parent()
        .ok_or_else(|| ImportError("holdings snapshot has no parent directory".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| ImportError("holdings snapshot directory could not be created".into()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = parent.join(format!(
        ".axon-finance-holdings-{}-{nonce}",
        std::process::id()
    ));
    let body = serde_json::to_vec_pretty(&collection)
        .map_err(|_| ImportError("holdings snapshot could not be serialized".into()))?;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| {
                ImportError("holdings snapshot temporary file could not be created".into())
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| {
                    ImportError("holdings snapshot permissions could not be set".into())
                })?;
        }
        file.write_all(&body)
            .and_then(|_| file.sync_all())
            .map_err(|_| ImportError("holdings snapshot could not be written".into()))?;
        std::fs::rename(&temporary, path)
            .map_err(|_| ImportError("holdings snapshot could not be replaced".into()))?;
        Ok(true)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_collection(path: &Path) -> ImportResult<Option<ReviewedHoldingsCollection>> {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ImportError("holdings snapshot could not be read".into())),
    };
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| ImportError("holdings snapshot has an unsupported shape".into()))?;
    if !value
        .get("sources")
        .is_some_and(serde_json::Value::is_object)
    {
        return Err(ImportError(
            "legacy holdings snapshot requires an explicit source migration".into(),
        ));
    }
    let collection: ReviewedHoldingsCollection = serde_json::from_value(value)
        .map_err(|_| ImportError("holdings snapshot has an unsupported shape".into()))?;
    aggregate_collection(&collection)?;
    Ok(Some(collection))
}

fn aggregate_collection(
    collection: &ReviewedHoldingsCollection,
) -> ImportResult<ReviewedHoldingsSnapshot> {
    if collection.schema_version != 2 || collection.sources.is_empty() {
        return Err(ImportError(
            "holdings snapshot has an unsupported shape".into(),
        ));
    }
    struct AggregatePosition {
        quantity: Quantity,
        currency: String,
        latest_unit_price: Option<Quantity>,
        source_count: usize,
    }

    let mut positions: BTreeMap<String, AggregatePosition> = BTreeMap::new();
    let mut sources = Vec::with_capacity(collection.sources.len());
    for (source_key, snapshot) in &collection.sources {
        validate_source_key(source_key)?;
        validate_source_snapshot(snapshot)?;
        sources.push(ReviewedHoldingsSource {
            source_key: source_key.clone(),
            snapshot_id: snapshot.snapshot_id.clone(),
            reviewed_at: snapshot.reviewed_at.clone(),
            coverage: snapshot.coverage,
        });
        for holding in &snapshot.holdings {
            let position = positions
                .entry(holding.instrument.clone())
                .or_insert_with(|| AggregatePosition {
                    quantity: Quantity {
                        mantissa: 0,
                        scale: holding.quantity.scale,
                    },
                    currency: holding.currency.clone(),
                    latest_unit_price: holding.latest_unit_price.clone(),
                    source_count: 0,
                });
            if position.currency != holding.currency {
                return Err(ImportError(
                    "one instrument cannot use multiple currencies across sources".into(),
                ));
            }
            position.quantity = add(&position.quantity, &holding.quantity)?;
            position.source_count += 1;
            if position.source_count > 1 {
                position.latest_unit_price = None;
            }
        }
    }
    let holdings = positions
        .into_iter()
        .filter(|(_, position)| position.quantity.mantissa != 0)
        .map(|(instrument, position)| Holding {
            instrument,
            quantity: position.quantity,
            latest_unit_price: position.latest_unit_price,
            currency: position.currency,
        })
        .collect();
    let reviewed_at = sources
        .iter()
        .map(|source| source.reviewed_at.as_str())
        .max()
        .expect("a nonempty collection has a review date")
        .to_string();
    let coverage = if sources
        .iter()
        .any(|source| source.coverage == HoldingsCoverage::Partial)
    {
        HoldingsCoverage::Partial
    } else {
        HoldingsCoverage::Complete
    };
    Ok(ReviewedHoldingsSnapshot {
        schema_version: 2,
        snapshot_id: collection_identity(&sources),
        reviewed_at,
        coverage,
        holdings,
        sources,
    })
}

fn validate_mapping(mapping: &InvestmentCsvMapping) -> ImportResult<()> {
    if !mapping.delimiter.is_ascii() {
        return Err(ImportError("delimiter must be one ASCII character".into()));
    }
    if !matches!(mapping.decimal_separator, ',' | '.') {
        return Err(ImportError(
            "decimal separator must be comma or period".into(),
        ));
    }
    validate_currency(&mapping.default_currency.to_ascii_uppercase())?;
    match mapping.activity_type_column.as_deref() {
        Some(column) if column.trim().is_empty() => {
            return Err(ImportError("activity type column cannot be empty".into()));
        }
        Some(_) if mapping.position_activity_values.is_empty() => {
            return Err(ImportError(
                "position activity values are required with an activity type column".into(),
            ));
        }
        None if !mapping.position_activity_values.is_empty()
            || !mapping.non_position_activity_values.is_empty() =>
        {
            return Err(ImportError(
                "activity values require an activity type column".into(),
            ));
        }
        _ => {}
    }
    let mut activity_values = HashSet::new();
    for value in mapping
        .position_activity_values
        .iter()
        .chain(&mapping.non_position_activity_values)
    {
        if value.trim().is_empty() || value.trim() != value {
            return Err(ImportError(
                "activity values must be nonempty and trimmed".into(),
            ));
        }
        if !activity_values.insert(value) {
            return Err(ImportError(
                "activity values must be unique across classifications".into(),
            ));
        }
    }
    for alias in mapping.instrument_aliases.values() {
        validate_instrument(alias)?;
    }
    Ok(())
}

fn validate_currency(currency: &str) -> ImportResult<()> {
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ImportError("currency must be a three-letter code".into()))
    }
}

fn validate_instrument(instrument: &str) -> ImportResult<()> {
    let valid = !instrument.is_empty()
        && instrument.len() <= 64
        && instrument
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "instrument must be a symbolic ASCII identifier".into(),
        ))
    }
}

fn parse_quantity(value: &str, decimal_separator: char) -> ImportResult<Quantity> {
    parse_decimal(value, decimal_separator, "quantity")
}

/// `pub(crate)` so `price.rs` parses a provider's decimal with the reader that
/// already handles both decimal marks, grouping separators and a trailing minus,
/// rather than a second parser that would disagree at the edges.
pub(crate) fn parse_decimal(
    value: &str,
    decimal_separator: char,
    kind: &str,
) -> ImportResult<Quantity> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping_separator = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping_separator, "");
    if value.matches(decimal_separator).count() > 1 {
        return Err(ImportError(format!(
            "{kind} has more than one decimal mark"
        )));
    }
    let negative = value.starts_with('-');
    let unsigned = value.trim_start_matches(['-', '+']);
    let mut parts = unsigned.split(decimal_separator);
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 12
    {
        return Err(ImportError(format!("{kind} is not a supported decimal")));
    }
    let digits = format!("{whole}{fraction}");
    let mut mantissa = digits
        .parse::<i64>()
        .map_err(|_| ImportError(format!("{kind} is outside the supported range")))?;
    if negative {
        mantissa = -mantissa;
    }
    Ok(Quantity {
        mantissa,
        scale: fraction.len() as u32,
    })
}

fn add(left: &Quantity, right: &Quantity) -> ImportResult<Quantity> {
    let scale = left.scale.max(right.scale);
    let left_factor = 10_i64
        .checked_pow(scale - left.scale)
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    let right_factor = 10_i64
        .checked_pow(scale - right.scale)
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    let mantissa = left
        .mantissa
        .checked_mul(left_factor)
        .and_then(|value| {
            right
                .mantissa
                .checked_mul(right_factor)
                .and_then(|right| value.checked_add(right))
        })
        .ok_or_else(|| ImportError("quantity is outside the supported range".into()))?;
    Ok(Quantity { mantissa, scale })
}

fn multiply(left: &Quantity, right: &Quantity) -> ImportResult<DecimalValue> {
    let mantissa = i128::from(left.mantissa)
        .checked_mul(i128::from(right.mantissa))
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    let scale = left
        .scale
        .checked_add(right.scale)
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    Ok(normalize_value(DecimalValue { mantissa, scale }))
}

fn add_values(left: &DecimalValue, right: &DecimalValue) -> ImportResult<DecimalValue> {
    let scale = left.scale.max(right.scale);
    let left_factor = 10_i128
        .checked_pow(scale - left.scale)
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    let right_factor = 10_i128
        .checked_pow(scale - right.scale)
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    let mantissa = left
        .mantissa
        .checked_mul(left_factor)
        .and_then(|value| {
            right
                .mantissa
                .checked_mul(right_factor)
                .and_then(|right| value.checked_add(right))
        })
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    Ok(normalize_value(DecimalValue { mantissa, scale }))
}

/// One position's value: quantity times unit price, exactly.
///
/// The public door onto `multiply`, so `portfolio.rs` can value a position
/// against a market price without `multiply` and `add_values` becoming public
/// arithmetic anyone may reach for.
pub fn position_value(quantity: &Quantity, unit_price: &Quantity) -> ImportResult<DecimalValue> {
    multiply(quantity, unit_price)
}

/// Two values added at the wider of their two scales.
pub fn add_decimal_values(left: &DecimalValue, right: &DecimalValue) -> ImportResult<DecimalValue> {
    add_values(left, right)
}

/// A value in minor units (cents), truncated toward zero.
///
/// Truncation rather than rounding, and it is stated rather than assumed: the
/// only readers are the share and drift figures, both of which are reported in
/// basis points over a total that was truncated the same way, so the error is
/// bounded by one cent per position and never accumulates into the comparison.
/// Nothing in the journal is derived from this.
pub fn to_minor_units(value: &DecimalValue) -> ImportResult<i128> {
    if value.scale <= 2 {
        let factor = 10_i128
            .checked_pow(2 - value.scale)
            .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
        return value
            .mantissa
            .checked_mul(factor)
            .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()));
    }
    let factor = 10_i128
        .checked_pow(value.scale - 2)
        .ok_or_else(|| ImportError("portfolio value is outside the supported range".into()))?;
    Ok(value.mantissa / factor)
}

/// A minor-unit amount back as an exact decimal at scale 2.
pub fn from_minor_units(cents: i128) -> DecimalValue {
    normalize_value(DecimalValue {
        mantissa: cents,
        scale: 2,
    })
}

fn normalize_value(mut value: DecimalValue) -> DecimalValue {
    while value.scale > 0 && value.mantissa % 10 == 0 {
        value.mantissa /= 10;
        value.scale -= 1;
    }
    value
}

fn serialize_i64_as_string<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn serialize_i128_as_string<S>(value: &i128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn deserialize_i64_from_string<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(serde::de::Error::custom)
}

fn holdings_identity(holdings: &[Holding]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"axon-finance-holdings-v1");
    for holding in holdings {
        hash.update(holding.instrument.as_bytes());
        hash.update([0xff]);
        hash.update(holding.quantity.mantissa.to_string().as_bytes());
        hash.update([0xff]);
        hash.update(holding.quantity.scale.to_string().as_bytes());
        hash.update([0xff]);
        if let Some(price) = &holding.latest_unit_price {
            hash.update(price.mantissa.to_string().as_bytes());
            hash.update([0xff]);
            hash.update(price.scale.to_string().as_bytes());
        }
        hash.update([0xff]);
        hash.update(holding.currency.as_bytes());
        hash.update([0xfe]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn collection_identity(sources: &[ReviewedHoldingsSource]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"axon-finance-holdings-collection-v2");
    for source in sources {
        hash.update(source.source_key.as_bytes());
        hash.update([0xff]);
        hash.update(source.snapshot_id.as_bytes());
        hash.update([0xff]);
        hash.update(source.reviewed_at.as_bytes());
        hash.update([0xff]);
        hash.update(source.coverage.as_str().as_bytes());
        hash.update([0xfe]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn validate_source_key(source_key: &str) -> ImportResult<()> {
    let valid = !source_key.is_empty()
        && source_key.len() <= 64
        && source_key.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "source key must be lowercase ASCII with digits, hyphens or underscores".into(),
        ))
    }
}

fn validate_source_snapshot(snapshot: &ReviewedHoldingsSnapshot) -> ImportResult<()> {
    if snapshot.schema_version != 1
        || !snapshot.sources.is_empty()
        || normalize_date(&snapshot.reviewed_at)? != snapshot.reviewed_at
    {
        return Err(ImportError(
            "holdings snapshot has an unsupported shape".into(),
        ));
    }
    let mut previous: Option<&str> = None;
    for holding in &snapshot.holdings {
        validate_instrument(&holding.instrument)?;
        validate_currency(&holding.currency)?;
        if holding.quantity.mantissa == 0 || holding.quantity.scale > 12 {
            return Err(ImportError(
                "holdings snapshot has an invalid quantity".into(),
            ));
        }
        if holding
            .latest_unit_price
            .as_ref()
            .is_some_and(|price| price.scale > 12)
        {
            return Err(ImportError("holdings snapshot has an invalid price".into()));
        }
        if previous.is_some_and(|previous| previous >= holding.instrument.as_str()) {
            return Err(ImportError(
                "holdings snapshot instruments are not uniquely sorted".into(),
            ));
        }
        previous = Some(&holding.instrument);
    }
    if snapshot.snapshot_id != holdings_identity(&snapshot.holdings) {
        return Err(ImportError(
            "holdings snapshot failed its integrity check".into(),
        ));
    }
    Ok(())
}

fn column(headers: &StringRecord, name: &str) -> ImportResult<usize> {
    headers
        .iter()
        .position(|header| header.trim() == name)
        .ok_or_else(|| ImportError(format!("configured column {name:?} is absent")))
}

fn optional_column(headers: &StringRecord, name: Option<&str>) -> ImportResult<Option<usize>> {
    name.map(|name| column(headers, name)).transpose()
}

fn default_delimiter() -> char {
    ';'
}

fn default_decimal_separator() -> char {
    ','
}

fn default_currency() -> String {
    "EUR".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> InvestmentCsvMapping {
        InvestmentCsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Date".into(),
            instrument_column: "Instrument".into(),
            quantity_column: "Quantity".into(),
            activity_type_column: None,
            position_activity_values: Vec::new(),
            non_position_activity_values: Vec::new(),
            reference_column: Some("Reference".into()),
            price_column: Some("Price".into()),
            currency_column: Some("Currency".into()),
            default_currency: "EUR".into(),
            instrument_aliases: BTreeMap::new(),
        }
    }

    #[test]
    fn signed_fractional_activity_reconstructs_the_open_holding() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,250;one;10,00;EUR\n2026-02-03;ACME;-0,250;two;12,1234;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 2);
        assert_eq!(preview.holdings.len(), 1);
        assert_eq!(
            preview.holdings[0].quantity,
            Quantity {
                mantissa: 1000,
                scale: 3
            }
        );
        assert_eq!(
            preview.holdings[0].latest_unit_price,
            Some(Quantity {
                mantissa: 121234,
                scale: 4
            })
        );
    }

    #[test]
    fn duplicate_activity_is_counted_once() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,5;same;;EUR\n2026-01-02;ACME;1,5;same;;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 1);
        assert_eq!(preview.duplicate_rows, 1);
        assert_eq!(preview.holdings[0].quantity.mantissa, 15);
    }

    #[test]
    fn distinct_source_references_preserve_identical_trades() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,5;one;;EUR\n2026-01-02;ACME;1,5;two;;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.activity_count, 2);
        assert_eq!(preview.duplicate_rows, 0);
        assert_eq!(preview.holdings[0].quantity.mantissa, 30);
    }

    #[test]
    fn rows_without_quantities_are_reported_but_not_positions() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;;;;EUR\n2026-01-03;ACME;2,0;one;9,00;EUR\n";
        let preview = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(preview.ignored_non_position_rows, 1);
        assert_eq!(preview.activity_count, 1);
    }

    #[test]
    fn classified_non_position_quantities_do_not_inflate_holdings() {
        let mut mapping = mapping();
        mapping.activity_type_column = Some("Type".into());
        mapping.position_activity_values = vec!["BUY".into(), "SELL".into()];
        mapping.non_position_activity_values = vec!["DIVIDEND".into()];
        let csv = b"Date;Instrument;Quantity;Type;Reference;Price;Currency\n2026-01-02;ACME;2,0;BUY;one;10,00;EUR\n2026-02-03;ACME;2,0;DIVIDEND;two;;EUR\n2026-03-04;ACME;-2,0;SELL;three;12,00;EUR\n";

        let preview = preview_csv(csv, &mapping).unwrap();
        assert_eq!(preview.activity_count, 2);
        assert_eq!(preview.ignored_non_position_rows, 1);
        assert_eq!(preview.closed_positions, 1);
        assert!(preview.holdings.is_empty());
    }

    #[test]
    fn nonzero_quantities_with_unknown_activity_types_fail_closed() {
        let mut mapping = mapping();
        mapping.activity_type_column = Some("Type".into());
        mapping.position_activity_values = vec!["BUY".into(), "SELL".into()];
        mapping.non_position_activity_values = vec!["DIVIDEND".into()];
        let csv = b"Date;Instrument;Quantity;Type;Reference;Price;Currency\n2026-01-02;ACME;1,0;NEW_POSITION_EVENT;one;10,00;EUR\n";

        assert_eq!(
            preview_csv(csv, &mapping).unwrap_err().0,
            "a nonzero quantity activity type is not classified"
        );
    }

    #[test]
    fn a_nonzero_quantity_requires_an_instrument() {
        let csv =
            b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;;1,0;one;10,00;EUR\n";
        let error = preview_csv(csv, &mapping()).unwrap_err();
        assert_eq!(error.0, "a nonzero quantity requires an instrument");
    }

    #[test]
    fn aliases_keep_source_identifiers_out_of_the_holding_name() {
        let mut mapping = mapping();
        mapping
            .instrument_aliases
            .insert("source-1".into(), "ACME".into());
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;source-1;1,0;one;10,00;EUR\n";
        let preview = preview_csv(csv, &mapping).unwrap();
        assert_eq!(preview.holdings[0].instrument, "ACME");
    }

    #[test]
    fn snapshot_identity_tracks_the_reviewed_aggregate() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,0;one;10,1234;EUR\n";
        let first = preview_csv(csv, &mapping()).unwrap();
        let second = preview_csv(csv, &mapping()).unwrap();
        assert_eq!(first.snapshot_id, second.snapshot_id);

        let changed = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;2,0;one;10,1234;EUR\n";
        assert_ne!(
            first.snapshot_id,
            preview_csv(changed, &mapping()).unwrap().snapshot_id
        );
    }

    #[test]
    fn reviewed_snapshot_round_trips_without_source_rows() {
        let mut mapping = mapping();
        mapping
            .instrument_aliases
            .insert("private-source-id".into(), "ACME".into());
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;private-source-id;1,0;private-reference;10,1234;EUR\n";
        let preview = preview_csv(csv, &mapping).unwrap();
        let snapshot =
            reviewed_snapshot(&preview, &mapping, "2026-08-09", HoldingsCoverage::Complete)
                .unwrap();
        let directory =
            std::env::temp_dir().join(format!("axon-finance-holdings-test-{}", std::process::id()));
        let path = directory.join("holdings.json");
        let _ = std::fs::remove_dir_all(&directory);

        assert!(write_reviewed_snapshot(&path, "synthetic-broker", &snapshot).unwrap());
        let reconfirmed = ReviewedHoldingsSnapshot {
            reviewed_at: "2026-08-10".into(),
            ..snapshot.clone()
        };
        assert!(write_reviewed_snapshot(&path, "synthetic-broker", &reconfirmed).unwrap());
        assert!(!write_reviewed_snapshot(&path, "synthetic-broker", &reconfirmed).unwrap());
        let partial = ReviewedHoldingsSnapshot {
            coverage: HoldingsCoverage::Partial,
            ..reconfirmed.clone()
        };
        assert!(write_reviewed_snapshot(&path, "synthetic-broker", &partial).unwrap());
        assert!(!write_reviewed_snapshot(&path, "synthetic-broker", &partial).unwrap());
        let canonical = read_reviewed_snapshot(&path).unwrap().unwrap();
        assert_eq!(canonical.schema_version, 2);
        assert_eq!(canonical.holdings, snapshot.holdings);
        assert_eq!(canonical.sources.len(), 1);
        assert_eq!(canonical.sources[0].source_key, "synthetic-broker");
        assert_eq!(canonical.sources[0].reviewed_at, "2026-08-10");
        assert_eq!(canonical.coverage, HoldingsCoverage::Partial);
        assert_eq!(canonical.sources[0].coverage, HoldingsCoverage::Partial);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("private-source-id"));
        assert!(!body.contains("private-reference"));
        assert!(body.contains("\"mantissa\": \"10\""));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_updates_preserve_other_sources_and_aggregate_quantities() {
        let directory = std::env::temp_dir().join(format!(
            "axon-finance-holdings-sources-test-{}",
            std::process::id()
        ));
        let path = directory.join("holdings.json");
        let _ = std::fs::remove_dir_all(&directory);
        let source = |quantity, reviewed_at: &str, coverage| {
            let holdings = vec![Holding {
                instrument: "ACME".into(),
                quantity: Quantity {
                    mantissa: quantity,
                    scale: 0,
                },
                latest_unit_price: Some(Quantity {
                    mantissa: 1000,
                    scale: 2,
                }),
                currency: "EUR".into(),
            }];
            ReviewedHoldingsSnapshot {
                schema_version: 1,
                snapshot_id: holdings_identity(&holdings),
                reviewed_at: reviewed_at.into(),
                coverage,
                holdings,
                sources: Vec::new(),
            }
        };

        assert!(write_reviewed_snapshot(
            &path,
            "broker-one",
            &source(2, "2026-08-08", HoldingsCoverage::Complete),
        )
        .unwrap());
        assert!(write_reviewed_snapshot(
            &path,
            "broker-two",
            &source(3, "2026-08-09", HoldingsCoverage::Partial),
        )
        .unwrap());
        let aggregate = read_reviewed_snapshot(&path).unwrap().unwrap();
        assert_eq!(aggregate.sources.len(), 2);
        assert_eq!(aggregate.coverage, HoldingsCoverage::Partial);
        assert_eq!(aggregate.sources[1].coverage, HoldingsCoverage::Partial);
        assert_eq!(aggregate.holdings[0].quantity.mantissa, 5);
        assert_eq!(aggregate.holdings[0].latest_unit_price, None);
        assert_eq!(aggregate.reviewed_at, "2026-08-09");

        assert!(write_reviewed_snapshot(
            &path,
            "broker-one",
            &source(4, "2026-08-10", HoldingsCoverage::Complete),
        )
        .unwrap());
        let aggregate = read_reviewed_snapshot(&path).unwrap().unwrap();
        assert_eq!(aggregate.sources.len(), 2);
        assert_eq!(aggregate.holdings[0].quantity.mantissa, 7);
        assert_eq!(aggregate.reviewed_at, "2026-08-10");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_snapshots_remain_readable_but_require_named_migration_before_writes() {
        let directory = std::env::temp_dir().join(format!(
            "axon-finance-holdings-legacy-test-{}",
            std::process::id()
        ));
        let path = directory.join("holdings.json");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let holdings = vec![Holding {
            instrument: "ACME".into(),
            quantity: Quantity {
                mantissa: 1,
                scale: 0,
            },
            latest_unit_price: None,
            currency: "EUR".into(),
        }];
        let snapshot = ReviewedHoldingsSnapshot {
            schema_version: 1,
            snapshot_id: holdings_identity(&holdings),
            reviewed_at: "2026-08-09".into(),
            coverage: HoldingsCoverage::Complete,
            holdings,
            sources: Vec::new(),
        };
        let mut legacy = serde_json::to_value(&snapshot).unwrap();
        legacy.as_object_mut().unwrap().remove("coverage");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        assert_eq!(
            read_reviewed_snapshot(&path).unwrap(),
            Some(snapshot.clone())
        );
        assert_eq!(
            write_reviewed_snapshot(&path, "synthetic-broker", &snapshot)
                .unwrap_err()
                .0,
            "legacy holdings snapshot requires an explicit source migration"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn confirmation_requires_an_explicit_alias_for_every_holding() {
        let csv = b"Date;Instrument;Quantity;Reference;Price;Currency\n2026-01-02;ACME;1,0;one;10,00;EUR\n";
        let mapping = mapping();
        let preview = preview_csv(csv, &mapping).unwrap();
        assert_eq!(
            reviewed_snapshot(&preview, &mapping, "2026-08-09", HoldingsCoverage::Complete)
                .unwrap_err()
                .0,
            "every confirmed instrument requires an explicit private alias"
        );
    }

    #[test]
    fn portfolio_valuation_is_exact_and_grouped_by_currency() {
        let snapshot = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-08-09".into(),
            coverage: HoldingsCoverage::Partial,
            holdings: vec![
                Holding {
                    instrument: "ACME".into(),
                    quantity: Quantity {
                        mantissa: 125,
                        scale: 2,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: 1234,
                        scale: 2,
                    }),
                    currency: "EUR".into(),
                },
                Holding {
                    instrument: "EXAMPLE".into(),
                    quantity: Quantity {
                        mantissa: 2,
                        scale: 0,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: 500,
                        scale: 2,
                    }),
                    currency: "EUR".into(),
                },
                Holding {
                    instrument: "SAMPLE".into(),
                    quantity: Quantity {
                        mantissa: 3,
                        scale: 0,
                    },
                    latest_unit_price: None,
                    currency: "USD".into(),
                },
            ],
            sources: Vec::new(),
        };

        assert_eq!(
            portfolio_valuations(&snapshot).unwrap(),
            vec![
                PortfolioValuation {
                    currency: "EUR".into(),
                    value: DecimalValue {
                        mantissa: 25425,
                        scale: 3,
                    },
                    priced_holdings: 2,
                    unpriced_holdings: 0,
                },
                PortfolioValuation {
                    currency: "USD".into(),
                    value: DecimalValue {
                        mantissa: 0,
                        scale: 0,
                    },
                    priced_holdings: 0,
                    unpriced_holdings: 1,
                },
            ]
        );
    }

    #[test]
    fn portfolio_valuation_rejects_decimal_overflow() {
        let snapshot = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-08-09".into(),
            coverage: HoldingsCoverage::Complete,
            holdings: (0..3)
                .map(|index| Holding {
                    instrument: format!("ACME-{index}"),
                    quantity: Quantity {
                        mantissa: i64::MAX,
                        scale: 0,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: i64::MAX,
                        scale: 0,
                    }),
                    currency: "EUR".into(),
                })
                .collect(),
            sources: Vec::new(),
        };

        assert_eq!(
            portfolio_valuations(&snapshot).unwrap_err().0,
            "portfolio value is outside the supported range"
        );
    }
}

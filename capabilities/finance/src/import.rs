//! Bank-export staging and reviewed journal writes.
//!
//! CSV is an edge format. It is parsed into candidates, immediately discarded,
//! and never becomes canonical. Only an explicitly confirmed candidate is rendered
//! into the plaintext journal; rejection writes nothing.

use candidate_fingerprint::CandidateKey;
use csv::StringRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub type ImportResult<T> = Result<T, ImportError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvMapping {
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    #[serde(default = "default_decimal_separator")]
    pub decimal_separator: char,
    pub date_column: String,
    pub amount_column: String,
    pub description_column: String,
    #[serde(default)]
    pub categorization_columns: Vec<String>,
    #[serde(default)]
    pub reference_column: Option<String>,
    #[serde(default)]
    pub currency_column: Option<String>,
    #[serde(default = "default_currency")]
    pub default_currency: String,
    pub source_account: String,
    #[serde(default = "default_outflow_account")]
    pub default_outflow_account: String,
    #[serde(default = "default_inflow_account")]
    pub default_inflow_account: String,
    #[serde(default)]
    pub categorization_rules: Vec<CsvCategorizationRule>,
    #[serde(default)]
    pub row_filter: Option<CsvRowFilter>,
    #[serde(default)]
    pub amount_sign: AmountSign,
    #[serde(default)]
    pub amount_rounding: AmountRounding,
    #[serde(default = "default_date_formats")]
    pub date_formats: Vec<CsvDateFormat>,
    #[serde(default)]
    pub row_policy: CsvRowPolicy,
    #[serde(default)]
    pub location_columns: Option<CsvLocationColumns>,
}

/// Raw location columns preserved from the export for the places capability,
/// which links spend to venues through these fields
/// (`capabilities/places/README.md`, D1: forward imports must stop discarding
/// them). Every column is optional because exports differ: the Amex shape keeps
/// the city inside the street column's second line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvLocationColumns {
    #[serde(default)]
    pub street_column: Option<String>,
    #[serde(default)]
    pub postal_code_column: Option<String>,
    #[serde(default)]
    pub city_column: Option<String>,
    #[serde(default)]
    pub country_column: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvCategorizationRule {
    #[serde(default)]
    pub description_contains_any: Vec<String>,
    #[serde(default)]
    pub description_starts_with_any: Vec<String>,
    #[serde(default)]
    pub field_equals_any: Vec<CsvFieldEquals>,
    #[serde(default)]
    pub direction: CsvRuleDirection,
    pub account: String,
    pub confidence_basis_points: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvFieldEquals {
    pub column: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvRowFilter {
    pub column: String,
    pub include_values: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvRuleDirection {
    #[default]
    Any,
    Outflow,
    Inflow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountSign {
    #[default]
    AsProvided,
    Invert,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountRounding {
    #[default]
    Reject,
    HalfAwayFromZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDateFormat {
    IsoYearMonthDay,
    DayMonthYearDots,
    DayMonthYearSlashes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvRowPolicy {
    #[default]
    Strict,
    RequiredFields,
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

fn default_outflow_account() -> String {
    "expenses:uncategorized".into()
}

fn default_inflow_account() -> String {
    "income:uncategorized".into()
}

fn default_date_formats() -> Vec<CsvDateFormat> {
    vec![
        CsvDateFormat::IsoYearMonthDay,
        CsvDateFormat::DayMonthYearDots,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Pending,
    Confirmed,
    Rejected,
    Duplicate,
}

impl CandidateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::Duplicate => "duplicate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            "duplicate" => Some(Self::Duplicate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionCandidate {
    pub id: String,
    pub fingerprint: String,
    pub booked_at: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    pub source_account: String,
    pub source_reference: Option<String>,
    pub proposed_account: String,
    pub confidence_basis_points: u16,
    pub state: CandidateState,
    /// Stored verbatim from the export, embedded newlines included, so the
    /// places capability can geocode the exact text the bank printed. Never
    /// part of the fingerprint: venue links key on fingerprints minted before
    /// these fields existed (`capabilities/places/README.md`, D2).
    #[serde(default)]
    pub location_street: Option<String>,
    #[serde(default)]
    pub location_postal_code: Option<String>,
    #[serde(default)]
    pub location_city: Option<String>,
    #[serde(default)]
    pub location_country: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CsvImportPreview {
    pub preview_id: String,
    pub candidate_count: usize,
    pub duplicate_rows: usize,
    pub preserved_repetitions: usize,
    pub ignored_non_transaction_rows: usize,
    pub outflow_count: usize,
    pub inflow_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCsv {
    pub preview: CsvImportPreview,
    pub candidates: Vec<TransactionCandidate>,
}

pub fn parse_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<Vec<TransactionCandidate>> {
    Ok(prepare_csv(bytes, mapping)?.candidates)
}

pub fn preview_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<CsvImportPreview> {
    Ok(prepare_csv(bytes, mapping)?.preview)
}

pub fn prepare_csv(bytes: &[u8], mapping: &CsvMapping) -> ImportResult<PreparedCsv> {
    validate_mapping(mapping)?;
    if !mapping.delimiter.is_ascii() {
        return Err(ImportError("delimiter must be one ASCII character".into()));
    }
    if !matches!(mapping.decimal_separator, ',' | '.') {
        return Err(ImportError(
            "decimal separator must be comma or period".into(),
        ));
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(mapping.delimiter as u8)
        .flexible(mapping.row_policy == CsvRowPolicy::RequiredFields)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| ImportError("CSV header could not be read".into()))?
        .clone();
    let date = column(&headers, &mapping.date_column)?;
    let amount = column(&headers, &mapping.amount_column)?;
    let description = column(&headers, &mapping.description_column)?;
    let categorization_columns = mapping
        .categorization_columns
        .iter()
        .map(|name| column(&headers, name))
        .collect::<ImportResult<Vec<_>>>()?;
    let reference = optional_column(&headers, mapping.reference_column.as_deref())?;
    let currency = optional_column(&headers, mapping.currency_column.as_deref())?;
    let location = mapping.location_columns.as_ref();
    let street = optional_column(&headers, location.and_then(|l| l.street_column.as_deref()))?;
    let postal_code = optional_column(
        &headers,
        location.and_then(|l| l.postal_code_column.as_deref()),
    )?;
    let city = optional_column(&headers, location.and_then(|l| l.city_column.as_deref()))?;
    let country = optional_column(&headers, location.and_then(|l| l.country_column.as_deref()))?;
    let row_filter = mapping
        .row_filter
        .as_ref()
        .map(|filter| {
            Ok::<_, ImportError>((
                column(&headers, &filter.column)?,
                filter
                    .include_values
                    .iter()
                    .map(|value| value.trim().to_ascii_lowercase())
                    .collect::<HashSet<_>>(),
            ))
        })
        .transpose()?;

    let mut candidates = Vec::new();
    let mut stable_fingerprints = HashSet::new();
    let mut occurrence_counts = HashMap::new();
    let mut duplicate_rows = 0;
    let mut preserved_repetitions = 0;
    let mut ignored_non_transaction_rows = 0;
    for (index, record) in reader.records().enumerate() {
        let row = index + 2;
        let record =
            record.map_err(|_| ImportError(format!("CSV row {row} does not match its header")))?;
        let date_value = record.get(date).unwrap_or_default();
        let amount_value = record.get(amount).unwrap_or_default();
        let description_value = record.get(description).unwrap_or_default();
        if row_filter.as_ref().is_some_and(|(column, include_values)| {
            !include_values.contains(
                &normalize_text(record.get(*column).unwrap_or_default()).to_ascii_lowercase(),
            )
        }) {
            ignored_non_transaction_rows += 1;
            continue;
        }
        if mapping.row_policy == CsvRowPolicy::RequiredFields
            && amount_value.trim().is_empty()
            && normalize_date_with_formats(date_value, &mapping.date_formats).is_err()
        {
            ignored_non_transaction_rows += 1;
            continue;
        }
        let booked_at = normalize_date_with_formats(date_value, &mapping.date_formats)
            .map_err(|error| row_error(row, error))?;
        let mut amount_cents = parse_decimal_cents_with_rounding(
            amount_value,
            mapping.decimal_separator,
            mapping.amount_rounding,
        )
        .map_err(|error| row_error(row, error))?;
        if mapping.amount_sign == AmountSign::Invert {
            amount_cents = amount_cents.checked_neg().ok_or_else(|| {
                ImportError(format!(
                    "CSV row {row}: amount is outside the supported range"
                ))
            })?;
        }
        let description = normalize_text(description_value);
        if description.is_empty() {
            return Err(ImportError(format!(
                "CSV row {row}: description must not be blank"
            )));
        }
        let categorization_text =
            categorization_columns
                .iter()
                .fold(description.clone(), |mut combined, index| {
                    let value = normalize_text(record.get(*index).unwrap_or_default());
                    if !value.is_empty() {
                        combined.push(' ');
                        combined.push_str(&value);
                    }
                    combined
                });
        let source_reference = reference
            .and_then(|index| record.get(index))
            .map(normalize_text)
            .filter(|value| !value.is_empty());
        let currency = currency
            .and_then(|index| record.get(index))
            .map(normalize_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| mapping.default_currency.clone())
            .to_ascii_uppercase();
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ImportError(format!(
                "CSV row {row}: currency must be a three-letter code"
            )));
        }
        let base_fingerprint = CandidateKey {
            booked_at: &booked_at,
            amount_cents,
            currency: &currency,
            description: &description,
            source_reference: source_reference.as_deref(),
            source_account: &mapping.source_account,
        }
        .fingerprint();
        let fingerprint = if source_reference.is_some() {
            if !stable_fingerprints.insert(base_fingerprint.clone()) {
                duplicate_rows += 1;
                continue;
            }
            base_fingerprint
        } else {
            let occurrence = occurrence_counts
                .entry(base_fingerprint.clone())
                .or_insert(0_usize);
            if *occurrence > 0 {
                preserved_repetitions += 1;
            }
            let fingerprint = candidate_fingerprint::repeated(&base_fingerprint, *occurrence);
            *occurrence += 1;
            fingerprint
        };
        let (proposed_account, confidence_basis_points) = categorize(
            mapping,
            &categorization_text,
            amount_cents,
            &headers,
            &record,
        )?;
        // Verbatim by contract: no normalization, so the Amex two-line street
        // value keeps its embedded newline. Blank cells become absent.
        let location_value = |index: Option<usize>| {
            index
                .and_then(|index| record.get(index))
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        };
        candidates.push(TransactionCandidate {
            id: format!("candidate_{fingerprint}"),
            fingerprint,
            booked_at,
            description,
            amount_cents,
            currency,
            source_account: mapping.source_account.clone(),
            source_reference,
            proposed_account,
            confidence_basis_points,
            state: CandidateState::Pending,
            location_street: location_value(street),
            location_postal_code: location_value(postal_code),
            location_city: location_value(city),
            location_country: location_value(country),
        });
    }
    let outflow_count = candidates
        .iter()
        .filter(|candidate| candidate.amount_cents < 0)
        .count();
    let inflow_count = candidates
        .iter()
        .filter(|candidate| candidate.amount_cents > 0)
        .count();
    let preview = CsvImportPreview {
        preview_id: preview_id(
            &candidates,
            duplicate_rows,
            preserved_repetitions,
            ignored_non_transaction_rows,
        ),
        candidate_count: candidates.len(),
        duplicate_rows,
        preserved_repetitions,
        ignored_non_transaction_rows,
        outflow_count,
        inflow_count,
    };
    Ok(PreparedCsv {
        preview,
        candidates,
    })
}

pub const MAIL_SOURCE_ACCOUNT_REQUIRED: &str = "review:source-account-required";

/// Build one idempotent candidate from a paid-receipt mail extraction.
pub fn mail_candidate(
    source_id: &str,
    booked_at: &str,
    description: &str,
    amount: &str,
    decimal_separator: char,
    currency: &str,
    source_account: &str,
) -> ImportResult<TransactionCandidate> {
    if source_id.is_empty()
        || source_id.len() > 128
        || !source_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ImportError("invalid Gmail source id".into()));
    }
    let booked_at = normalize_date(booked_at)?;
    let description = sanitize_description(description);
    if description.is_empty() || description.chars().count() > 200 {
        return Err(ImportError(
            "description must contain 1 to 200 characters".into(),
        ));
    }
    if description.chars().any(char::is_control) {
        return Err(ImportError(
            "description contains a control character".into(),
        ));
    }
    if amount.len() > 32 {
        return Err(ImportError("amount is too long".into()));
    }
    if amount.contains(['.', ',']) && decimal_separator != mail_decimal_separator(amount) {
        return Err(ImportError(
            "amount decimal separator does not match the extracted amount".into(),
        ));
    }
    let cents = parse_decimal_cents(amount, decimal_separator)?;
    if cents <= 0 {
        return Err(ImportError("paid receipt amount must be positive".into()));
    }
    let currency = currency.trim().to_ascii_uppercase();
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ImportError("currency must be a three-letter code".into()));
    }
    if source_account != MAIL_SOURCE_ACCOUNT_REQUIRED {
        validate_account(source_account)?;
        if !is_balance_account(source_account) {
            return Err(ImportError(
                "source account must be an asset or liability account".into(),
            ));
        }
    }
    let digest = Sha256::digest(format!("gmail-thread:{source_id}").as_bytes());
    let fingerprint: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(TransactionCandidate {
        id: format!("candidate_{fingerprint}"),
        fingerprint,
        booked_at,
        description,
        amount_cents: -cents,
        currency,
        source_account: source_account.into(),
        source_reference: Some(format!("gmail:{source_id}")),
        proposed_account: "expenses:uncategorized".into(),
        confidence_basis_points: 0,
        state: CandidateState::Pending,
        location_street: None,
        location_postal_code: None,
        location_city: None,
        location_country: None,
    })
}

pub fn render_journal_entry(
    candidate: &TransactionCandidate,
    account: &str,
) -> ImportResult<String> {
    validate_account(&candidate.source_account)?;
    if candidate
        .source_reference
        .as_deref()
        .is_some_and(|reference| reference.starts_with("gmail:"))
        && (candidate.source_account == MAIL_SOURCE_ACCOUNT_REQUIRED
            || !is_balance_account(&candidate.source_account))
    {
        return Err(ImportError(
            "mail candidates require a reviewed asset or liability source account".into(),
        ));
    }
    validate_account(account)?;
    if matches!(
        candidate.state,
        CandidateState::Rejected | CandidateState::Duplicate
    ) {
        return Err(ImportError(
            "a rejected or duplicate candidate cannot be confirmed".into(),
        ));
    }
    if candidate.amount_cents == 0 {
        return Err(ImportError(
            "zero-value candidates are not journal entries".into(),
        ));
    }
    let balance_transfer = is_balance_account(&candidate.source_account)
        && is_balance_account(account)
        && candidate.source_account != account;
    if candidate.amount_cents < 0 && !account.starts_with("expenses:") && !balance_transfer {
        return Err(ImportError(
            "outflows must be reviewed into an expenses account or a different balance account"
                .into(),
        ));
    }
    if candidate.amount_cents > 0 && !account.starts_with("income:") && !balance_transfer {
        return Err(ImportError(
            "inflows must be reviewed into an income account or a different balance account".into(),
        ));
    }
    let description = sanitize_description(&candidate.description);
    Ok(format!(
        "\n{} * {}\n    {}  {} {}\n    {}\n    ; source-id: {}\n",
        candidate.booked_at,
        description,
        candidate.source_account,
        decimal(candidate.amount_cents),
        candidate.currency,
        account,
        candidate.fingerprint,
    ))
}

pub fn transfer_match_ids(
    candidates: &[TransactionCandidate],
    candidate: &TransactionCandidate,
) -> Vec<String> {
    if candidate.state != CandidateState::Pending {
        return Vec::new();
    }
    let mut matches: Vec<_> = candidates
        .iter()
        .filter(|other| other.id != candidate.id)
        .filter(|other| {
            matches!(
                other.state,
                CandidateState::Pending | CandidateState::Confirmed
            )
        })
        .filter(|other| is_transfer_pair(candidate, other))
        .map(|other| other.id.clone())
        .collect();
    matches.sort();
    matches
}

pub fn is_transfer_pair(left: &TransactionCandidate, right: &TransactionCandidate) -> bool {
    left.currency == right.currency
        && left.source_account != right.source_account
        && left.proposed_account == right.source_account
        && right.proposed_account == left.source_account
        && left.amount_cents.checked_neg() == Some(right.amount_cents)
        && match (iso_day(&left.booked_at), iso_day(&right.booked_at)) {
            (Some(left), Some(right)) => left.abs_diff(right) <= 3,
            _ => false,
        }
}

fn iso_day(value: &str) -> Option<i64> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
    {
        return None;
    }
    let mut year = value.get(..4)?.parse::<i64>().ok()?;
    let month = value.get(5..7)?.parse::<i64>().ok()?;
    let day = value.get(8..10)?.parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    year -= i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Append at most once. A crash after the file write but before the database state
/// update is recoverable: the retry sees the marker and does not duplicate money.
pub fn append_confirmed(
    journal: &Path,
    candidate: &TransactionCandidate,
    account: &str,
) -> ImportResult<bool> {
    let existing = std::fs::read_to_string(journal)
        .map_err(|error| ImportError(format!("journal could not be read: {error}")))?;
    let marker = format!("source-id: {}", candidate.fingerprint);
    if existing.contains(&marker) {
        return Ok(false);
    }
    let entry = render_journal_entry(candidate, account)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .map_err(|error| ImportError(format!("journal could not be opened: {error}")))?;
    file.write_all(entry.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| ImportError(format!("journal could not be updated: {error}")))?;
    Ok(true)
}

/// Rewrite the reviewed account for one confirmed source entry without changing
/// its date, amount, description, or stable source marker. The caller validates
/// and atomically replaces the complete journal returned here.
pub fn rewrite_confirmed_account(
    journal: &str,
    candidate: &TransactionCandidate,
    account: &str,
) -> ImportResult<(String, bool)> {
    if candidate.state != CandidateState::Confirmed {
        return Err(ImportError(
            "only a confirmed candidate can be reclassified".into(),
        ));
    }
    let current = render_journal_entry(candidate, &candidate.proposed_account)?;
    let replacement = render_journal_entry(candidate, account)?;
    if current == replacement {
        return Ok((journal.to_string(), false));
    }
    let marker = format!("source-id: {}", candidate.fingerprint);
    if journal.matches(&marker).count() != 1 {
        return Err(ImportError(
            "confirmed source marker must occur exactly once in the journal".into(),
        ));
    }
    if journal.matches(&replacement).count() == 1 {
        return Ok((journal.to_string(), false));
    }
    if journal.matches(&current).count() != 1 {
        return Err(ImportError(
            "confirmed journal entry no longer matches its reviewed candidate".into(),
        ));
    }
    Ok((journal.replacen(&current, &replacement, 1), true))
}

fn validate_mapping(mapping: &CsvMapping) -> ImportResult<()> {
    validate_account(&mapping.source_account)?;
    validate_account(&mapping.default_outflow_account)?;
    validate_account(&mapping.default_inflow_account)?;
    if let Some(filter) = &mapping.row_filter {
        if filter.column.trim().is_empty()
            || filter.include_values.is_empty()
            || filter
                .include_values
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(ImportError(
                "row filter requires a column and at least one non-blank included value".into(),
            ));
        }
    }
    for rule in &mapping.categorization_rules {
        validate_account(&rule.account)?;
        if (rule.description_contains_any.is_empty()
            && rule.description_starts_with_any.is_empty()
            && rule.field_equals_any.is_empty())
            || rule
                .description_contains_any
                .iter()
                .chain(&rule.description_starts_with_any)
                .any(|value| value.trim().is_empty())
            || rule.field_equals_any.iter().any(|matcher| {
                matcher.column.trim().is_empty()
                    || matcher.values.is_empty()
                    || matcher.values.iter().any(|value| value.trim().is_empty())
            })
        {
            return Err(ImportError(
                "categorization rules require at least one non-blank matcher".into(),
            ));
        }
        if rule.confidence_basis_points > 10_000 {
            return Err(ImportError(
                "categorization confidence must not exceed 10000 basis points".into(),
            ));
        }
    }
    if mapping.default_currency.len() != 3 {
        return Err(ImportError(
            "default currency must be a three-letter code".into(),
        ));
    }
    if mapping.date_formats.is_empty() {
        return Err(ImportError(
            "at least one explicit date format is required".into(),
        ));
    }
    Ok(())
}

fn categorize(
    mapping: &CsvMapping,
    description: &str,
    amount_cents: i64,
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
) -> ImportResult<(String, u16)> {
    let normalized_description = description.to_lowercase();
    for rule in &mapping.categorization_rules {
        let direction_matches = match rule.direction {
            CsvRuleDirection::Any => true,
            CsvRuleDirection::Outflow => amount_cents < 0,
            CsvRuleDirection::Inflow => amount_cents > 0,
        };
        let contains = rule
            .description_contains_any
            .iter()
            .any(|fragment| normalized_description.contains(&fragment.trim().to_lowercase()));
        let starts_with = rule
            .description_starts_with_any
            .iter()
            .any(|fragment| normalized_description.starts_with(&fragment.trim().to_lowercase()));
        let mut field_equals = false;
        for matcher in &rule.field_equals_any {
            let index = column(headers, &matcher.column)?;
            let value = normalize_text(record.get(index).unwrap_or_default()).to_ascii_lowercase();
            field_equals |= matcher
                .values
                .iter()
                .any(|expected| value == expected.trim().to_ascii_lowercase());
        }
        if direction_matches && (contains || starts_with || field_equals) {
            return Ok((rule.account.clone(), rule.confidence_basis_points));
        }
    }
    let account = if amount_cents < 0 {
        &mapping.default_outflow_account
    } else {
        &mapping.default_inflow_account
    };
    Ok((account.clone(), 0))
}

fn is_balance_account(account: &str) -> bool {
    account.starts_with("assets:") || account.starts_with("liabilities:")
}

pub fn validate_account(account: &str) -> ImportResult<()> {
    let valid = !account.is_empty()
        && account.split(':').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "account must be a symbolic colon-separated name".into(),
        ))
    }
}

fn column(headers: &StringRecord, name: &str) -> ImportResult<usize> {
    headers
        .iter()
        .position(|header| header.trim().trim_start_matches('\u{feff}') == name)
        .ok_or_else(|| ImportError(format!("configured column {name:?} is absent")))
}

fn optional_column(headers: &StringRecord, name: Option<&str>) -> ImportResult<Option<usize>> {
    name.map(|name| column(headers, name)).transpose()
}

pub(crate) fn normalize_date(value: &str) -> ImportResult<String> {
    normalize_date_with_formats(value, &default_date_formats())
}

fn normalize_date_with_formats(value: &str, formats: &[CsvDateFormat]) -> ImportResult<String> {
    let value = value.trim();
    for format in formats {
        let parts = match format {
            CsvDateFormat::IsoYearMonthDay => date_parts(value, b'-', true),
            CsvDateFormat::DayMonthYearDots => date_parts(value, b'.', false),
            CsvDateFormat::DayMonthYearSlashes => date_parts(value, b'/', false),
        };
        if let Some((year, month, day)) = parts.filter(|parts| valid_date(*parts)) {
            return Ok(format!("{year:04}-{month:02}-{day:02}"));
        }
    }
    Err(ImportError(
        "date does not match the configured formats or is not a calendar date".into(),
    ))
}

fn date_parts(value: &str, separator: u8, year_first: bool) -> Option<(u32, u32, u32)> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[2 + usize::from(year_first) * 2] != separator
        || bytes[5 + usize::from(year_first) * 2] != separator
    {
        return None;
    }
    let digits = |start: usize, end: usize| value.get(start..end)?.parse::<u32>().ok();
    if year_first {
        Some((digits(0, 4)?, digits(5, 7)?, digits(8, 10)?))
    } else {
        Some((digits(6, 10)?, digits(3, 5)?, digits(0, 2)?))
    }
}

fn valid_date((year, month, day): (u32, u32, u32)) -> bool {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day > 0 && day <= days
}

fn row_error(row: usize, error: ImportError) -> ImportError {
    ImportError(format!("CSV row {row}: {}", error.0))
}

pub fn parse_decimal_cents(value: &str, decimal_separator: char) -> ImportResult<i64> {
    parse_decimal_cents_with_rounding(value, decimal_separator, AmountRounding::Reject)
}

fn mail_decimal_separator(amount: &str) -> char {
    let marks: Vec<_> = amount
        .char_indices()
        .filter(|(_, character)| matches!(character, '.' | ','))
        .collect();
    let Some((index, mark)) = marks.last().copied() else {
        return '.';
    };
    let other = if mark == ',' { '.' } else { ',' };
    let trailing_digits = amount[index + mark.len_utf8()..]
        .chars()
        .filter(char::is_ascii_digit)
        .count();
    if marks.iter().all(|(_, character)| *character == mark) && trailing_digits == 3 {
        other
    } else {
        mark
    }
}

fn parse_decimal_cents_with_rounding(
    value: &str,
    decimal_separator: char,
    rounding: AmountRounding,
) -> ImportResult<i64> {
    let mut value = value.trim().replace(['\u{a0}', ' '], "");
    if value.ends_with('-') {
        value.pop();
        value.insert(0, '-');
    }
    let grouping_separator = if decimal_separator == ',' { '.' } else { ',' };
    value = value.replace(grouping_separator, "");
    if value.matches(decimal_separator).count() > 1 {
        return Err(ImportError("amount has more than one decimal mark".into()));
    }
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value.as_str());
    if unsigned.is_empty()
        || !unsigned
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == decimal_separator as u8)
    {
        return Err(ImportError("amount is not a decimal number".into()));
    }
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value.as_str());
    let (whole, fraction) = unsigned
        .split_once(decimal_separator)
        .unwrap_or((unsigned, ""));
    if fraction.len() > 2 && rounding == AmountRounding::Reject {
        return Err(ImportError(
            "amount has more than two decimal places".into(),
        ));
    }
    let whole = if whole.is_empty() {
        0
    } else {
        whole
            .parse::<i64>()
            .map_err(|_| ImportError("amount is outside the supported range".into()))?
    };
    let fraction_cents = fraction
        .bytes()
        .take(2)
        .fold(0_i64, |value, digit| value * 10 + i64::from(digit - b'0'))
        * if fraction.len() == 1 { 10 } else { 1 };
    let mut cents = whole
        .checked_mul(100)
        .and_then(|value| value.checked_add(fraction_cents))
        .ok_or_else(|| ImportError("amount is outside the supported range".into()))?;
    if rounding == AmountRounding::HalfAwayFromZero
        && fraction
            .as_bytes()
            .get(2)
            .is_some_and(|digit| *digit >= b'5')
    {
        cents = cents
            .checked_add(1)
            .ok_or_else(|| ImportError("amount is outside the supported range".into()))?;
    }
    if negative {
        cents
            .checked_neg()
            .ok_or_else(|| ImportError("amount is outside the supported range".into()))
    } else {
        Ok(cents)
    }
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn sanitize_description(value: &str) -> String {
    let sanitized = normalize_text(value).replace([';', '\n', '\r'], " ");
    // A description written directly after the status mark is read as a
    // transaction CODE when it opens with a parenthesis -- by hledger, and so
    // by `journal.rs`, which keeps parity with it. The bank text would come
    // back missing its first word, so the ambiguity is removed at the one place
    // that writes the line. Measured 2026-08-28: 0 of the 1,339 live
    // descriptions open with `(`, so this changes no existing entry, and it
    // never touches the fingerprint, which hashes the unsanitized text.
    sanitized.trim_start_matches('(').trim_start().to_string()
}

pub(crate) fn decimal(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let absolute = cents.unsigned_abs();
    format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
}

fn preview_id(
    candidates: &[TransactionCandidate],
    duplicate_rows: usize,
    preserved_repetitions: usize,
    ignored_non_transaction_rows: usize,
) -> String {
    let mut fingerprints: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.fingerprint.as_str())
        .collect();
    fingerprints.sort_unstable();
    let duplicate_rows = duplicate_rows.to_string();
    let preserved_repetitions = preserved_repetitions.to_string();
    let ignored_non_transaction_rows = ignored_non_transaction_rows.to_string();
    fingerprints.push(&duplicate_rows);
    fingerprints.push(&preserved_repetitions);
    fingerprints.push(&ignored_non_transaction_rows);
    candidate_fingerprint::digest(&fingerprints)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> CsvMapping {
        CsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Buchung".into(),
            amount_column: "Betrag".into(),
            description_column: "Text".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Referenz".into()),
            currency_column: Some("Waehrung".into()),
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            default_outflow_account: default_outflow_account(),
            default_inflow_account: default_inflow_account(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: AmountRounding::Reject,
            date_formats: default_date_formats(),
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        }
    }

    #[test]
    fn a_paid_mail_candidate_is_negative_and_idempotent_by_gmail_thread() {
        let one = mail_candidate(
            "deadbeef",
            "2026-08-10",
            "Cafe receipt",
            "12,50",
            ',',
            "eur",
            "assets:bank:checking",
        )
        .unwrap();
        let two = mail_candidate(
            "deadbeef",
            "2026-08-10",
            "Different extracted merchant",
            "99,00",
            ',',
            "EUR",
            MAIL_SOURCE_ACCOUNT_REQUIRED,
        )
        .unwrap();
        assert_eq!(one.id, two.id);
        assert_eq!(one.fingerprint, two.fingerprint);
        assert_eq!(one.amount_cents, -1_250);
        assert_eq!(one.source_reference.as_deref(), Some("gmail:deadbeef"));
        assert_eq!(one.proposed_account, "expenses:uncategorized");
    }

    #[test]
    fn a_paid_mail_candidate_rejects_invalid_money_and_accounts() {
        assert_eq!(mail_decimal_separator("12,50"), ',');
        assert_eq!(mail_decimal_separator("1.234,56"), ',');
        assert_eq!(mail_decimal_separator("1,234.56"), '.');
        assert_eq!(mail_decimal_separator("1,234"), '.');
        assert!(mail_candidate(
            "deadbeef",
            "2026-08-10",
            "Cafe",
            "12,500",
            ',',
            "EUR",
            "assets:bank:checking"
        )
        .is_err());
        assert!(mail_candidate(
            "deadbeef",
            "2026-08-10",
            "Cafe",
            "12,50",
            ',',
            "EUR",
            "expenses:food"
        )
        .is_err());
    }

    #[test]
    fn a_configurable_german_csv_becomes_review_candidates() {
        let csv = b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n03.08.2026;1.234,50;Employer;row-2;EUR\n";
        let rows = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(rows[0].booked_at, "2026-08-02");
        assert_eq!(rows[0].amount_cents, -1234);
        assert_eq!(rows[0].proposed_account, "expenses:uncategorized");
        assert_eq!(rows[1].amount_cents, 123450);
        assert_eq!(rows[1].proposed_account, "income:uncategorized");
        assert!(rows.iter().all(|row| row.state == CandidateState::Pending));
    }

    #[test]
    fn ordered_rules_are_case_insensitive_and_direction_aware() {
        let mut mapping = mapping();
        mapping.categorization_rules = vec![
            CsvCategorizationRule {
                description_contains_any: vec!["synthetic".into()],
                description_starts_with_any: Vec::new(),
                field_equals_any: Vec::new(),
                direction: CsvRuleDirection::Inflow,
                account: "income:synthetic-refund".into(),
                confidence_basis_points: 8_000,
            },
            CsvCategorizationRule {
                description_contains_any: vec!["SERVICE".into()],
                description_starts_with_any: Vec::new(),
                field_equals_any: Vec::new(),
                direction: CsvRuleDirection::Outflow,
                account: "expenses:software".into(),
                confidence_basis_points: 9_500,
            },
        ];
        let candidates = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Synthetic Service;one;EUR\n03.08.2026;12,34;Synthetic Service;two;EUR\n",
            &mapping,
        )
        .unwrap();

        assert_eq!(candidates[0].proposed_account, "expenses:software");
        assert_eq!(candidates[0].confidence_basis_points, 9_500);
        assert_eq!(candidates[1].proposed_account, "income:synthetic-refund");
        assert_eq!(candidates[1].confidence_basis_points, 8_000);
    }

    #[test]
    fn rule_only_columns_do_not_replace_the_canonical_description() {
        let mut mapping = mapping();
        mapping.categorization_columns = vec!["Statement".into()];
        mapping.categorization_rules = vec![CsvCategorizationRule {
            description_contains_any: vec!["synthetic provider".into()],
            description_starts_with_any: Vec::new(),
            field_equals_any: Vec::new(),
            direction: CsvRuleDirection::Outflow,
            account: "expenses:software".into(),
            confidence_basis_points: 9_000,
        }];
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Statement;Referenz;Waehrung\n02.08.2026;-12,34;Readable purchase;Synthetic Provider;one;EUR\n",
            &mapping,
        )
        .unwrap()
        .remove(0);

        assert_eq!(candidate.description, "Readable purchase");
        assert_eq!(candidate.proposed_account, "expenses:software");
    }

    #[test]
    fn a_custom_inflow_default_can_fail_closed_for_manual_review() {
        let mut mapping = mapping();
        mapping.default_inflow_account = "review:credit-or-refund".into();
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;12,34;Synthetic credit;one;EUR\n",
            &mapping,
        )
        .unwrap()
        .remove(0);

        assert_eq!(candidate.proposed_account, "review:credit-or-refund");
        assert_eq!(candidate.confidence_basis_points, 0);
        assert!(render_journal_entry(&candidate, &candidate.proposed_account).is_err());
    }

    #[test]
    fn settlement_rules_can_propose_balance_transfers() {
        let mut mapping = mapping();
        mapping.source_account = "liabilities:card:review".into();
        mapping.categorization_rules = vec![CsvCategorizationRule {
            description_contains_any: vec!["settlement".into()],
            description_starts_with_any: Vec::new(),
            field_equals_any: Vec::new(),
            direction: CsvRuleDirection::Inflow,
            account: "assets:bank:checking".into(),
            confidence_basis_points: 10_000,
        }];
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;12,34;Synthetic Settlement;one;EUR\n",
            &mapping,
        )
        .unwrap()
        .remove(0);

        assert_eq!(candidate.proposed_account, "assets:bank:checking");
        assert!(render_journal_entry(&candidate, &candidate.proposed_account).is_ok());
    }

    #[test]
    fn invalid_categorization_rules_are_rejected_before_rows_are_read() {
        let mut mapping = mapping();
        mapping.categorization_rules = vec![CsvCategorizationRule {
            description_contains_any: vec![" ".into()],
            description_starts_with_any: Vec::new(),
            field_equals_any: Vec::new(),
            direction: CsvRuleDirection::Any,
            account: "expenses:software".into(),
            confidence_basis_points: 10_001,
        }];

        let error = preview_csv(b"", &mapping).unwrap_err();
        assert!(error.0.contains("non-blank matcher"));
    }

    #[test]
    fn exact_field_rules_and_row_filters_keep_mixed_exports_bounded() {
        let mut mapping = mapping();
        mapping.categorization_rules = vec![CsvCategorizationRule {
            description_contains_any: Vec::new(),
            description_starts_with_any: Vec::new(),
            field_equals_any: vec![CsvFieldEquals {
                column: "Code".into(),
                values: vec!["5811".into()],
            }],
            direction: CsvRuleDirection::Outflow,
            account: "expenses:food:canteen".into(),
            confidence_basis_points: 10_000,
        }];
        mapping.row_filter = Some(CsvRowFilter {
            column: "Type".into(),
            include_values: vec!["card".into()],
        });
        let prepared = prepare_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung;Type;Code\n02.08.2026;-12,34;Synthetic lunch;one;EUR;CARD;5811\n03.08.2026;-5,00;Synthetic trade;two;EUR;BUY;5811\n",
            &mapping,
        )
        .unwrap();

        assert_eq!(prepared.candidates.len(), 1);
        assert_eq!(prepared.preview.ignored_non_transaction_rows, 1);
        assert_eq!(
            prepared.candidates[0].proposed_account,
            "expenses:food:canteen"
        );
    }

    #[test]
    fn location_columns_capture_raw_address_fields_verbatim() {
        let mut with_location = mapping();
        with_location.location_columns = Some(CsvLocationColumns {
            street_column: Some("Adresse".into()),
            postal_code_column: Some("PLZ".into()),
            city_column: None,
            country_column: Some("Land".into()),
        });
        // The quoted street field carries two lines, as the Amex export does:
        // line 1 street, line 2 city. It must survive byte-for-byte.
        let csv = b"Buchung;Betrag;Text;Referenz;Waehrung;Adresse;PLZ;Land\n02.08.2026;-12,34;Synthetic market;row-1;EUR;\"Beispielstr. 1\nMusterstadt\";12345;Deutschland\n03.08.2026;-5,00;Synthetic service;row-2;EUR;; ;\n";

        let rows = parse_csv(csv, &with_location).unwrap();

        assert_eq!(
            rows[0].location_street.as_deref(),
            Some("Beispielstr. 1\nMusterstadt")
        );
        assert_eq!(rows[0].location_postal_code.as_deref(), Some("12345"));
        assert_eq!(rows[0].location_city, None);
        assert_eq!(rows[0].location_country.as_deref(), Some("Deutschland"));
        // Blank and whitespace-only cells are absent, not empty strings.
        assert_eq!(rows[1].location_street, None);
        assert_eq!(rows[1].location_postal_code, None);

        // Location never enters the fingerprint: venue links in the places
        // capability key on identities minted before these fields existed.
        let without_location = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(rows[0].fingerprint, without_location[0].fingerprint);
        assert_eq!(without_location[0].location_street, None);

        // A configured location column that is absent fails closed like every
        // other configured column.
        let error =
            parse_csv(b"Buchung;Betrag;Text;Referenz;Waehrung\n", &with_location).unwrap_err();
        assert!(error.0.contains("Adresse"));
    }

    #[test]
    fn a_mapping_without_location_columns_parses_exactly_as_before() {
        let json = r#"{"date_column":"Buchung","amount_column":"Betrag","description_column":"Text","source_account":"assets:bank:checking"}"#;
        let parsed: serde_json::Result<CsvMapping> = serde_json::from_str(json);
        let deserialized = parsed.unwrap();
        assert_eq!(deserialized.location_columns, None);

        let csv =
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n";
        let rows = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(rows[0].location_street, None);
        assert_eq!(rows[0].location_postal_code, None);
        assert_eq!(rows[0].location_city, None);
        assert_eq!(rows[0].location_country, None);
    }

    #[test]
    fn fingerprints_make_reimport_deterministic() {
        let csv =
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n";
        let first = parse_csv(csv, &mapping()).unwrap();
        let second = parse_csv(csv, &mapping()).unwrap();
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn credit_card_shape_is_explicitly_normalized_before_preview() {
        let mut mapping = mapping();
        mapping.delimiter = ',';
        mapping.date_column = "Date".into();
        mapping.amount_column = "Amount".into();
        mapping.description_column = "Description".into();
        mapping.reference_column = None;
        mapping.currency_column = None;
        mapping.amount_sign = AmountSign::Invert;
        mapping.date_formats = vec![CsvDateFormat::DayMonthYearSlashes];
        mapping.row_policy = CsvRowPolicy::RequiredFields;
        mapping.source_account = "liabilities:card:review".into();
        let csv = b"\xef\xbb\xbfDate,Amount,Description,Optional\n09/08/2026,\"12,34\",Synthetic service\nSummary\n09/08/2026,\"12,34\",Synthetic service\n";

        let prepared = prepare_csv(csv, &mapping).unwrap();

        assert_eq!(prepared.preview.candidate_count, 2);
        assert_eq!(prepared.preview.outflow_count, 2);
        assert_eq!(prepared.preview.inflow_count, 0);
        assert_eq!(prepared.preview.duplicate_rows, 0);
        assert_eq!(prepared.preview.preserved_repetitions, 1);
        assert_eq!(prepared.preview.ignored_non_transaction_rows, 1);
        assert_eq!(prepared.candidates[0].booked_at, "2026-08-09");
        assert_eq!(prepared.candidates[0].amount_cents, -1234);
        assert_ne!(prepared.candidates[0].id, prepared.candidates[1].id);
        assert_eq!(
            prepared.candidates[0].proposed_account,
            "expenses:uncategorized"
        );
    }

    #[test]
    fn a_stable_reference_still_collapses_an_exact_duplicate() {
        let csv = b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Synthetic market;row-1;EUR\n02.08.2026;-12,34;Synthetic market;row-1;EUR\n";

        let prepared = prepare_csv(csv, &mapping()).unwrap();

        assert_eq!(prepared.preview.candidate_count, 1);
        assert_eq!(prepared.preview.duplicate_rows, 1);
        assert_eq!(prepared.preview.preserved_repetitions, 0);
    }

    #[test]
    fn the_first_reference_less_occurrence_keeps_its_existing_identity() {
        let mut mapping = mapping();
        mapping.reference_column = None;
        let single = b"Buchung;Betrag;Text;Waehrung\n02.08.2026;-12,34;Synthetic market;EUR\n";
        let repeated = b"Buchung;Betrag;Text;Waehrung\n02.08.2026;-12,34;Synthetic market;EUR\n02.08.2026;-12,34;Synthetic market;EUR\n";

        let single = prepare_csv(single, &mapping).unwrap();
        let repeated = prepare_csv(repeated, &mapping).unwrap();

        assert_eq!(single.candidates[0].id, repeated.candidates[0].id);
        assert_ne!(repeated.candidates[0].id, repeated.candidates[1].id);
        assert_eq!(repeated.preview.preserved_repetitions, 1);
    }

    #[test]
    fn rows_with_amounts_fail_closed_when_dates_are_unsupported() {
        let mut mapping = mapping();
        mapping.delimiter = ',';
        mapping.date_column = "Date".into();
        mapping.amount_column = "Amount".into();
        mapping.description_column = "Description".into();
        mapping.reference_column = None;
        mapping.currency_column = None;
        mapping.date_formats = vec![CsvDateFormat::DayMonthYearSlashes];
        mapping.row_policy = CsvRowPolicy::RequiredFields;
        let error = preview_csv(
            b"Date,Amount,Description\n09-08-2026,12.34,Synthetic service\n",
            &mapping,
        )
        .unwrap_err();

        assert!(error.0.contains("CSV row 2"));
        assert!(error.0.contains("configured formats"));
    }

    #[test]
    fn amounts_with_unconfigured_text_fail_closed() {
        let error = parse_decimal_cents("EUR 12,34", ',').unwrap_err();
        assert_eq!(error.0, "amount is not a decimal number");
        assert_eq!(parse_decimal_cents("+12,34", ',').unwrap(), 1234);
        assert_eq!(parse_decimal_cents("12,34-", ',').unwrap(), -1234);
    }

    #[test]
    fn configured_rounding_handles_high_precision_cash_exports() {
        assert_eq!(
            parse_decimal_cents_with_rounding("12.345", '.', AmountRounding::HalfAwayFromZero)
                .unwrap(),
            1235
        );
        assert_eq!(
            parse_decimal_cents_with_rounding("-12.345", '.', AmountRounding::HalfAwayFromZero)
                .unwrap(),
            -1235
        );
        assert!(parse_decimal_cents("12.345", '.').is_err());
    }

    #[test]
    fn balance_account_transfers_render_without_weakening_direction_checks() {
        let mut candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Synthetic settlement;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);

        assert!(render_journal_entry(&candidate, "liabilities:card:review").is_ok());
        assert!(render_journal_entry(&candidate, "income:salary").is_err());

        candidate.source_account = "liabilities:card:review".into();
        candidate.amount_cents = 1234;
        assert!(render_journal_entry(&candidate, "assets:bank:checking").is_ok());
        assert!(render_journal_entry(&candidate, "expenses:food").is_err());
    }

    #[test]
    fn only_a_reviewed_direction_can_render() {
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        assert!(render_journal_entry(&candidate, "income:salary").is_err());
        let entry = render_journal_entry(&candidate, "expenses:food").unwrap();
        assert!(entry.contains("assets:bank:checking  -12.34 EUR"));
        assert!(entry.contains("source-id:"));
    }

    #[test]
    fn reciprocal_balance_rows_match_across_nearby_booking_dates() {
        let mut bank = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n14.07.2026;-12,34;Synthetic card debit;bank;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        bank.proposed_account = "liabilities:card:review".into();
        let mut card = bank.clone();
        card.id = "card-candidate".into();
        card.fingerprint = "card-fingerprint".into();
        card.booked_at = "2026-07-12".into();
        card.amount_cents = 1234;
        card.source_account = "liabilities:card:review".into();
        card.proposed_account = "assets:bank:checking".into();

        assert!(is_transfer_pair(&bank, &card));
        assert_eq!(
            transfer_match_ids(&[bank.clone(), card.clone()], &bank),
            [card.id.clone()]
        );
        card.booked_at = "2026-07-10".into();
        assert!(!is_transfer_pair(&bank, &card));
    }

    #[test]
    fn transfer_matching_requires_reciprocal_accounts_and_an_unambiguous_amount() {
        let mut bank = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n31.07.2026;-12,34;Synthetic card debit;bank;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        bank.proposed_account = "liabilities:card:review".into();
        let mut card = bank.clone();
        card.id = "card-candidate".into();
        card.booked_at = "2026-08-01".into();
        card.amount_cents = 1234;
        card.source_account = "liabilities:card:review".into();
        card.proposed_account = "assets:bank:checking".into();
        assert!(is_transfer_pair(&bank, &card));

        card.proposed_account = "assets:bank:other".into();
        assert!(!is_transfer_pair(&bank, &card));
    }

    #[test]
    fn symbolic_accounts_cannot_carry_bank_identifiers_or_journal_syntax() {
        assert!(validate_account("assets:bank:checking").is_ok());
        assert!(validate_account("assets:bank:account with space").is_err());
        assert!(validate_account("expenses:food\n2026-01-01 bad").is_err());
    }

    #[test]
    fn confirmation_appends_once_and_remains_a_valid_journal() {
        let candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,34;Example market;row-1;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        let path = std::env::temp_dir().join(format!(
            "axon-finance-import-{}-{}.journal",
            std::process::id(),
            candidate.fingerprint
        ));
        std::fs::write(&path, "decimal-mark .\ncommodity 1,000.00 EUR\n").unwrap();
        assert!(append_confirmed(&path, &candidate, "expenses:food").unwrap());
        assert!(!append_confirmed(&path, &candidate, "expenses:food").unwrap());
        let journal = std::fs::read_to_string(&path).unwrap();
        assert_eq!(journal.matches("source-id:").count(), 1);
        // The appended journal has to be readable, not merely written. This was
        // conditional on hledger being installed until 2026-08-28, so on a
        // machine without it -- this one -- the assertion never ran at all.
        use crate::accounting::AccountingEngine;
        crate::accounting::JournalEngine::new(&path)
            .check()
            .unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_confirmed_account_can_be_reclassified_without_changing_source_identity() {
        let mut candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,00;Synthetic housing;row-reclass;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        candidate.state = CandidateState::Confirmed;
        candidate.proposed_account = "expenses:uncategorized".into();
        let journal = render_journal_entry(&candidate, &candidate.proposed_account).unwrap();

        let (rewritten, changed) =
            rewrite_confirmed_account(&journal, &candidate, "expenses:housing").unwrap();
        assert!(changed);
        assert!(!rewritten.contains("expenses:uncategorized"));
        assert!(rewritten.contains("expenses:housing"));
        assert_eq!(rewritten.matches("source-id:").count(), 1);

        let (same, changed) =
            rewrite_confirmed_account(&rewritten, &candidate, "expenses:housing").unwrap();
        assert!(!changed);
        assert_eq!(same, rewritten);
    }

    #[test]
    fn reclassification_refuses_a_confirmed_entry_that_drifted_in_the_journal() {
        let mut candidate = parse_csv(
            b"Buchung;Betrag;Text;Referenz;Waehrung\n02.08.2026;-12,00;Synthetic housing;row-reclass-drift;EUR\n",
            &mapping(),
        )
        .unwrap()
        .remove(0);
        candidate.state = CandidateState::Confirmed;
        candidate.proposed_account = "expenses:uncategorized".into();
        let journal = render_journal_entry(&candidate, "expenses:manually-reviewed").unwrap();

        assert!(rewrite_confirmed_account(&journal, &candidate, "expenses:housing").is_err());
    }
}

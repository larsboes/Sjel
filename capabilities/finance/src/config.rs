//! Public, zero-personal-data config for the finance store.
//!
//! Nothing here names a vault, a bank, an institution or a figure. The overlay
//! supplies all of it, which is what keeps this capability publishable while the
//! data it operates on is the most private in the system.
//!
//! The store path comes from `sjel_config::database_path`: `$SJEL_DB_PATH`, else
//! `$SJEL_PERSONAL_ROOT/data/axon/axon.db`. It is a deployment fact rather than a
//! capability one (PRD Q45), so `$SJEL_FINANCE_DATABASE_URL` is gone -- a file per
//! capability would drop the cross-capability joins places builds its spend layer
//! on.

use crate::import::CsvMapping;
use crate::investment::{HoldingsCoverage, InvestmentCsvMapping};
use crate::planning::PlanningConfig;
use serde::{Deserialize, Serialize};
use sjel_config::{database_path, expand_tilde, resolve_port};
use std::path::PathBuf;

/// Where subscription notes live. Journal and budget configuration are separate
/// because neither requires an Obsidian vault.
#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub subscriptions_dir: PathBuf,
}

/// A reusable import shape whose values come only from the private overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvMappingProfile {
    pub label: String,
    pub mapping: CsvMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestmentCsvMappingProfile {
    pub source_key: String,
    pub label: String,
    #[serde(default)]
    pub coverage: HoldingsCoverage,
    pub mapping: InvestmentCsvMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringCommitment {
    pub id: String,
    pub label: String,
    pub account: String,
    pub monthly_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub valid_from: String,
    #[serde(default)]
    pub valid_until: Option<String>,
}

fn default_currency() -> String {
    "EUR".into()
}

impl RecurringCommitment {
    pub fn active_on(&self, date: &str) -> bool {
        self.valid_from.as_str() <= date
            && self
                .valid_until
                .as_deref()
                .is_none_or(|valid_until| date <= valid_until)
    }
}

/// One instrument the owner holds, named so a proposal can talk about it.
///
/// Three fields, and `currency` is deliberately not one of them:
/// `finance_holding_projection.currency` is NOT NULL and written by the reviewed
/// import, and a hand-typed second copy that disagrees is silent drift inside the
/// very arithmetic that produces the drift figure.
///
/// `label` is the token the feed-evidence matcher looks for. Under three
/// characters it is refused as a matcher rather than used, because a two-letter
/// label matched as a substring finds nothing but noise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentProfile {
    pub instrument: String,
    pub label: String,
    #[serde(default = "default_asset_class")]
    pub asset_class: String,
    /// The symbol a market-data provider knows this instrument by. Absent means
    /// no networked provider prices it and `broker` is its only source.
    #[serde(default)]
    pub ticker: Option<String>,
}

fn default_asset_class() -> String {
    "equity".into()
}

/// One target allocation, in basis points, with the band around it that decides
/// when drift is worth a decision.
///
/// `valid_from`/`valid_until` copy [`RecurringCommitment`]'s dating and its
/// `active_on` predicate, so a cohort is "the allocations active on this date".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetAllocation {
    pub id: String,
    pub label: String,
    /// `instrument` or `asset_class`; selects which nullable key below is read.
    pub scope: String,
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub asset_class: Option<String>,
    pub target_bp: i64,
    pub band_bp: i64,
    #[serde(default)]
    pub rationale: String,
    pub valid_from: String,
    #[serde(default)]
    pub valid_until: Option<String>,
}

impl TargetAllocation {
    pub fn active_on(&self, date: &str) -> bool {
        self.valid_from.as_str() <= date
            && self
                .valid_until
                .as_deref()
                .is_none_or(|valid_until| date <= valid_until)
    }
}

/// The hand-typed investment policy, in the overlay beside every other hand-typed
/// policy this capability reads (commitments, spending rules, card options,
/// loyalty balances). Not a table: it needs no write route and no confirm gate,
/// and the overlay is already inside `service.toml`'s backup contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetPolicy {
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Below this, a contribution is not worth a decision.
    #[serde(default)]
    pub contribution_floor_cents: i64,
    /// How old a market price may be before a position reads as stale. Here and
    /// not on `PlanningConfig`, because `portfolio` reads it and `planning` does
    /// not.
    #[serde(default = "default_price_freshness_days")]
    pub price_freshness_days: i64,
    /// The annual risk-free rate in basis points. `None` omits the max-Sharpe
    /// portfolio with a reason rather than defaulting it to zero -- a zero here
    /// is a claim about the world, not a missing value.
    #[serde(default)]
    pub risk_free_rate_bp: Option<i64>,
    #[serde(default)]
    pub allocations: Vec<TargetAllocation>,
}

fn default_price_freshness_days() -> i64 {
    4
}

impl Default for TargetPolicy {
    fn default() -> Self {
        Self {
            currency: default_currency(),
            contribution_floor_cents: 0,
            price_freshness_days: default_price_freshness_days(),
            risk_free_rate_bp: None,
            allocations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The one shared SQLite file, under the table prefix `finance` (PRD Q45).
    pub database_path: PathBuf,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
    pub journal: Option<PathBuf>,
    pub investment_snapshot: Option<PathBuf>,
    pub balance_snapshot: Option<PathBuf>,
    pub commitments: Vec<RecurringCommitment>,
    pub csv_mappings: Vec<CsvMappingProfile>,
    pub investment_csv_mappings: Vec<InvestmentCsvMappingProfile>,
    pub planning: PlanningConfig,
    pub instruments: Vec<InstrumentProfile>,
    pub targets: Option<TargetPolicy>,
    /// Where the decision ledger's human-readable copy is written.
    ///
    /// Its own field rather than "the overlay root, always", because a live check
    /// needs to redirect the WRITE without also redirecting the config READ --
    /// pointing `SJEL_PERSONAL_ROOT` at a scratch directory does both, so a
    /// verification run either writes into the owner's overlay or runs against a
    /// configuration that is not theirs. `SJEL_FINANCE_DECISIONS_ROOT` separates
    /// the two. `None` means no overlay is configured and nothing is exported.
    pub decisions_root: Option<PathBuf>,
    /// Where the feed-evidence lookup goes. Loopback, and the class rule that
    /// makes it safe is stated at the point of copy in `decision.rs`: only id,
    /// title, url and day are read, and those are published sources.
    pub comms_base_url: String,
}

#[derive(Debug, Deserialize)]
struct FinanceFileConfig {
    obsidian: Option<FinanceFileObsidian>,
    journal: Option<String>,
    investment_snapshot: Option<String>,
    balance_snapshot: Option<String>,
    #[serde(default)]
    commitments: Vec<RecurringCommitment>,
    #[serde(default)]
    csv_mappings: Vec<CsvMappingProfile>,
    #[serde(default)]
    investment_csv_mappings: Vec<InvestmentCsvMappingProfile>,
    #[serde(default)]
    planning: PlanningConfig,
    #[serde(default)]
    instruments: Vec<InstrumentProfile>,
    #[serde(default)]
    targets: Option<TargetPolicy>,
    #[serde(default)]
    comms_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FinanceFileObsidian {
    root: String,
    #[serde(default = "default_subscriptions_dir")]
    subscriptions_dir: String,
}

fn default_subscriptions_dir() -> String {
    "Atlas/Finance/Subscriptions".into()
}

/// comms' loopback port. A capability calling another capability's HTTP contract
/// rather than linking its crate is the direction `CONTRIBUTING.md#schemas-and-dependency-direction`
/// asks for, and the port is the deployment's, so the overlay may override it.
const DEFAULT_COMMS_BASE_URL: &str = "http://127.0.0.1:8083";

fn file_config() -> Option<FinanceFileConfig> {
    let overlay = sjel_config::env_var("SJEL_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("finance.json");
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

impl Config {
    pub fn load() -> Self {
        let port = resolve_port(None, None, 8090);
        let personal = file_config();
        let obsidian = match sjel_config::env_var("SJEL_FINANCE_OBSIDIAN_ROOT") {
            Ok(root) => Some(ObsidianConfig {
                root: expand_tilde(&root),
                subscriptions_dir: PathBuf::from(
                    sjel_config::env_var("SJEL_FINANCE_OBSIDIAN_DIR")
                        .unwrap_or_else(|_| default_subscriptions_dir()),
                ),
            }),
            Err(_) => personal
                .as_ref()
                .and_then(|config| config.obsidian.as_ref())
                .map(|obsidian| ObsidianConfig {
                    root: expand_tilde(&obsidian.root),
                    subscriptions_dir: PathBuf::from(&obsidian.subscriptions_dir),
                }),
        };
        let commitments = personal
            .as_ref()
            .map(|config| config.commitments.clone())
            .unwrap_or_default();
        let csv_mappings = personal
            .as_ref()
            .map(|config| config.csv_mappings.clone())
            .unwrap_or_default();
        let investment_csv_mappings = personal
            .as_ref()
            .map(|config| config.investment_csv_mappings.clone())
            .unwrap_or_default();
        let planning = personal
            .as_ref()
            .map(|config| config.planning.clone())
            .unwrap_or_default();
        let instruments = personal
            .as_ref()
            .map(|config| config.instruments.clone())
            .unwrap_or_default();
        let targets = personal.as_ref().and_then(|config| config.targets.clone());
        let decisions_root = sjel_config::env_var("SJEL_FINANCE_DECISIONS_ROOT")
            .ok()
            .map(|root| expand_tilde(&root))
            .or_else(|| {
                sjel_config::env_var("SJEL_PERSONAL_ROOT")
                    .ok()
                    .map(|root| expand_tilde(&root))
            });
        let comms_base_url = sjel_config::env_var("SJEL_COMMS_BASE_URL")
            .ok()
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.comms_base_url.clone())
            })
            .unwrap_or_else(|| DEFAULT_COMMS_BASE_URL.to_string());
        let journal = sjel_config::env_var("SJEL_FINANCE_JOURNAL")
            .ok()
            .map(|path| expand_tilde(&path))
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.journal.as_ref())
                    .map(|path| expand_tilde(path))
            });
        let investment_snapshot = sjel_config::env_var("SJEL_FINANCE_INVESTMENT_SNAPSHOT")
            .ok()
            .map(|path| expand_tilde(&path))
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.investment_snapshot.as_ref())
                    .map(|path| expand_tilde(path))
            });
        let balance_snapshot = sjel_config::env_var("SJEL_FINANCE_BALANCE_SNAPSHOT")
            .ok()
            .map(|path| expand_tilde(&path))
            .or_else(|| {
                personal
                    .as_ref()
                    .and_then(|config| config.balance_snapshot.as_ref())
                    .map(|path| expand_tilde(path))
            });
        Self {
            database_path: database_path(),
            port,
            obsidian,
            journal,
            investment_snapshot,
            balance_snapshot,
            commitments,
            csv_mappings,
            investment_csv_mappings,
            planning,
            instruments,
            targets,
            decisions_root,
            comms_base_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_directory_matches_where_the_notes_already_live() {
        assert_eq!(default_subscriptions_dir(), "Atlas/Finance/Subscriptions");
    }

    #[test]
    fn private_csv_mapping_profiles_parse_as_typed_config() {
        let config: FinanceFileConfig = serde_json::from_value(serde_json::json!({
            "csv_mappings": [{
                "label": "Synthetic semicolon export",
                "mapping": {
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "date_column": "Date",
                    "amount_column": "Amount",
                    "description_column": "Description",
                    "reference_column": "Reference",
                    "currency_column": "Currency",
                    "default_currency": "EUR",
                    "source_account": "assets:bank:checking",
                    "amount_sign": "invert",
                    "date_formats": ["day_month_year_slashes"],
                    "row_policy": "required_fields"
                }
            }]
        }))
        .unwrap();

        assert_eq!(config.csv_mappings.len(), 1);
        assert_eq!(config.csv_mappings[0].label, "Synthetic semicolon export");
        assert_eq!(config.csv_mappings[0].mapping.date_column, "Date");
        assert_eq!(
            config.csv_mappings[0].mapping.amount_sign,
            crate::import::AmountSign::Invert
        );
        assert_eq!(
            config.csv_mappings[0].mapping.date_formats,
            [crate::import::CsvDateFormat::DayMonthYearSlashes]
        );
        assert_eq!(
            config.csv_mappings[0].mapping.row_policy,
            crate::import::CsvRowPolicy::RequiredFields
        );
    }

    #[test]
    fn csv_mapping_profiles_are_optional() {
        let config: FinanceFileConfig = serde_json::from_str("{}").unwrap();
        assert!(config.csv_mappings.is_empty());
        assert!(config.investment_csv_mappings.is_empty());
        assert!(config.commitments.is_empty());
    }

    #[test]
    fn dated_commitments_are_active_only_inside_their_window() {
        let commitment = RecurringCommitment {
            id: "synthetic-rent".into(),
            label: "Synthetic rent".into(),
            account: "expenses:housing:rent".into(),
            monthly_cents: 75_000,
            currency: "EUR".into(),
            valid_from: "2026-09-01".into(),
            valid_until: Some("2027-08-31".into()),
        };
        assert!(!commitment.active_on("2026-08-31"));
        assert!(commitment.active_on("2026-09-01"));
        assert!(commitment.active_on("2027-08-31"));
        assert!(!commitment.active_on("2027-09-01"));
    }

    #[test]
    fn private_investment_mapping_profiles_parse_as_typed_config() {
        let config: FinanceFileConfig = serde_json::from_value(serde_json::json!({
            "investment_csv_mappings": [{
                "source_key": "synthetic-broker",
                "label": "Synthetic activity export",
                "mapping": {
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "date_column": "Date",
                    "instrument_column": "Instrument",
                    "quantity_column": "Quantity",
                    "activity_type_column": "Type",
                    "position_activity_values": ["BUY", "SELL"],
                    "non_position_activity_values": ["DIVIDEND"],
                    "reference_column": "Reference",
                    "price_column": "Price",
                    "currency_column": "Currency",
                    "default_currency": "EUR",
                    "instrument_aliases": {"source-1": "ACME"}
                }
            }],
            "investment_snapshot": "/private/state/holdings.json"
        }))
        .unwrap();

        assert_eq!(config.investment_csv_mappings.len(), 1);
        assert_eq!(
            config.investment_csv_mappings[0].source_key,
            "synthetic-broker"
        );
        assert_eq!(
            config.investment_csv_mappings[0].coverage,
            HoldingsCoverage::Complete
        );
        assert_eq!(
            config.investment_snapshot.as_deref(),
            Some("/private/state/holdings.json")
        );
        assert_eq!(
            config.investment_csv_mappings[0].mapping.instrument_aliases["source-1"],
            "ACME"
        );
        assert_eq!(
            config.investment_csv_mappings[0]
                .mapping
                .position_activity_values,
            ["BUY", "SELL"]
        );
    }
}

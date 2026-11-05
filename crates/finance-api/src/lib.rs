//! Typed API contract, wire models, and route constants for the Sjel finance capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-finance`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_DASHBOARD: &str = "/api/dashboard";
pub const ROUTE_SUBSCRIPTIONS: &str = "/api/subscriptions";
pub const ROUTE_SUBSCRIPTIONS_BURN: &str = "/api/subscriptions/burn";
pub const ROUTE_SUBSCRIPTION_PRICE: &str = "/api/subscriptions/{id}/price";
pub const ROUTE_SUBSCRIPTION_STATE: &str = "/api/subscriptions/{id}/state";
pub const ROUTE_IMPORT_CSV_MAPPINGS: &str = "/api/import/csv/mappings";
pub const ROUTE_IMPORT_CANDIDATES: &str = "/api/import/candidates";
pub const ROUTE_IMPORT_CSV_PREVIEW: &str = "/api/import/csv/preview";
pub const ROUTE_IMPORT_CSV: &str = "/api/import/csv";
pub const ROUTE_IMPORT_CANDIDATE_REVIEW: &str = "/api/import/candidates/{id}/review";
pub const ROUTE_IMPORT_CANDIDATE_RECONCILE_TRANSFER: &str =
    "/api/import/candidates/{id}/reconcile-transfer";
pub const ROUTE_IMPORT_CANDIDATES_RECLASSIFY_BATCH: &str =
    "/api/import/candidates/reclassify-batch";
pub const ROUTE_IMPORT_CANDIDATE_ALLOCATION: &str = "/api/import/candidates/{id}/allocation";
pub const ROUTE_IMPORT_CANDIDATE_REIMBURSEMENT: &str = "/api/import/candidates/{id}/reimbursement";
pub const ROUTE_BALANCE_SNAPSHOT: &str = "/api/balance-snapshot";
pub const ROUTE_PORTFOLIO: &str = "/api/portfolio";
pub const ROUTE_PRICES_STATUS: &str = "/api/prices/status";
pub const ROUTE_DECISIONS: &str = "/api/decisions";
pub const ROUTE_DECISIONS_RUN: &str = "/api/decisions/run";
pub const ROUTE_DECISION_VERDICT: &str = "/api/decisions/{id}/verdict";
pub const ROUTE_TRIP_SPENDING: &str = "/api/trips/{id}/spending";
pub const ROUTE_LINKS: &str = "/api/links";

// ============================================================================
// Enums and Common Vocabularies
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Income,
    Expense,
    Transfer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Pending,
    Confirmed,
    Rejected,
    Duplicate,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BalanceCoverage {
    Complete,
    #[default]
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BalanceKind {
    Asset,
    Liability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BillingCycle {
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
    OneOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionState {
    Considering,
    Trial,
    Active,
    Covered,
    Paused,
    Cancelled,
}

// ============================================================================
// Data Models
// ============================================================================

/// A transaction in `GET /finance/api/dashboard`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceTransaction {
    pub id: String,
    pub date: String,
    pub description: String,
    pub kind: String,
    pub account: String,
    pub category: String,
    pub amount_cents: i64,
    pub currency: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub trip_id: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub shared_cents: i64,
    #[serde(default)]
    pub cash_amount_cents: i64,
    #[serde(default)]
    pub reimbursement_for: Option<String>,
}

/// Finance CSV profile returned by `GET /finance/api/import/csv/mappings`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct FinanceCsvMappingProfile {
    pub label: String,
    pub mapping: serde_json::Value,
}

/// Candidate shown in the import review queue (`GET /finance/api/import/candidates`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceCandidate {
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
    pub state: String,
    #[serde(default)]
    pub transfer_match_ids: Vec<String>,
}

/// Preview of a CSV upload before confirmation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceCsvPreview {
    pub preview_id: String,
    pub candidate_count: usize,
    pub duplicate_rows: usize,
    pub preserved_repetitions: usize,
    pub ignored_non_transaction_rows: usize,
    pub outflow_count: usize,
    pub inflow_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceDashboardSummary {
    #[serde(default)]
    pub income_cents: i64,
    #[serde(default)]
    pub personal_spending_cents: i64,
    #[serde(default)]
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SharedExpenseSummary {
    pub source_id: String,
    pub candidate_id: String,
    pub date: String,
    pub description: String,
    pub account: String,
    pub category: String,
    pub purpose: Option<String>,
    pub trip_id: Option<String>,
    pub gross_cents: i64,
    pub personal_cents: i64,
    pub shared_cents: i64,
    pub reimbursed_cents: i64,
    pub outstanding_cents: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ManualBalance {
    pub id: String,
    pub label: String,
    pub kind: BalanceKind,
    pub amount_cents: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ManualBalanceSnapshot {
    pub schema_version: u32,
    pub as_of: String,
    pub updated_at: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ManualBalanceUpdate {
    pub as_of: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceDecimalValue {
    pub mantissa: String,
    pub scale: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct TrackedNetWorth {
    pub currency: String,
    pub value: FinanceDecimalValue,
    pub manual_balance_cents: i64,
    pub portfolio_included: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct FinanceDashboardProjection {
    #[serde(default)]
    pub summary: FinanceDashboardSummary,
    #[serde(default)]
    pub transactions: Vec<FinanceTransaction>,
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub shared_expenses: Vec<SharedExpenseSummary>,
    #[serde(default)]
    pub balance_snapshot: Option<ManualBalanceSnapshot>,
    #[serde(default)]
    pub tracked_net_worth: Option<TrackedNetWorth>,
    /// One point per calendar month (`YYYY-MM`), oldest first.
    #[serde(default)]
    pub trend: Vec<FinanceTrendPoint>,
    #[serde(default)]
    pub portfolio: Option<FinancePortfolio>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinanceTrendPoint {
    pub month: String,
    #[serde(default)]
    pub income_cents: i64,
    #[serde(default)]
    pub personal_spending_cents: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinancePortfolio {
    pub as_of: String,
    pub currency: String,
    pub total: FinanceDecimalValue,
    #[serde(default)]
    pub positions: Vec<FinancePosition>,
    #[serde(default)]
    pub caveats: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinancePosition {
    pub instrument: String,
    pub label: String,
    pub currency: String,
    pub value: FinanceDecimalValue,
    /// `market` or `review`.
    pub value_basis: String,
    #[serde(default)]
    pub market_price_source: Option<String>,
    #[serde(default)]
    pub market_price_observed_on: Option<String>,
    #[serde(default)]
    pub price_age_days: Option<i64>,
    /// `fresh`, `stale` or `none`.
    pub price_freshness: String,
    #[serde(default)]
    pub share_bp: i64,
    #[serde(default)]
    pub change_since_review_bp: Option<i64>,
    #[serde(default)]
    pub conversion: Option<FinancePriceConversion>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct FinancePriceConversion {
    pub quoted_price: FinanceDecimalValue,
    pub quoted_currency: String,
    pub rate: FinanceDecimalValue,
    pub rate_observed_on: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct PricePointRecord {
    pub valid_from: String,
    pub amount_cents: i64,
    pub currency: String,
    pub cycle: String,
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct StateChangeRecord {
    pub effective: String,
    pub state: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SubscriptionRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub value_rating: Option<i16>,
    #[serde(default)]
    pub prices: Vec<PricePointRecord>,
    #[serde(default)]
    pub states: Vec<StateChangeRecord>,
}

// ============================================================================
// Action and Mutation Requests
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReviewCandidateRequest {
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_account: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReconcileTransferRequest {
    pub counterpart_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReclassifyItem {
    pub id: String,
    pub account: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReclassifyBatchRequest {
    pub items: Vec<ReclassifyItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AllocateExpenseRequest {
    pub personal_cents: i64,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct LinkReimbursementRequest {
    pub expense_candidate_id: String,
}

//! One normalized projection for every finance view.
//!
//! Cards, trends, the transaction table and Sankey links are derived in this
//! module. Keeping the arithmetic together makes reconciliation testable and
//! prevents the UI from growing a second definition of income or spending.
//!
//! Budget targets stood here until 2026-08-28 (PRD Q50). `BudgetTarget`,
//! `BudgetRow`, the per-account variance loop and the two `Summary` budget fields
//! were fed from `budgets` in the private Finance config, and nothing ever set it:
//! the live config has no such key and `schemas/finance.json.example` ships it
//! empty. Every deployment therefore rendered an empty panel and a variance of
//! zero. Deleted rather than kept warm -- a projection nothing feeds reports a
//! number that looks measured and is not.

use crate::accounting::JournalTransaction;
use crate::allocation::{SpendingPurpose, SHARED_RECEIVABLE_ACCOUNT};
use crate::investment::{PortfolioValuation, ReviewedHoldingsSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Income,
    Expense,
    Transfer,
}

impl TransactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Income => "income",
            Self::Expense => "expense",
            Self::Transfer => "transfer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "income" => Some(Self::Income),
            "expense" => Some(Self::Expense),
            "transfer" => Some(Self::Transfer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionRow {
    pub id: String,
    pub date: String,
    pub description: String,
    pub kind: TransactionKind,
    pub account: String,
    pub category: String,
    /// Always positive. Direction lives in `kind` so callers cannot accidentally
    /// invert expenses twice.
    pub amount_cents: i64,
    pub currency: String,
    pub source_id: Option<String>,
    pub purpose: Option<SpendingPurpose>,
    pub trip_id: Option<String>,
    /// Cash or card-liability movement before any shared-cost split.
    pub cash_amount_cents: i64,
    /// Amount fronted for somebody else by this expense.
    pub shared_cents: i64,
    /// Source id of the shared expense settled by this transfer.
    pub reimbursement_for: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct AnalyticsFilter {
    pub start: Option<String>,
    pub end: Option<String>,
    pub account: Option<String>,
    pub category: Option<String>,
    pub currency: Option<String>,
    #[serde(default)]
    pub include_transfers: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub income_cents: i64,
    pub personal_spending_cents: i64,
    pub gross_cash_outflow_cents: i64,
    pub reimbursement_received_cents: i64,
    pub personal_result_cents: i64,
    pub external_cash_inflow_cents: i64,
    pub external_cash_movement_cents: i64,
    pub savings_rate_percent: Option<f64>,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedExpenseSummary {
    pub source_id: String,
    pub candidate_id: String,
    pub date: String,
    pub description: String,
    pub account: String,
    pub category: String,
    pub purpose: Option<SpendingPurpose>,
    pub trip_id: Option<String>,
    pub gross_cents: i64,
    pub personal_cents: i64,
    pub shared_cents: i64,
    pub reimbursed_cents: i64,
    pub outstanding_cents: i64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PurposeSpendingSummary {
    pub purpose: Option<SpendingPurpose>,
    pub personal_spending_cents: i64,
    pub expense_posting_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TripSpendingSummary {
    pub trip_id: String,
    pub personal_spending_cents: i64,
    pub gross_cash_outflow_cents: i64,
    pub reimbursed_cents: i64,
    pub outstanding_cents: i64,
    pub expense_posting_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendPoint {
    pub month: String,
    pub income_cents: i64,
    pub personal_spending_cents: i64,
    pub gross_cash_outflow_cents: i64,
    pub reimbursement_received_cents: i64,
    pub personal_result_cents: i64,
    pub external_cash_inflow_cents: i64,
    pub external_cash_movement_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalyticsQuality {
    pub expense_posting_count: usize,
    pub categorized_expense_posting_count: usize,
    pub personal_spending_cents: i64,
    pub categorized_personal_spending_cents: i64,
    pub categorization_count_percent: Option<f64>,
    pub categorization_value_percent: Option<f64>,
    pub first_transaction_date: Option<String>,
    pub latest_transaction_date: Option<String>,
    pub observed_months: usize,
    pub expected_months: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CategoryTrendPoint {
    pub month: String,
    pub category: String,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SankeyLink {
    pub month: String,
    pub source: String,
    pub target: String,
    pub amount_cents: i64,
    pub account: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DashboardProjection {
    pub summary: Summary,
    pub quality: AnalyticsQuality,
    pub trend: Vec<TrendPoint>,
    pub category_trend: Vec<CategoryTrendPoint>,
    pub transactions: Vec<TransactionRow>,
    pub sankey: Vec<SankeyLink>,
    pub accounts: Vec<String>,
    pub categories: Vec<String>,
    pub investment: Option<ReviewedHoldingsSnapshot>,
    pub portfolio_values: Vec<PortfolioValuation>,
    pub shared_expenses: Vec<SharedExpenseSummary>,
    pub purpose_spending: Vec<PurposeSpendingSummary>,
    pub trip_spending: Vec<TripSpendingSummary>,
}

pub fn project(transactions: &[JournalTransaction], currency: &str) -> Vec<TransactionRow> {
    let mut rows = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        let balance_accounts: Vec<_> = transaction
            .postings
            .iter()
            .filter(|posting| {
                posting.account.starts_with("assets:")
                    || posting.account.starts_with("liabilities:")
            })
            .collect();
        let account = balance_accounts
            .iter()
            .find(|posting| amount_cents(posting, currency).unwrap_or(0) != 0)
            .map(|posting| posting.account.as_str())
            .unwrap_or("assets:unknown");
        let cash_amount_cents = balance_accounts
            .iter()
            .filter_map(|posting| amount_cents(posting, currency))
            .find(|amount| *amount != 0)
            .and_then(|amount| amount.checked_abs())
            .unwrap_or(0);
        let purpose = transaction
            .tags
            .get("axon-purpose")
            .and_then(|value| SpendingPurpose::parse(value));
        let trip_id = transaction.tags.get("axon-trip-id").cloned();
        let tagged_shared_cents = transaction
            .tags
            .get("axon-shared-cents")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let reimbursement_for = transaction.tags.get("axon-reimbursement-for").cloned();
        let mut classified = false;
        let mut cash_recorded = false;
        for (posting_index, posting) in transaction.postings.iter().enumerate() {
            let Some(signed_cents) = amount_cents(posting, currency) else {
                continue;
            };
            let (kind, amount_cents) =
                if posting.account.starts_with("expenses:") && signed_cents > 0 {
                    (TransactionKind::Expense, signed_cents)
                } else if posting.account.starts_with("income:") && signed_cents < 0 {
                    (TransactionKind::Income, -signed_cents)
                } else {
                    continue;
                };
            classified = true;
            let row_cash_amount_cents = if cash_recorded {
                0
            } else {
                cash_recorded = true;
                cash_amount_cents
            };
            rows.push(TransactionRow {
                id: format!(
                    "transaction_{}_{}_{}",
                    transaction.index,
                    posting_index,
                    currency.to_ascii_lowercase()
                ),
                date: transaction.date.clone(),
                description: transaction.description.clone(),
                kind,
                account: account.to_string(),
                category: posting.account.clone(),
                amount_cents,
                currency: currency.to_string(),
                source_id: transaction.source_id.clone(),
                purpose,
                trip_id: trip_id.clone(),
                cash_amount_cents: row_cash_amount_cents,
                shared_cents: tagged_shared_cents,
                reimbursement_for: reimbursement_for.clone(),
            });
        }
        if !classified && balance_accounts.len() >= 2 {
            let source = balance_accounts
                .iter()
                .find(|posting| amount_cents(posting, currency).unwrap_or(0) < 0);
            let target = balance_accounts
                .iter()
                .find(|posting| amount_cents(posting, currency).unwrap_or(0) > 0);
            if let (Some(source), Some(target)) = (source, target) {
                let is_reimbursement =
                    reimbursement_for.is_some() && source.account == SHARED_RECEIVABLE_ACCOUNT;
                rows.push(TransactionRow {
                    id: format!(
                        "transaction_{}_transfer_{}",
                        transaction.index,
                        currency.to_ascii_lowercase()
                    ),
                    date: transaction.date.clone(),
                    description: transaction.description.clone(),
                    kind: TransactionKind::Transfer,
                    account: if is_reimbursement {
                        target.account.clone()
                    } else {
                        source.account.clone()
                    },
                    category: if is_reimbursement {
                        source.account.clone()
                    } else {
                        target.account.clone()
                    },
                    amount_cents: amount_cents(source, currency)
                        .unwrap_or(0)
                        .checked_abs()
                        .unwrap_or(i64::MAX),
                    currency: currency.to_string(),
                    source_id: transaction.source_id.clone(),
                    purpose,
                    trip_id,
                    cash_amount_cents,
                    shared_cents: 0,
                    reimbursement_for,
                });
            }
        }
    }
    rows
}

pub fn dashboard(rows: &[TransactionRow], filter: &AnalyticsFilter) -> DashboardProjection {
    let currency = filter.currency.as_deref().unwrap_or("EUR");
    let scoped_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.currency == currency)
        .filter(|row| filter.start.as_ref().is_none_or(|start| row.date >= *start))
        .filter(|row| filter.end.as_ref().is_none_or(|end| row.date <= *end))
        .filter(|row| {
            filter
                .account
                .as_ref()
                .is_none_or(|account| row.account == *account)
        })
        .filter(|row| {
            filter
                .category
                .as_ref()
                .is_none_or(|category| row.category.starts_with(category))
        })
        .cloned()
        .collect();
    let mut transactions: Vec<_> = scoped_rows
        .iter()
        .filter(|row| filter.include_transfers || row.kind != TransactionKind::Transfer)
        .cloned()
        .collect();
    transactions.sort_by(|left, right| right.date.cmp(&left.date).then(left.id.cmp(&right.id)));

    let income_cents = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Income)
        .map(|row| row.amount_cents)
        .sum();
    let personal_spending_cents = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense)
        .map(|row| row.amount_cents)
        .sum();
    let gross_cash_outflow_cents = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense)
        .map(|row| row.cash_amount_cents)
        .sum();
    let reimbursement_received_cents = scoped_rows
        .iter()
        .filter(|row| row.reimbursement_for.is_some())
        .map(|row| row.cash_amount_cents)
        .sum();
    let income_cash_cents: i64 = scoped_rows
        .iter()
        .filter(|row| row.kind == TransactionKind::Income)
        .map(|row| row.cash_amount_cents)
        .sum();
    let personal_result_cents = income_cents - personal_spending_cents;
    let external_cash_inflow_cents = income_cash_cents + reimbursement_received_cents;
    let external_cash_movement_cents = external_cash_inflow_cents - gross_cash_outflow_cents;

    let mut trend: BTreeMap<String, TrendPoint> = BTreeMap::new();
    let mut category_trend: BTreeMap<(String, String), i64> = BTreeMap::new();
    for row in &scoped_rows {
        if row.date.len() < 7 {
            continue;
        }
        let point = trend
            .entry(row.date[..7].to_string())
            .or_insert(TrendPoint {
                month: row.date[..7].to_string(),
                income_cents: 0,
                personal_spending_cents: 0,
                gross_cash_outflow_cents: 0,
                reimbursement_received_cents: 0,
                personal_result_cents: 0,
                external_cash_inflow_cents: 0,
                external_cash_movement_cents: 0,
            });
        match row.kind {
            TransactionKind::Income => {
                point.income_cents += row.amount_cents;
                point.external_cash_inflow_cents += row.cash_amount_cents;
            }
            TransactionKind::Expense => {
                point.personal_spending_cents += row.amount_cents;
                point.gross_cash_outflow_cents += row.cash_amount_cents;
            }
            TransactionKind::Transfer if row.reimbursement_for.is_some() => {
                point.reimbursement_received_cents += row.cash_amount_cents;
                point.external_cash_inflow_cents += row.cash_amount_cents;
            }
            TransactionKind::Transfer => {}
        }
        point.personal_result_cents = point.income_cents - point.personal_spending_cents;
        point.external_cash_movement_cents =
            point.external_cash_inflow_cents - point.gross_cash_outflow_cents;
        if row.kind == TransactionKind::Expense {
            *category_trend
                .entry((row.date[..7].to_string(), row.category.clone()))
                .or_default() += row.amount_cents;
        }
    }

    let mut sankey: BTreeMap<(String, String, String, String, String), i64> = BTreeMap::new();
    for row in &transactions {
        if row.date.len() < 7 {
            continue;
        }
        let (source, target) = match row.kind {
            TransactionKind::Income => (row.category.clone(), row.account.clone()),
            TransactionKind::Expense => (row.account.clone(), row.category.clone()),
            TransactionKind::Transfer => continue,
        };
        *sankey
            .entry((
                row.date[..7].to_string(),
                source,
                target,
                row.account.clone(),
                row.category.clone(),
            ))
            .or_default() += row.amount_cents;
    }

    let expense_rows: Vec<_> = transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense)
        .collect();
    let categorized_rows: Vec<_> = expense_rows
        .iter()
        .filter(|row| !is_uncategorized(&row.category))
        .collect();
    let categorized_personal_spending_cents = categorized_rows
        .iter()
        .map(|row| row.amount_cents)
        .sum::<i64>();
    let observed_months = scoped_rows
        .iter()
        .filter_map(|row| row.date.get(..7))
        .collect::<BTreeSet<_>>()
        .len();
    let quality = AnalyticsQuality {
        expense_posting_count: expense_rows.len(),
        categorized_expense_posting_count: categorized_rows.len(),
        personal_spending_cents,
        categorized_personal_spending_cents,
        categorization_count_percent: (!expense_rows.is_empty())
            .then_some(categorized_rows.len() as f64 / expense_rows.len() as f64 * 100.0),
        categorization_value_percent: (personal_spending_cents > 0).then_some(
            categorized_personal_spending_cents as f64 / personal_spending_cents as f64 * 100.0,
        ),
        first_transaction_date: scoped_rows.iter().map(|row| row.date.clone()).min(),
        latest_transaction_date: scoped_rows.iter().map(|row| row.date.clone()).max(),
        observed_months,
        expected_months: month_count(filter, &scoped_rows),
    };

    let shared_expenses = shared_expenses(&scoped_rows);
    let purpose_spending = [
        Some(SpendingPurpose::DayToDay),
        Some(SpendingPurpose::Trip),
        Some(SpendingPurpose::Work),
        Some(SpendingPurpose::Housing),
        Some(SpendingPurpose::Other),
        None,
    ]
    .into_iter()
    .filter_map(|purpose| {
        let matching: Vec<_> = transactions
            .iter()
            .filter(|row| row.kind == TransactionKind::Expense && row.purpose == purpose)
            .collect();
        (!matching.is_empty()).then(|| PurposeSpendingSummary {
            purpose,
            personal_spending_cents: matching.iter().map(|row| row.amount_cents).sum(),
            expense_posting_count: matching.len(),
        })
    })
    .collect();
    let trip_spending = fold_trip_spending(&transactions, &shared_expenses);
    let accounts: BTreeSet<_> = rows.iter().map(|row| row.account.clone()).collect();
    let categories: BTreeSet<_> = rows.iter().map(|row| row.category.clone()).collect();
    DashboardProjection {
        summary: Summary {
            income_cents,
            personal_spending_cents,
            gross_cash_outflow_cents,
            reimbursement_received_cents,
            personal_result_cents,
            external_cash_inflow_cents,
            external_cash_movement_cents,
            savings_rate_percent: (income_cents > 0)
                .then_some(personal_result_cents as f64 / income_cents as f64 * 100.0),
            currency: currency.into(),
        },
        quality,
        trend: trend.into_values().collect(),
        category_trend: category_trend
            .into_iter()
            .map(|((month, category), amount_cents)| CategoryTrendPoint {
                month,
                category,
                amount_cents,
            })
            .collect(),
        transactions,
        sankey: sankey
            .into_iter()
            .map(
                |((month, source, target, account, category), amount_cents)| SankeyLink {
                    month,
                    source,
                    target,
                    amount_cents,
                    account,
                    category,
                },
            )
            .collect(),
        accounts: accounts.into_iter().collect(),
        categories: categories.into_iter().collect(),
        investment: None,
        portfolio_values: Vec::new(),
        shared_expenses,
        purpose_spending,
        trip_spending,
    }
}

fn is_uncategorized(category: &str) -> bool {
    category
        .split(':')
        .any(|segment| segment == "uncategorized")
}

pub fn outstanding_shared_cents(
    rows: &[TransactionRow],
    expense_source_id: &str,
    currency: &str,
    excluding_reimbursement_source: Option<&str>,
) -> Option<i64> {
    let shared = rows
        .iter()
        .find(|row| {
            row.currency == currency
                && row.kind == TransactionKind::Expense
                && row.source_id.as_deref() == Some(expense_source_id)
                && row.shared_cents > 0
        })?
        .shared_cents;
    let reimbursed: i64 = rows
        .iter()
        .filter(|row| {
            row.currency == currency
                && row.reimbursement_for.as_deref() == Some(expense_source_id)
                && excluding_reimbursement_source
                    .is_none_or(|source| row.source_id.as_deref() != Some(source))
        })
        .map(|row| row.cash_amount_cents)
        .sum();
    Some(shared.saturating_sub(reimbursed))
}

/// The per-trip roll-up, folded once and reachable from two doors.
///
/// Private, and the two entry points are what is public: [`dashboard`] calls it
/// with the `transactions` and `shared_expenses` it has already derived, and
/// [`trip_spending`] derives the same two from raw rows. Two folds would be two
/// definitions of what a trip cost, which is exactly the drift this extraction
/// exists to remove.
///
/// The two halves read two different derived sets on purpose: the expense loop
/// walks `transactions` (currency-, window-, account- and category-filtered, with
/// transfers excluded) while `shared` comes from `shared_expenses(&scoped_rows)`,
/// which includes transfers -- a reimbursement inflow has to be visible to pair
/// against.
fn fold_trip_spending(
    transactions: &[TransactionRow],
    shared: &[SharedExpenseSummary],
) -> Vec<TripSpendingSummary> {
    let mut trip_spending: BTreeMap<String, TripSpendingSummary> = BTreeMap::new();
    for row in transactions
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense)
    {
        let Some(trip_id) = row.trip_id.clone() else {
            continue;
        };
        let summary = trip_spending
            .entry(trip_id.clone())
            .or_insert(TripSpendingSummary {
                trip_id,
                personal_spending_cents: 0,
                gross_cash_outflow_cents: 0,
                reimbursed_cents: 0,
                outstanding_cents: 0,
                expense_posting_count: 0,
            });
        summary.personal_spending_cents += row.amount_cents;
        summary.gross_cash_outflow_cents += row.cash_amount_cents;
        summary.expense_posting_count += 1;
    }
    for shared in shared {
        let Some(trip_id) = shared.trip_id.as_ref() else {
            continue;
        };
        let summary = trip_spending
            .entry(trip_id.clone())
            .or_insert(TripSpendingSummary {
                trip_id: trip_id.clone(),
                personal_spending_cents: 0,
                gross_cash_outflow_cents: 0,
                reimbursed_cents: 0,
                outstanding_cents: 0,
                expense_posting_count: 0,
            });
        summary.reimbursed_cents += shared.reimbursed_cents;
        summary.outstanding_cents += shared.outstanding_cents;
    }
    trip_spending.into_values().collect()
}

/// Every trip's actuals, derived from raw projection rows.
///
/// The narrow door `GET /api/trips/:id/spending` uses. It derives `scoped_rows`,
/// `transactions` and `shared_expenses` exactly as [`dashboard`] does, so the two
/// numbers are equal by construction rather than by coincidence.
///
/// It reads every row and that is necessary rather than lazy: a reimbursement
/// inflow does not carry the trip id, so `shared_expenses` needs the whole set to
/// pair against. The saving is on the wire -- one object instead of the entire
/// projection.
pub fn trip_spending(
    rows: &[TransactionRow],
    filter: &AnalyticsFilter,
) -> Vec<TripSpendingSummary> {
    let currency = filter.currency.as_deref().unwrap_or("EUR");
    let scoped_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.currency == currency)
        .filter(|row| filter.start.as_ref().is_none_or(|start| row.date >= *start))
        .filter(|row| filter.end.as_ref().is_none_or(|end| row.date <= *end))
        .filter(|row| {
            filter
                .account
                .as_ref()
                .is_none_or(|account| row.account == *account)
        })
        .filter(|row| {
            filter
                .category
                .as_ref()
                .is_none_or(|category| row.category.starts_with(category))
        })
        .cloned()
        .collect();
    let transactions: Vec<_> = scoped_rows
        .iter()
        .filter(|row| filter.include_transfers || row.kind != TransactionKind::Transfer)
        .cloned()
        .collect();
    let shared = shared_expenses(&scoped_rows);
    fold_trip_spending(&transactions, &shared)
}

fn shared_expenses(rows: &[TransactionRow]) -> Vec<SharedExpenseSummary> {
    let mut summaries: Vec<_> = rows
        .iter()
        .filter(|row| row.kind == TransactionKind::Expense && row.shared_cents > 0)
        .filter_map(|row| {
            let source_id = row.source_id.clone()?;
            let reimbursed_cents: i64 = rows
                .iter()
                .filter(|candidate| {
                    candidate.currency == row.currency
                        && candidate.reimbursement_for.as_deref() == Some(source_id.as_str())
                })
                .map(|candidate| candidate.cash_amount_cents)
                .sum();
            Some(SharedExpenseSummary {
                candidate_id: format!("candidate_{source_id}"),
                source_id,
                date: row.date.clone(),
                description: row.description.clone(),
                account: row.account.clone(),
                category: row.category.clone(),
                purpose: row.purpose,
                trip_id: row.trip_id.clone(),
                gross_cents: row.cash_amount_cents,
                personal_cents: row.amount_cents,
                shared_cents: row.shared_cents,
                reimbursed_cents,
                outstanding_cents: row.shared_cents.saturating_sub(reimbursed_cents),
                currency: row.currency.clone(),
            })
        })
        .collect();
    summaries.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then(left.source_id.cmp(&right.source_id))
    });
    summaries
}

fn amount_cents(posting: &crate::accounting::Posting, currency: &str) -> Option<i64> {
    posting
        .amounts
        .iter()
        .find(|amount| amount.commodity == currency)
        .and_then(|amount| amount.minor_units(2))
}

fn month_count(filter: &AnalyticsFilter, rows: &[TransactionRow]) -> usize {
    let start = filter
        .start
        .as_deref()
        .or_else(|| rows.iter().map(|row| row.date.as_str()).min());
    let end = filter
        .end
        .as_deref()
        .or_else(|| rows.iter().map(|row| row.date.as_str()).max());
    let (Some(start), Some(end)) = (start, end) else {
        return 1;
    };
    let parse = |date: &str| -> Option<i32> {
        Some(date.get(..4)?.parse::<i32>().ok()? * 12 + date.get(5..7)?.parse::<i32>().ok()?)
    };
    match (parse(start), parse(end)) {
        (Some(start), Some(end)) if end >= start => (end - start + 1) as usize,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{Amount, Posting};

    fn posting(account: &str, cents: i64) -> Posting {
        Posting {
            account: account.into(),
            amounts: vec![Amount {
                commodity: "EUR".into(),
                mantissa: cents,
                scale: 2,
            }],
        }
    }

    fn journal() -> Vec<JournalTransaction> {
        vec![
            JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "salary".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("assets:bank:checking", 200_000),
                    posting("income:salary", -200_000),
                ],
            },
            JournalTransaction {
                index: 2,
                date: "2026-08-02".into(),
                description: "market".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("expenses:food", 25_00),
                    posting("assets:bank:checking", -25_00),
                ],
            },
            JournalTransaction {
                index: 3,
                date: "2026-08-03".into(),
                description: "save".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("assets:bank:savings", 50_000),
                    posting("assets:bank:checking", -50_000),
                ],
            },
        ]
    }

    #[test]
    fn one_projection_drives_reconciling_metrics_and_sankey_links() {
        let rows = project(&journal(), "EUR");
        let view = dashboard(&rows, &AnalyticsFilter::default());
        assert_eq!(view.summary.income_cents, 200_000);
        assert_eq!(view.summary.personal_spending_cents, 25_00);
        assert_eq!(view.summary.personal_result_cents, 197_500);
        assert_eq!(view.summary.external_cash_movement_cents, 197_500);
        assert_eq!(view.transactions.len(), 2);
        assert_eq!(view.category_trend.len(), 1);
        assert_eq!(view.category_trend[0].month, "2026-08");
        assert_eq!(view.category_trend[0].category, "expenses:food");
        assert_eq!(view.category_trend[0].amount_cents, 25_00);
        assert_eq!(view.quality.categorization_value_percent, Some(100.0));
        assert_eq!(view.purpose_spending.len(), 1);
        assert_eq!(view.purpose_spending[0].purpose, None);
        assert_eq!(view.purpose_spending[0].personal_spending_cents, 25_00);
        assert_eq!(
            view.sankey
                .iter()
                .map(|link| link.amount_cents)
                .sum::<i64>(),
            202_500
        );
    }

    #[test]
    fn transfers_are_excluded_unless_the_filter_asks_for_them() {
        let rows = project(&journal(), "EUR");
        assert_eq!(
            dashboard(&rows, &AnalyticsFilter::default())
                .transactions
                .len(),
            2
        );
        let filter = AnalyticsFilter {
            include_transfers: true,
            ..AnalyticsFilter::default()
        };
        assert_eq!(dashboard(&rows, &filter).transactions.len(), 3);
    }

    #[test]
    fn card_purchases_are_expenses_and_settlements_are_transfers() {
        let transactions = vec![
            JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "synthetic purchase".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("liabilities:card:review", -12_00),
                    posting("expenses:services", 12_00),
                ],
            },
            JournalTransaction {
                index: 2,
                date: "2026-08-02".into(),
                description: "synthetic settlement".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("assets:bank:checking", -12_00),
                    posting("liabilities:card:review", 12_00),
                ],
            },
        ];
        let rows = project(&transactions, "EUR");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, TransactionKind::Expense);
        assert_eq!(rows[0].account, "liabilities:card:review");
        assert_eq!(rows[1].kind, TransactionKind::Transfer);
        let view = dashboard(&rows, &AnalyticsFilter::default());
        assert_eq!(view.summary.personal_spending_cents, 12_00);
        assert_eq!(view.transactions.len(), 1);
    }

    #[test]
    fn savings_rate_is_unavailable_without_positive_income() {
        let rows = vec![TransactionRow {
            id: "expense".into(),
            date: "2026-08-02".into(),
            description: "market".into(),
            kind: TransactionKind::Expense,
            account: "assets:bank:checking".into(),
            category: "expenses:food".into(),
            amount_cents: 100,
            currency: "EUR".into(),
            source_id: None,
            purpose: None,
            trip_id: None,
            cash_amount_cents: 100,
            shared_cents: 0,
            reimbursement_for: None,
        }];
        assert_eq!(
            dashboard(&rows, &AnalyticsFilter::default())
                .summary
                .savings_rate_percent,
            None
        );
    }

    /// The narrow route and the dashboard field must be one number.
    ///
    /// The fixture carries a transfer, a reimbursement and a foreign-currency row
    /// on purpose: those are the three shapes where the two derivations could
    /// disagree, and the point of one fold is that they cannot.
    #[test]
    fn the_narrow_trip_route_equals_the_dashboard_number() {
        let mut expense_tags = BTreeMap::new();
        expense_tags.insert("axon-purpose".into(), "trip".into());
        expense_tags.insert("axon-trip-id".into(), "trip:synthetic".into());
        expense_tags.insert("axon-shared-cents".into(), "3000".into());
        let mut reimbursement_tags = BTreeMap::new();
        reimbursement_tags.insert("axon-reimbursement-for".into(), "synthetic-expense".into());
        let transactions = vec![
            JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "synthetic group meal".into(),
                source_id: Some("synthetic-expense".into()),
                tags: expense_tags,
                postings: vec![
                    posting("assets:bank:checking", -40_00),
                    posting("expenses:food", 10_00),
                    posting(SHARED_RECEIVABLE_ACCOUNT, 30_00),
                ],
            },
            JournalTransaction {
                index: 2,
                date: "2026-08-02".into(),
                description: "synthetic reimbursement".into(),
                source_id: Some("synthetic-reimbursement".into()),
                tags: reimbursement_tags,
                postings: vec![
                    posting("assets:bank:checking", 20_00),
                    posting(SHARED_RECEIVABLE_ACCOUNT, -20_00),
                ],
            },
            JournalTransaction {
                index: 3,
                date: "2026-08-03".into(),
                description: "synthetic transfer".into(),
                source_id: Some("synthetic-transfer".into()),
                tags: BTreeMap::new(),
                postings: vec![
                    posting("assets:bank:checking", -50_00),
                    posting("assets:bank:savings", 50_00),
                ],
            },
        ];
        let mut rows = project(&transactions, "EUR");
        // A row in another currency, which the currency filter must drop on both
        // paths identically.
        let mut foreign = rows[0].clone();
        foreign.id = "synthetic-foreign".into();
        foreign.source_id = Some("synthetic-foreign".into());
        foreign.currency = "USD".into();
        rows.push(foreign);

        let filter = AnalyticsFilter {
            currency: Some("EUR".into()),
            ..AnalyticsFilter::default()
        };
        assert_eq!(
            dashboard(&rows, &filter).trip_spending,
            trip_spending(&rows, &filter)
        );
        assert_eq!(trip_spending(&rows, &filter).len(), 1);
        assert_eq!(trip_spending(&rows, &filter)[0].trip_id, "trip:synthetic");
    }

    #[test]
    fn shared_costs_separate_personal_spending_from_cash_and_reimbursements() {
        let mut expense_tags = BTreeMap::new();
        expense_tags.insert("axon-purpose".into(), "trip".into());
        expense_tags.insert("axon-trip-id".into(), "trip:synthetic".into());
        expense_tags.insert("axon-shared-cents".into(), "3000".into());
        let mut reimbursement_tags = BTreeMap::new();
        reimbursement_tags.insert("axon-reimbursement-for".into(), "synthetic-expense".into());
        let transactions = vec![
            JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "synthetic group meal".into(),
                source_id: Some("synthetic-expense".into()),
                tags: expense_tags,
                postings: vec![
                    posting("assets:bank:checking", -40_00),
                    posting("expenses:food", 10_00),
                    posting(SHARED_RECEIVABLE_ACCOUNT, 30_00),
                ],
            },
            JournalTransaction {
                index: 2,
                date: "2026-08-02".into(),
                description: "synthetic reimbursement".into(),
                source_id: Some("synthetic-reimbursement".into()),
                tags: reimbursement_tags,
                postings: vec![
                    posting("assets:bank:checking", 20_00),
                    posting(SHARED_RECEIVABLE_ACCOUNT, -20_00),
                ],
            },
        ];

        let rows = project(&transactions, "EUR");
        let view = dashboard(&rows, &AnalyticsFilter::default());
        assert_eq!(view.summary.personal_spending_cents, 10_00);
        assert_eq!(view.summary.gross_cash_outflow_cents, 40_00);
        assert_eq!(view.summary.reimbursement_received_cents, 20_00);
        assert_eq!(view.summary.personal_result_cents, -10_00);
        assert_eq!(view.summary.external_cash_movement_cents, -20_00);
        assert_eq!(view.summary.income_cents, 0);
        assert_eq!(view.shared_expenses.len(), 1);
        assert_eq!(view.shared_expenses[0].shared_cents, 30_00);
        assert_eq!(view.shared_expenses[0].outstanding_cents, 10_00);
        assert_eq!(view.purpose_spending.len(), 1);
        assert_eq!(
            view.purpose_spending[0].purpose,
            Some(SpendingPurpose::Trip)
        );
        assert_eq!(view.purpose_spending[0].personal_spending_cents, 10_00);
        assert_eq!(view.trip_spending.len(), 1);
        assert_eq!(view.trip_spending[0].trip_id, "trip:synthetic");
        assert_eq!(view.trip_spending[0].gross_cash_outflow_cents, 40_00);
        assert_eq!(view.trip_spending[0].reimbursed_cents, 20_00);
        assert_eq!(view.trip_spending[0].outstanding_cents, 10_00);
        assert_eq!(
            outstanding_shared_cents(&rows, "synthetic-expense", "EUR", None),
            Some(10_00)
        );
    }

    #[test]
    fn split_categories_record_external_cash_once() {
        let rows = project(
            &[JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "synthetic split purchase".into(),
                source_id: None,
                tags: BTreeMap::new(),
                postings: vec![
                    posting("assets:bank:checking", -10_00),
                    posting("expenses:food:groceries", 6_00),
                    posting("expenses:household", 4_00),
                ],
            }],
            "EUR",
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().map(|row| row.cash_amount_cents).sum::<i64>(),
            10_00
        );
        let view = dashboard(&rows, &AnalyticsFilter::default());
        assert_eq!(view.summary.personal_spending_cents, 10_00);
        assert_eq!(view.summary.gross_cash_outflow_cents, 10_00);
        assert_eq!(view.summary.external_cash_movement_cents, -10_00);
    }

    #[test]
    fn quality_reports_categorization_and_period_coverage() {
        let rows = vec![
            TransactionRow {
                id: "categorized".into(),
                date: "2026-07-02".into(),
                description: "synthetic categorized".into(),
                kind: TransactionKind::Expense,
                account: "assets:bank:checking".into(),
                category: "expenses:food".into(),
                amount_cents: 75,
                currency: "EUR".into(),
                source_id: None,
                purpose: None,
                trip_id: None,
                cash_amount_cents: 75,
                shared_cents: 0,
                reimbursement_for: None,
            },
            TransactionRow {
                id: "uncategorized".into(),
                date: "2026-08-02".into(),
                description: "synthetic uncategorized".into(),
                kind: TransactionKind::Expense,
                account: "assets:bank:checking".into(),
                category: "expenses:uncategorized".into(),
                amount_cents: 25,
                currency: "EUR".into(),
                source_id: None,
                purpose: None,
                trip_id: None,
                cash_amount_cents: 25,
                shared_cents: 0,
                reimbursement_for: None,
            },
        ];
        let filter = AnalyticsFilter {
            start: Some("2026-07-01".into()),
            end: Some("2026-08-31".into()),
            ..AnalyticsFilter::default()
        };

        let view = dashboard(&rows, &filter);
        assert_eq!(view.quality.expense_posting_count, 2);
        assert_eq!(view.quality.categorized_expense_posting_count, 1);
        assert_eq!(view.quality.categorization_count_percent, Some(50.0));
        assert_eq!(view.quality.categorization_value_percent, Some(75.0));
        assert_eq!(view.quality.observed_months, 2);
        assert_eq!(view.quality.expected_months, 2);
        assert_eq!(
            view.quality.first_transaction_date.as_deref(),
            Some("2026-07-02")
        );
        assert_eq!(
            view.quality.latest_transaction_date.as_deref(),
            Some("2026-08-02")
        );
    }
}

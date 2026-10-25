//! Core domain logic for the Finance surface.
//!
//! Provides models, calculations, and filtering for financial transactions,
//! account balances, and recurring subscriptions.

use serde::{Deserialize, Serialize};

use crate::shell::{FinanceTransaction, SubscriptionRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewQueueMode {
    Unreviewed,
    Shared,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpenseReview {
    pub candidate: crate::shell::FinanceCandidate,
    pub transaction: Option<FinanceTransaction>,
    pub total_cents: i64,
    pub reviewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewScope {
    pub start: String,
    pub end: String,
    pub account: String,
    pub category: String,
}

/// Confirmed, categorized outflows eligible for the spending context queue.
pub fn reviewable_expenses(
    candidates: &[crate::shell::FinanceCandidate],
    transactions: &[FinanceTransaction],
    scope: &ReviewScope,
) -> Vec<ExpenseReview> {
    let transactions_by_source: std::collections::HashMap<_, _> = transactions
        .iter()
        .filter_map(|transaction| {
            transaction
                .source_id
                .as_deref()
                .map(|source_id| (source_id, transaction))
        })
        .collect();

    candidates
        .iter()
        .filter(|candidate| {
            candidate.state == "confirmed"
                && candidate.amount_cents < 0
                && candidate.booked_at >= scope.start
                && candidate.booked_at <= scope.end
                && (scope.account.is_empty() || candidate.source_account == scope.account)
                && (scope.category.is_empty() || candidate.proposed_account == scope.category)
                && candidate.proposed_account.starts_with("expenses:")
                && !candidate
                    .proposed_account
                    .split(':')
                    .any(|segment| segment == "uncategorized")
        })
        .map(|candidate| {
            let transaction = transactions_by_source
                .get(candidate.fingerprint.as_str())
                .map(|row| (*row).clone());
            let reviewed = transaction
                .as_ref()
                .is_some_and(|row| row.purpose.is_some());
            ExpenseReview {
                candidate: candidate.clone(),
                transaction,
                total_cents: candidate.amount_cents.saturating_abs(),
                reviewed,
            }
        })
        .collect()
}

/// Apply the optional trip window and review mode, then prioritize open and larger expenses.
pub fn prioritized_expense_queue(
    entries: &[ExpenseReview],
    mode: ReviewQueueMode,
    plan: Option<(&str, &str)>,
) -> Vec<ExpenseReview> {
    let mut queue: Vec<_> = entries
        .iter()
        .filter(|entry| {
            plan.is_none_or(|(start, end)| {
                entry.candidate.booked_at.as_str() >= start
                    && entry.candidate.booked_at.as_str() <= end
            })
        })
        .filter(|entry| match mode {
            ReviewQueueMode::All => true,
            ReviewQueueMode::Unreviewed => !entry.reviewed,
            ReviewQueueMode::Shared => entry
                .transaction
                .as_ref()
                .is_some_and(|transaction| transaction.shared_cents > 0),
        })
        .cloned()
        .collect();
    queue.sort_by(|left, right| {
        left.reviewed
            .cmp(&right.reviewed)
            .then_with(|| right.total_cents.cmp(&left.total_cents))
            .then_with(|| right.candidate.booked_at.cmp(&left.candidate.booked_at))
    });
    queue
}

pub fn parse_personal_cents(value: &str) -> Option<i64> {
    let normalized = value.trim().replace(',', ".");
    let (whole, fraction) = normalized.split_once('.').unwrap_or((&normalized, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 2
        || (normalized.contains('.') && fraction.is_empty())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?;
    let cents = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()?.checked_mul(10)?,
        _ => fraction.parse::<i64>().ok()?,
    };
    let cents = whole.checked_mul(100)?.checked_add(cents)?;
    (cents <= 9_007_199_254_740_991).then_some(cents)
}

pub fn preset_share_cents(total_cents: i64, percent: f64) -> i64 {
    ((total_cents as f64 * percent / 100.0).round()) as i64
}

/// Format the exact decimal value used by Finance net worth without converting through `f64`.
pub fn format_exact_money(mantissa: &str, scale: u32, currency: &str) -> Option<String> {
    let negative = mantissa.starts_with('-');
    let digits = mantissa
        .strip_prefix('-')
        .unwrap_or(mantissa)
        .parse::<i128>()
        .ok()?;
    let divisor = if scale > 2 {
        10_i128.checked_pow(scale - 2)?
    } else {
        1
    };
    let scaled = if scale > 2 {
        digits.checked_add(divisor / 2)?.checked_div(divisor)?
    } else {
        digits.checked_mul(10_i128.checked_pow(2 - scale)?)?
    };
    let whole = (scaled / 100).to_string();
    let grouped = whole
        .chars()
        .rev()
        .enumerate()
        .fold(String::new(), |mut output, (index, digit)| {
            if index > 0 && index % 3 == 0 {
                output.push('.');
            }
            output.push(digit);
            output
        })
        .chars()
        .rev()
        .collect::<String>();
    let sign = if negative && scaled != 0 { "-" } else { "" };
    Some(format!("{sign}{grouped},{:02} {currency}", scaled % 100))
}

/// Normalize a CSV profile with the same import defaults used by the retired web form.
pub fn selected_csv_mapping(mapping: &serde_json::Value) -> serde_json::Value {
    let mut selected = mapping.clone();
    let Some(object) = selected.as_object_mut() else {
        return selected;
    };
    object
        .entry("amount_sign")
        .or_insert_with(|| serde_json::json!("as_provided"));
    object
        .entry("amount_rounding")
        .or_insert_with(|| serde_json::json!("reject"));
    object
        .entry("default_outflow_account")
        .or_insert_with(|| serde_json::json!("expenses:uncategorized"));
    object
        .entry("default_inflow_account")
        .or_insert_with(|| serde_json::json!("income:uncategorized"));
    object
        .entry("date_formats")
        .or_insert_with(|| serde_json::json!(["iso_year_month_day", "day_month_year_dots"]));
    object
        .entry("row_policy")
        .or_insert_with(|| serde_json::json!("strict"));
    object
        .entry("categorization_columns")
        .or_insert_with(|| serde_json::json!([]));
    object
        .entry("categorization_rules")
        .or_insert_with(|| serde_json::json!([]));
    selected
}

/// Filter options for transaction search.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionFilter {
    pub query: String,
    pub category: Option<String>,
}

/// A parsed subscription ready for UI display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionSummary {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: String,
    pub cycle: String,
    pub amount_cents: i64,
    pub monthly_equivalent_cents: i64,
    pub currency: String,
}

impl SubscriptionSummary {
    /// Extracts the active price and state from a subscription record.
    pub fn from_record(record: &SubscriptionRecord) -> Self {
        let latest_price = record.prices.last();
        let latest_state = record.states.last();

        let status = latest_state
            .map(|s| s.state.to_ascii_lowercase())
            .unwrap_or_else(|| "active".to_string());

        let (amount_cents, currency, cycle) = latest_price
            .map(|p| {
                (
                    p.amount_cents,
                    p.currency.clone(),
                    p.cycle.to_ascii_lowercase(),
                )
            })
            .unwrap_or_else(|| (0, "EUR".to_string(), "monthly".to_string()));

        let monthly_equivalent_cents = match cycle.as_str() {
            "annual" | "yearly" | "year" => amount_cents / 12,
            "quarterly" => amount_cents / 3,
            "weekly" => (amount_cents * 52) / 12,
            _ => amount_cents,
        };

        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            category: record
                .category
                .clone()
                .unwrap_or_else(|| "General".to_string()),
            status,
            cycle,
            amount_cents,
            monthly_equivalent_cents,
            currency,
        }
    }
}

/// Calculates the total monthly recurring burn from active subscriptions in cents.
pub fn calculate_monthly_burn(subscriptions: &[SubscriptionRecord]) -> i64 {
    subscriptions
        .iter()
        .map(SubscriptionSummary::from_record)
        .filter(|s| s.status == "active" || s.status == "trial")
        .map(|s| s.monthly_equivalent_cents)
        .sum()
}

/// Filters transactions by search query and category.
pub fn filter_transactions<'a>(
    transactions: &'a [FinanceTransaction],
    filter: &TransactionFilter,
) -> Vec<&'a FinanceTransaction> {
    let q = filter.query.trim().to_ascii_lowercase();
    transactions
        .iter()
        .filter(|tx| {
            if let Some(cat) = &filter.category {
                if !cat.is_empty() && !tx.category.eq_ignore_ascii_case(cat) {
                    return false;
                }
            }
            if q.is_empty() {
                return true;
            }
            tx.description.to_ascii_lowercase().contains(&q)
                || tx.category.to_ascii_lowercase().contains(&q)
                || tx.account.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{PricePointRecord, StateChangeRecord};

    #[test]
    fn calculates_monthly_burn_with_mixed_cycles() {
        let subs = vec![
            SubscriptionRecord {
                id: "sub-1".into(),
                name: "Spotify".into(),
                category: Some("Music".into()),
                value_rating: None,
                prices: vec![PricePointRecord {
                    valid_from: "2026-01-01".into(),
                    amount_cents: 1099,
                    currency: "EUR".into(),
                    cycle: "monthly".into(),
                    plan: None,
                }],
                states: vec![StateChangeRecord {
                    effective: "2026-01-01".into(),
                    state: "active".into(),
                    note: String::new(),
                }],
            },
            SubscriptionRecord {
                id: "sub-2".into(),
                name: "Domain".into(),
                category: Some("Tech".into()),
                value_rating: None,
                prices: vec![PricePointRecord {
                    valid_from: "2026-01-01".into(),
                    amount_cents: 12000,
                    currency: "EUR".into(),
                    cycle: "yearly".into(),
                    plan: None,
                }],
                states: vec![StateChangeRecord {
                    effective: "2026-01-01".into(),
                    state: "active".into(),
                    note: String::new(),
                }],
            },
            SubscriptionRecord {
                id: "sub-3".into(),
                name: "Old Gym".into(),
                category: Some("Fitness".into()),
                value_rating: None,
                prices: vec![PricePointRecord {
                    valid_from: "2026-01-01".into(),
                    amount_cents: 5000,
                    currency: "EUR".into(),
                    cycle: "monthly".into(),
                    plan: None,
                }],
                states: vec![StateChangeRecord {
                    effective: "2026-01-01".into(),
                    state: "cancelled".into(),
                    note: String::new(),
                }],
            },
        ];

        // Spotify: 1099/mo + Domain: 12000/12 = 1000/mo. Old Gym is cancelled -> total 2099.
        let burn = calculate_monthly_burn(&subs);
        assert_eq!(burn, 2099);
    }

    #[test]
    fn filters_transactions_by_query_and_category() {
        let txs = vec![
            FinanceTransaction {
                id: "tx-1".into(),
                date: "2026-10-22".into(),
                description: "Supermarket Rewe".into(),
                kind: "expense".into(),
                account: "Giro".into(),
                category: "Food".into(),
                amount_cents: -3550,
                currency: "EUR".into(),
                source_id: None,
                trip_id: None,
                purpose: None,
                shared_cents: 0,
                cash_amount_cents: 0,
                reimbursement_for: None,
            },
            FinanceTransaction {
                id: "tx-2".into(),
                date: "2026-10-21".into(),
                description: "Coffee Shop".into(),
                kind: "expense".into(),
                account: "Visa".into(),
                category: "Food".into(),
                amount_cents: -450,
                currency: "EUR".into(),
                source_id: None,
                trip_id: None,
                purpose: None,
                shared_cents: 0,
                cash_amount_cents: 0,
                reimbursement_for: None,
            },
            FinanceTransaction {
                id: "tx-3".into(),
                date: "2026-10-20".into(),
                description: "Train Ticket".into(),
                kind: "expense".into(),
                account: "Visa".into(),
                category: "Transport".into(),
                amount_cents: -2900,
                currency: "EUR".into(),
                source_id: None,
                trip_id: None,
                purpose: None,
                shared_cents: 0,
                cash_amount_cents: 0,
                reimbursement_for: None,
            },
        ];

        let f1 = TransactionFilter {
            query: "coffee".into(),
            category: None,
        };
        let res1 = filter_transactions(&txs, &f1);
        assert_eq!(res1.len(), 1);
        assert_eq!(res1[0].id, "tx-2");

        let f2 = TransactionFilter {
            query: String::new(),
            category: Some("Food".into()),
        };
        let res2 = filter_transactions(&txs, &f2);
        assert_eq!(res2.len(), 2);
    }
}

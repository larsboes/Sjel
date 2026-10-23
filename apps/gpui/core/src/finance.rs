//! Core domain logic for the Finance surface.
//!
//! Provides models, calculations, and filtering for financial transactions,
//! account balances, and recurring subscriptions.

use serde::{Deserialize, Serialize};

use crate::shell::{FinanceTransaction, SubscriptionRecord};

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

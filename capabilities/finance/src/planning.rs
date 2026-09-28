//! Decision projections built from reviewed Finance state.
//!
//! The journal remains the record of what happened. This module answers the
//! forward-looking questions that should not be encoded as journal entries:
//! a representative monthly baseline, dated commitment changes, liquidity
//! runway, subscription portfolio anomalies, and card/reward break-even math.
//! Every personal assumption arrives through the private overlay config.

use crate::allocation::SpendingPurpose;
use crate::analytics::{TransactionKind, TransactionRow};
use crate::balance::{BalanceCoverage, BalanceKind, ManualBalanceSnapshot};
use crate::config::RecurringCommitment;
use crate::investment::{HoldingsCoverage, PortfolioValuation, ReviewedHoldingsSnapshot};
use crate::subscription::{burn_by_currency, State, Subscription};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

fn default_baseline_months() -> usize {
    3
}

fn default_runway_months() -> u32 {
    6
}

fn default_include_subscription_burn() -> bool {
    true
}

fn default_journal_freshness_days() -> u32 {
    45
}

fn default_snapshot_freshness_days() -> u32 {
    14
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseBehavior {
    Fixed,
    Variable,
    Discretionary,
    Exceptional,
    #[default]
    Unclassified,
}

impl ExpenseBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Variable => "variable",
            Self::Discretionary => "discretionary",
            Self::Exceptional => "exceptional",
            Self::Unclassified => "unclassified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingRule {
    pub account_prefix: String,
    pub behavior: ExpenseBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastAdjustment {
    pub id: String,
    pub label: String,
    pub monthly_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub valid_from: String,
    #[serde(default)]
    pub valid_until: Option<String>,
}

impl ForecastAdjustment {
    fn active_on(&self, date: &str) -> bool {
        self.valid_from.as_str() <= date
            && self
                .valid_until
                .as_deref()
                .is_none_or(|valid_until| date <= valid_until)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardBenefitInput {
    pub id: String,
    pub label: String,
    pub annual_face_value_cents: i64,
    pub annual_personal_value_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardOptionInput {
    pub id: String,
    pub label: String,
    pub annual_fee_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    /// 1000 means one point per currency unit of eligible spend.
    #[serde(default)]
    pub points_per_currency_unit_milli: i64,
    /// 1000 means one cent of assumed value per point.
    #[serde(default)]
    pub point_value_milli_cents: i64,
    #[serde(default)]
    pub point_value_assumption: String,
    #[serde(default)]
    pub fx_fee_basis_points: i64,
    #[serde(default)]
    pub benefits: Vec<CardBenefitInput>,
    pub terms_checked_on: String,
    #[serde(default)]
    pub source_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardDecisionInput {
    /// Manual fallback when no reviewed journal account prefixes are supplied.
    pub annual_eligible_spend_cents: i64,
    #[serde(default)]
    pub annual_fx_spend_cents: i64,
    #[serde(default)]
    pub usage_reviewed: bool,
    #[serde(default)]
    pub eligible_account_prefixes: Vec<String>,
    #[serde(default)]
    pub ineligible_category_prefixes: Vec<String>,
    #[serde(default)]
    pub options: Vec<CardOptionInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoyaltyBalanceInput {
    pub id: String,
    pub label: String,
    pub points: i64,
    /// 1000 means one cent of assumed value per point.
    pub point_value_milli_cents: i64,
    #[serde(default)]
    pub transferable: bool,
    #[serde(default)]
    pub assumption: String,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub expires_on: Option<String>,
    #[serde(default)]
    pub transfer_path: Option<String>,
    #[serde(default)]
    pub source_urls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedCoverage {
    Complete,
    #[default]
    Partial,
}

impl ExpectedCoverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceExpectation {
    Transactions {
        id: String,
        label: String,
        account_prefixes: Vec<String>,
        #[serde(default)]
        freshness_days: Option<u32>,
        #[serde(default)]
        coverage: ExpectedCoverage,
    },
    Holdings {
        id: String,
        label: String,
        source_key: String,
        #[serde(default)]
        freshness_days: Option<u32>,
        #[serde(default)]
        coverage: ExpectedCoverage,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningConfig {
    #[serde(default = "default_baseline_months")]
    pub baseline_months: usize,
    #[serde(default = "default_runway_months")]
    pub runway_target_months: u32,
    #[serde(default = "default_journal_freshness_days")]
    pub journal_freshness_days: u32,
    #[serde(default = "default_snapshot_freshness_days")]
    pub snapshot_freshness_days: u32,
    #[serde(default)]
    pub spending_rules: Vec<SpendingRule>,
    /// Expense prefixes whose historical rows are replaced by the subscription
    /// series in forecasts, avoiding a recurring-cost double count.
    #[serde(default)]
    pub subscription_account_prefixes: Vec<String>,
    #[serde(default = "default_include_subscription_burn")]
    pub include_subscription_burn: bool,
    #[serde(default)]
    pub forecast_adjustments: Vec<ForecastAdjustment>,
    #[serde(default)]
    pub comparison_dates: Vec<String>,
    #[serde(default)]
    pub card_decision: Option<CardDecisionInput>,
    #[serde(default)]
    pub loyalty_balances: Vec<LoyaltyBalanceInput>,
    #[serde(default)]
    pub source_expectations: Vec<SourceExpectation>,
}

impl Default for PlanningConfig {
    fn default() -> Self {
        Self {
            baseline_months: default_baseline_months(),
            runway_target_months: default_runway_months(),
            journal_freshness_days: default_journal_freshness_days(),
            snapshot_freshness_days: default_snapshot_freshness_days(),
            spending_rules: Vec::new(),
            subscription_account_prefixes: Vec::new(),
            include_subscription_burn: true,
            forecast_adjustments: Vec::new(),
            comparison_dates: Vec::new(),
            card_decision: None,
            loyalty_balances: Vec::new(),
            source_expectations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonthlyBaseline {
    pub months: Vec<String>,
    pub monthly_income_cents: i64,
    pub monthly_spending_cents: i64,
    /// Historical monthly spend retained in forecasts after exceptional and
    /// explicitly replaced recurring categories are removed.
    pub forecast_base_cents: i64,
    pub monthly_result_cents: i64,
    pub savings_rate_percent: Option<f64>,
    pub behavior: Vec<BehaviorAmount>,
    pub classified_value_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviorAmount {
    pub behavior: String,
    pub monthly_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForecastPoint {
    pub as_of: String,
    pub historical_base_cents: i64,
    pub commitments_cents: i64,
    pub subscriptions_cents: i64,
    pub adjustments_cents: i64,
    pub projected_spending_cents: i64,
    pub projected_result_cents: i64,
    pub savings_rate_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiquidityInsight {
    pub currency: String,
    pub liquid_assets_cents: i64,
    pub liabilities_cents: i64,
    pub invested_cents: Option<i64>,
    pub net_worth_cents: Option<i64>,
    pub cash_share_percent: Option<f64>,
    pub largest_priced_holding_percent: Option<f64>,
    pub runway_months: Option<f64>,
    pub target_cash_cents: i64,
    pub cash_buffer_cents: i64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionAnomaly {
    pub subscription_id: String,
    pub subscription_name: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionPortfolio {
    pub monthly_cents: i64,
    pub annual_cents: i64,
    pub billing_count: usize,
    pub covered_count: usize,
    pub unknown_price_count: usize,
    pub anomalies: Vec<SubscriptionAnomaly>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardOptionResult {
    pub id: String,
    pub label: String,
    pub currency: String,
    pub annual_fee_cents: i64,
    pub annual_face_value_cents: i64,
    pub annual_benefit_value_cents: i64,
    pub annual_unvalued_face_value_cents: i64,
    pub annual_reward_value_cents: i64,
    pub annual_fx_cost_cents: i64,
    pub annual_net_value_cents: i64,
    pub break_even_eligible_spend_cents: Option<i64>,
    pub points_per_currency_unit_milli: i64,
    pub point_value_milli_cents: i64,
    pub point_value_assumption: String,
    pub terms_checked_on: String,
    pub source_urls: Vec<String>,
    pub benefits: Vec<CardBenefitResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardBenefitResult {
    pub id: String,
    pub label: String,
    pub annual_face_value_cents: i64,
    pub annual_personal_value_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardDecisionResult {
    pub annual_eligible_spend_cents: i64,
    pub annual_fx_spend_cents: i64,
    pub usage_reviewed: bool,
    pub spend_source: String,
    pub spend_period_start: Option<String>,
    pub spend_period_end: Option<String>,
    pub options: Vec<CardOptionResult>,
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoyaltyValuation {
    pub id: String,
    pub label: String,
    pub points: i64,
    pub point_value_milli_cents: i64,
    pub estimated_value_cents: i64,
    pub transferable: bool,
    pub assumption: String,
    pub as_of: Option<String>,
    pub expires_on: Option<String>,
    pub transfer_path: Option<String>,
    pub source_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanningReport {
    pub as_of: String,
    pub currency: String,
    pub baseline: MonthlyBaseline,
    pub forecasts: Vec<ForecastPoint>,
    pub liquidity: Option<LiquidityInsight>,
    pub subscriptions: SubscriptionPortfolio,
    pub card_decision: Option<CardDecisionResult>,
    pub loyalty: Vec<LoyaltyValuation>,
    pub caveats: Vec<String>,
}

pub struct PlanningInputs<'a> {
    pub rows: &'a [TransactionRow],
    pub commitments: &'a [RecurringCommitment],
    pub subscriptions: &'a [Subscription],
    pub balance_snapshot: Option<&'a ManualBalanceSnapshot>,
    pub investment_snapshot: Option<&'a ReviewedHoldingsSnapshot>,
    pub portfolio_values: &'a [PortfolioValuation],
    pub config: &'a PlanningConfig,
    pub as_of: &'a str,
    pub currency: &'a str,
}

pub fn report(inputs: PlanningInputs<'_>) -> PlanningReport {
    let baseline = baseline(
        inputs.rows,
        inputs.commitments,
        inputs.config,
        inputs.as_of,
        inputs.currency,
    );
    let forecasts = forecasts(
        &baseline,
        inputs.commitments,
        inputs.subscriptions,
        inputs.config,
        inputs.as_of,
        inputs.currency,
    );
    let current_spending = forecasts
        .first()
        .map(|point| point.projected_spending_cents)
        .unwrap_or(baseline.monthly_spending_cents);
    let liquidity = liquidity(
        inputs.balance_snapshot,
        inputs.investment_snapshot,
        inputs.portfolio_values,
        current_spending,
        inputs.config.runway_target_months,
        inputs.currency,
    );
    let subscriptions = subscription_portfolio(inputs.subscriptions, inputs.as_of, inputs.currency);
    let card_decision = inputs.config.card_decision.as_ref().map(|input| {
        let derived = eligible_card_spend(inputs.rows, input, inputs.as_of, inputs.currency);
        card_decision(input, derived, inputs.currency)
    });
    let loyalty = inputs
        .config
        .loyalty_balances
        .iter()
        .map(|balance| LoyaltyValuation {
            id: balance.id.clone(),
            label: balance.label.clone(),
            points: balance.points,
            point_value_milli_cents: balance.point_value_milli_cents,
            estimated_value_cents: balance
                .points
                .saturating_mul(balance.point_value_milli_cents)
                / 1000,
            transferable: balance.transferable,
            assumption: balance.assumption.clone(),
            as_of: balance.as_of.clone(),
            expires_on: balance.expires_on.clone(),
            transfer_path: balance.transfer_path.clone(),
            source_urls: balance.source_urls.clone(),
        })
        .collect();
    let mut caveats = Vec::new();
    if baseline.months.len() < inputs.config.baseline_months.max(1) {
        caveats.push("The baseline uses fewer complete months than requested.".into());
    }
    if baseline.classified_value_percent.unwrap_or(0.0) < 80.0 {
        caveats.push(
            "Less than 80% of baseline spending has a fixed, variable, or discretionary rule."
                .into(),
        );
    }
    if inputs.config.include_subscription_burn
        && inputs.config.subscription_account_prefixes.is_empty()
        && subscriptions.monthly_cents > 0
    {
        caveats.push("Subscription burn is included, but no historical subscription account prefixes are configured; check for double counting.".into());
    }
    if inputs.balance_snapshot.is_none() {
        caveats.push("Liquidity and runway need a reviewed balance snapshot.".into());
    }
    if inputs.investment_snapshot.is_none() {
        caveats.push(
            "Investment allocation is missing until a reviewed holdings snapshot exists.".into(),
        );
    }
    if card_decision
        .as_ref()
        .is_some_and(|decision| decision.provisional)
    {
        caveats.push("Card results are provisional until usage and every option's source evidence are reviewed.".into());
    }
    if inputs.config.loyalty_balances.is_empty() {
        caveats.push(
            "Loyalty value is pending until private point balances and valuation assumptions are recorded."
                .into(),
        );
    }
    PlanningReport {
        as_of: inputs.as_of.into(),
        currency: inputs.currency.into(),
        baseline,
        forecasts,
        liquidity,
        subscriptions,
        card_decision,
        loyalty,
        caveats,
    }
}

fn baseline(
    rows: &[TransactionRow],
    commitments: &[RecurringCommitment],
    config: &PlanningConfig,
    as_of: &str,
    currency: &str,
) -> MonthlyBaseline {
    let current_month = as_of.get(..7).unwrap_or(as_of);
    let mut available: Vec<String> = rows
        .iter()
        .filter(|row| row.currency == currency)
        .filter_map(|row| row.date.get(..7))
        .filter(|month| *month < current_month)
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if available.is_empty() {
        available = rows
            .iter()
            .filter(|row| row.currency == currency)
            .filter_map(|row| row.date.get(..7).map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    let keep = config.baseline_months.max(1).min(available.len());
    let months = available.split_off(available.len().saturating_sub(keep));
    let mut income_by_month = BTreeMap::new();
    let mut spending_by_month = BTreeMap::new();
    let mut forecast_base_by_month = BTreeMap::new();
    let mut behavior_by_month: BTreeMap<ExpenseBehavior, BTreeMap<String, i64>> = BTreeMap::new();
    let mut classified_value = 0_i64;
    let mut spending_value = 0_i64;
    for row in rows.iter().filter(|row| {
        row.currency == currency
            && row
                .date
                .get(..7)
                .is_some_and(|month| months.iter().any(|selected| selected == month))
    }) {
        let month = row.date[..7].to_string();
        match row.kind {
            TransactionKind::Income => {
                *income_by_month.entry(month).or_default() += row.amount_cents
            }
            TransactionKind::Expense => {
                *spending_by_month.entry(month.clone()).or_default() += row.amount_cents;
                spending_value += row.amount_cents;
                let behavior = classify(row, commitments, config);
                if behavior != ExpenseBehavior::Exceptional
                    && !replaced_in_forecast(row, commitments, config)
                {
                    *forecast_base_by_month.entry(month.clone()).or_default() += row.amount_cents;
                }
                if behavior != ExpenseBehavior::Unclassified {
                    classified_value += row.amount_cents;
                }
                *behavior_by_month
                    .entry(behavior)
                    .or_default()
                    .entry(month)
                    .or_default() += row.amount_cents;
            }
            TransactionKind::Transfer => {}
        }
    }
    let monthly_income_cents = median_for_months(&months, &income_by_month);
    let monthly_spending_cents = median_for_months(&months, &spending_by_month);
    let forecast_base_cents = median_for_months(&months, &forecast_base_by_month);
    let behavior = [
        ExpenseBehavior::Fixed,
        ExpenseBehavior::Variable,
        ExpenseBehavior::Discretionary,
        ExpenseBehavior::Exceptional,
        ExpenseBehavior::Unclassified,
    ]
    .into_iter()
    .map(|kind| BehaviorAmount {
        behavior: kind.as_str().into(),
        monthly_cents: median_for_months(
            &months,
            behavior_by_month.get(&kind).unwrap_or(&BTreeMap::new()),
        ),
    })
    .collect();
    MonthlyBaseline {
        months,
        monthly_income_cents,
        monthly_spending_cents,
        forecast_base_cents,
        monthly_result_cents: monthly_income_cents - monthly_spending_cents,
        savings_rate_percent: (monthly_income_cents > 0).then_some(
            (monthly_income_cents - monthly_spending_cents) as f64 / monthly_income_cents as f64
                * 100.0,
        ),
        behavior,
        classified_value_percent: (spending_value > 0)
            .then_some(classified_value as f64 / spending_value as f64 * 100.0),
    }
}

fn replaced_in_forecast(
    row: &TransactionRow,
    commitments: &[RecurringCommitment],
    config: &PlanningConfig,
) -> bool {
    commitments
        .iter()
        .any(|commitment| row.category.starts_with(&commitment.account))
        || (config.include_subscription_burn
            && config
                .subscription_account_prefixes
                .iter()
                .any(|prefix| row.category.starts_with(prefix)))
}

fn classify(
    row: &TransactionRow,
    commitments: &[RecurringCommitment],
    config: &PlanningConfig,
) -> ExpenseBehavior {
    if row.purpose == Some(SpendingPurpose::Trip) {
        return ExpenseBehavior::Exceptional;
    }
    if config
        .subscription_account_prefixes
        .iter()
        .any(|prefix| row.category.starts_with(prefix))
    {
        return ExpenseBehavior::Fixed;
    }
    config
        .spending_rules
        .iter()
        .filter(|rule| row.category.starts_with(&rule.account_prefix))
        .max_by_key(|rule| rule.account_prefix.len())
        .map(|rule| rule.behavior)
        .or_else(|| {
            commitments
                .iter()
                .any(|commitment| row.category.starts_with(&commitment.account))
                .then_some(ExpenseBehavior::Fixed)
        })
        .unwrap_or_default()
}

fn median_for_months(months: &[String], values: &BTreeMap<String, i64>) -> i64 {
    let mut amounts: Vec<i64> = months
        .iter()
        .map(|month| values.get(month).copied().unwrap_or(0))
        .collect();
    if amounts.is_empty() {
        return 0;
    }
    amounts.sort_unstable();
    let middle = amounts.len() / 2;
    if amounts.len() % 2 == 1 {
        amounts[middle]
    } else {
        amounts[middle - 1].saturating_add(amounts[middle]) / 2
    }
}

fn forecasts(
    baseline: &MonthlyBaseline,
    commitments: &[RecurringCommitment],
    subscriptions: &[Subscription],
    config: &PlanningConfig,
    as_of: &str,
    currency: &str,
) -> Vec<ForecastPoint> {
    let mut dates = BTreeSet::from([as_of.to_string()]);
    dates.extend(
        config
            .comparison_dates
            .iter()
            .filter(|date| date.as_str() >= as_of)
            .cloned(),
    );
    dates.extend(
        commitments
            .iter()
            .map(|commitment| &commitment.valid_from)
            .filter(|date| date.as_str() > as_of)
            .cloned(),
    );
    dates.extend(
        config
            .forecast_adjustments
            .iter()
            .map(|adjustment| &adjustment.valid_from)
            .filter(|date| date.as_str() > as_of)
            .cloned(),
    );
    for subscription in subscriptions {
        dates.extend(
            subscription
                .prices
                .iter()
                .map(|price| &price.valid_from)
                .filter(|date| date.as_str() > as_of)
                .cloned(),
        );
        dates.extend(
            subscription
                .states
                .iter()
                .map(|state| &state.effective)
                .filter(|date| date.as_str() > as_of)
                .cloned(),
        );
    }
    let historical_base_cents = baseline.forecast_base_cents;
    dates
        .into_iter()
        .take(8)
        .map(|date| {
            let commitments_cents = commitments
                .iter()
                .filter(|commitment| commitment.currency == currency && commitment.active_on(&date))
                .map(|commitment| commitment.monthly_cents)
                .sum::<i64>();
            let subscriptions_cents = config
                .include_subscription_burn
                .then(|| burn_by_currency(subscriptions, &date))
                .and_then(|burn| {
                    burn.currencies
                        .into_iter()
                        .find(|amount| amount.currency == currency)
                })
                .map(|amount| amount.monthly_cents)
                .unwrap_or(0);
            let adjustments_cents = config
                .forecast_adjustments
                .iter()
                .filter(|adjustment| adjustment.currency == currency && adjustment.active_on(&date))
                .map(|adjustment| adjustment.monthly_cents)
                .sum::<i64>();
            let projected_spending_cents = historical_base_cents
                .saturating_add(commitments_cents)
                .saturating_add(subscriptions_cents)
                .saturating_add(adjustments_cents);
            let projected_result_cents = baseline.monthly_income_cents - projected_spending_cents;
            ForecastPoint {
                as_of: date,
                historical_base_cents,
                commitments_cents,
                subscriptions_cents,
                adjustments_cents,
                projected_spending_cents,
                projected_result_cents,
                savings_rate_percent: (baseline.monthly_income_cents > 0).then_some(
                    projected_result_cents as f64 / baseline.monthly_income_cents as f64 * 100.0,
                ),
            }
        })
        .collect()
}

fn liquidity(
    balance: Option<&ManualBalanceSnapshot>,
    investment: Option<&ReviewedHoldingsSnapshot>,
    values: &[PortfolioValuation],
    monthly_spending_cents: i64,
    target_months: u32,
    currency: &str,
) -> Option<LiquidityInsight> {
    let balance = balance.filter(|snapshot| snapshot.currency == currency)?;
    let liquid_assets_cents = balance
        .balances
        .iter()
        .filter(|entry| entry.kind == BalanceKind::Asset)
        .map(|entry| entry.amount_cents)
        .sum::<i64>();
    let liabilities_cents = balance
        .balances
        .iter()
        .filter(|entry| entry.kind == BalanceKind::Liability)
        .map(|entry| entry.amount_cents)
        .sum::<i64>();
    let portfolio = values.iter().find(|value| value.currency == currency);
    let invested_cents =
        portfolio.and_then(|value| decimal_cents(value.value.mantissa, value.value.scale));
    let gross_assets = liquid_assets_cents.saturating_add(invested_cents.unwrap_or(0));
    let net_worth_cents = gross_assets.saturating_sub(liabilities_cents);
    let cash_share_percent = (gross_assets > 0)
        .then_some(gross_assets)
        .map(|assets| liquid_assets_cents as f64 / assets as f64 * 100.0);
    let target_cash_cents = monthly_spending_cents
        .max(0)
        .saturating_mul(i64::from(target_months));
    let complete = balance.coverage == BalanceCoverage::Complete
        && investment.is_some_and(|snapshot| snapshot.coverage == HoldingsCoverage::Complete)
        && portfolio.is_some_and(|value| value.unpriced_holdings == 0);
    Some(LiquidityInsight {
        currency: currency.into(),
        liquid_assets_cents,
        liabilities_cents,
        invested_cents,
        net_worth_cents: Some(net_worth_cents),
        cash_share_percent,
        largest_priced_holding_percent: investment
            .and_then(|snapshot| largest_holding_share(snapshot, currency)),
        runway_months: (monthly_spending_cents > 0)
            .then_some(liquid_assets_cents as f64 / monthly_spending_cents as f64),
        target_cash_cents,
        cash_buffer_cents: liquid_assets_cents.saturating_sub(target_cash_cents),
        complete,
    })
}

fn decimal_cents(mantissa: i128, scale: u32) -> Option<i64> {
    let cents = if scale > 2 {
        mantissa.checked_div(10_i128.checked_pow(scale - 2)?)?
    } else {
        mantissa.checked_mul(10_i128.checked_pow(2 - scale)?)?
    };
    i64::try_from(cents).ok()
}

fn largest_holding_share(snapshot: &ReviewedHoldingsSnapshot, currency: &str) -> Option<f64> {
    let values: Vec<i128> = snapshot
        .holdings
        .iter()
        .filter(|holding| holding.currency == currency)
        .filter_map(|holding| {
            let price = holding.latest_unit_price.as_ref()?;
            let mantissa =
                i128::from(holding.quantity.mantissa).checked_mul(i128::from(price.mantissa))?;
            let scale = holding.quantity.scale.checked_add(price.scale)?;
            normalize_decimal(mantissa, scale, 8)
        })
        .filter(|value| *value > 0)
        .collect();
    let total = values.iter().copied().sum::<i128>();
    let largest = values.iter().copied().max()?;
    (total > 0).then_some(largest as f64 / total as f64 * 100.0)
}

fn normalize_decimal(mantissa: i128, scale: u32, target_scale: u32) -> Option<i128> {
    if scale > target_scale {
        mantissa.checked_div(10_i128.checked_pow(scale - target_scale)?)
    } else {
        mantissa.checked_mul(10_i128.checked_pow(target_scale - scale)?)
    }
}

fn subscription_portfolio(
    subscriptions: &[Subscription],
    as_of: &str,
    currency: &str,
) -> SubscriptionPortfolio {
    let burn = burn_by_currency(subscriptions, as_of);
    let monthly_cents = burn
        .currencies
        .iter()
        .find(|amount| amount.currency == currency)
        .map(|amount| amount.monthly_cents)
        .unwrap_or(0);
    let mut anomalies = Vec::new();
    for subscription in subscriptions {
        let state = subscription.state_at(as_of);
        if state.is_billing() && subscription.price_at(as_of).is_none() {
            anomalies.push(SubscriptionAnomaly {
                subscription_id: subscription.id.clone(),
                subscription_name: subscription.name.clone(),
                kind: "missing_price".into(),
                detail: "Billing state has no price in force.".into(),
            });
        }
        if state.is_billing() && subscription.value_rating.is_some_and(|rating| rating <= 2) {
            anomalies.push(SubscriptionAnomaly {
                subscription_id: subscription.id.clone(),
                subscription_name: subscription.name.clone(),
                kind: "low_value".into(),
                detail: "Personally billing with a low value rating.".into(),
            });
        }
        // PRD Q103. `monthly_cents` above takes the bucket for `currency` and nothing
        // else, so a subscription billing in another currency contributes zero to the
        // burn with no trace. That is the same wrong figure as relabelling it, only
        // quieter: the total is short and nothing says by how much.
        if let (true, Some(price)) = (state.is_billing(), subscription.price_at(as_of)) {
            if !price.currency.eq_ignore_ascii_case(currency) {
                anomalies.push(SubscriptionAnomaly {
                    subscription_id: subscription.id.clone(),
                    subscription_name: subscription.name.clone(),
                    kind: "currency_outside_total".into(),
                    detail: format!(
                        "Priced in {} and the total is in {currency}, so it is not in the monthly figure.",
                        price.currency
                    ),
                });
            }
        }
        // A currency that changes inside one series is either a provider redenomination
        // or a bad seed, such as a USD point followed by a EUR point that the importer
        // wrote before it read `currency:`. Either way the series says two different
        // things about one price.
        if let Some(first) = subscription.prices.first() {
            if let Some(changed) = subscription
                .prices
                .iter()
                .find(|price| !price.currency.eq_ignore_ascii_case(&first.currency))
            {
                anomalies.push(SubscriptionAnomaly {
                    subscription_id: subscription.id.clone(),
                    subscription_name: subscription.name.clone(),
                    kind: "currency_change".into(),
                    detail: format!(
                        "The price series starts in {} on {} and switches to {} on {}.",
                        first.currency, first.valid_from, changed.currency, changed.valid_from
                    ),
                });
            }
        }
        let current = subscription.price_at(as_of);
        // The row that will actually be in force on its own date, which a `min_by` over
        // `valid_from` is not when two points share that date.
        let next = subscription.scheduled_price_after(as_of);
        if let (Some(current), Some(next)) = (current, next) {
            let current_monthly = current.cycle.monthly_cents(current.amount_cents);
            let next_monthly = next.cycle.monthly_cents(next.amount_cents);
            // PRD Q103: `next_monthly >= current_monthly * 5 / 4` is a comparison, and
            // a comparison needs one currency as much as a sum does. 15.00 USD
            // followed by 15.00 EUR is not a 0% rise, and 45.00 EUR following 15.00
            // USD is not a 200% one. The `currency_change` anomaly above already names
            // that series, so there is nothing left for a made-up ratio to add.
            let comparable = current.currency.eq_ignore_ascii_case(&next.currency);
            if comparable
                && current_monthly > 0
                && next_monthly >= current_monthly.saturating_mul(5) / 4
            {
                anomalies.push(SubscriptionAnomaly {
                    subscription_id: subscription.id.clone(),
                    subscription_name: subscription.name.clone(),
                    kind: "price_jump".into(),
                    detail: format!("Monthly equivalent rises on {}.", next.valid_from),
                });
            }
        }
        if matches!(state, State::Cancelled | State::Paused)
            && subscription
                .prices
                .iter()
                .any(|price| price.valid_from.as_str() > as_of)
        {
            anomalies.push(SubscriptionAnomaly {
                subscription_id: subscription.id.clone(),
                subscription_name: subscription.name.clone(),
                kind: "future_charge_after_stop".into(),
                detail: "A future price exists after the service stopped billing.".into(),
            });
        }
    }
    SubscriptionPortfolio {
        monthly_cents,
        annual_cents: monthly_cents.saturating_mul(12),
        billing_count: burn.billing_count,
        covered_count: burn.covered_count,
        unknown_price_count: burn.unknown_price_count,
        anomalies,
    }
}

struct DerivedCardSpend {
    cents: i64,
    source: String,
    start: Option<String>,
    end: Option<String>,
}

fn eligible_card_spend(
    rows: &[TransactionRow],
    input: &CardDecisionInput,
    as_of: &str,
    currency: &str,
) -> DerivedCardSpend {
    if input.eligible_account_prefixes.is_empty() {
        return DerivedCardSpend {
            cents: input.annual_eligible_spend_cents.max(0),
            source: "manual".into(),
            start: None,
            end: None,
        };
    }
    let end_month = as_of.get(..7).unwrap_or(as_of);
    let start_month = shift_month(end_month, -11).unwrap_or_else(|| end_month.to_string());
    let cents = rows
        .iter()
        .filter(|row| {
            row.currency == currency
                && row.kind == TransactionKind::Expense
                && row
                    .date
                    .get(..7)
                    .is_some_and(|month| month >= start_month.as_str() && month <= end_month)
                && input
                    .eligible_account_prefixes
                    .iter()
                    .any(|prefix| row.account.starts_with(prefix))
                && !input
                    .ineligible_category_prefixes
                    .iter()
                    .any(|prefix| row.category.starts_with(prefix))
        })
        .map(|row| row.amount_cents)
        .sum();
    DerivedCardSpend {
        cents,
        source: "reviewed_transactions".into(),
        start: Some(format!("{start_month}-01")),
        end: Some(as_of.into()),
    }
}

fn shift_month(month: &str, delta: i32) -> Option<String> {
    let (year, month) = month.split_once('-')?;
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<i32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let absolute = year
        .checked_mul(12)?
        .checked_add(month - 1)?
        .checked_add(delta)?;
    let shifted_year = absolute.div_euclid(12);
    let shifted_month = absolute.rem_euclid(12) + 1;
    Some(format!("{shifted_year:04}-{shifted_month:02}"))
}

fn card_decision(
    input: &CardDecisionInput,
    spend: DerivedCardSpend,
    currency: &str,
) -> CardDecisionResult {
    let mut options: Vec<CardOptionResult> = input
        .options
        .iter()
        .filter(|option| option.currency == currency)
        .map(|option| {
            let benefits: Vec<CardBenefitResult> = option
                .benefits
                .iter()
                .map(|benefit| {
                    let face = benefit.annual_face_value_cents.max(0);
                    CardBenefitResult {
                        id: benefit.id.clone(),
                        label: benefit.label.clone(),
                        annual_face_value_cents: face,
                        annual_personal_value_cents: benefit
                            .annual_personal_value_cents
                            .clamp(0, face),
                    }
                })
                .collect();
            let annual_face_value_cents = benefits.iter().fold(0_i64, |total, benefit| {
                total.saturating_add(benefit.annual_face_value_cents)
            });
            let annual_benefit_value_cents = benefits.iter().fold(0_i64, |total, benefit| {
                total.saturating_add(benefit.annual_personal_value_cents)
            });
            let reward_denominator = 100_i128 * 1000 * 1000;
            let annual_reward_value_cents = i64::try_from(
                i128::from(spend.cents)
                    .saturating_mul(i128::from(option.points_per_currency_unit_milli.max(0)))
                    .saturating_mul(i128::from(option.point_value_milli_cents.max(0)))
                    / reward_denominator,
            )
            .unwrap_or(i64::MAX);
            let annual_fx_cost_cents = input
                .annual_fx_spend_cents
                .max(0)
                .saturating_mul(option.fx_fee_basis_points.max(0))
                / 10_000;
            let annual_net_value_cents = annual_benefit_value_cents
                .saturating_add(annual_reward_value_cents)
                .saturating_sub(option.annual_fee_cents)
                .saturating_sub(annual_fx_cost_cents);
            let reward_product = i128::from(option.points_per_currency_unit_milli.max(0))
                .saturating_mul(i128::from(option.point_value_milli_cents.max(0)));
            let gap = option
                .annual_fee_cents
                .saturating_add(annual_fx_cost_cents)
                .saturating_sub(annual_benefit_value_cents)
                .max(0);
            let break_even_eligible_spend_cents = if gap == 0 {
                Some(0)
            } else if reward_product == 0 {
                None
            } else {
                i64::try_from(i128::from(gap).saturating_mul(reward_denominator) / reward_product)
                    .ok()
            };
            CardOptionResult {
                id: option.id.clone(),
                label: option.label.clone(),
                currency: option.currency.clone(),
                annual_fee_cents: option.annual_fee_cents,
                annual_face_value_cents,
                annual_benefit_value_cents,
                annual_unvalued_face_value_cents: annual_face_value_cents
                    .saturating_sub(annual_benefit_value_cents),
                annual_reward_value_cents,
                annual_fx_cost_cents,
                annual_net_value_cents,
                break_even_eligible_spend_cents,
                points_per_currency_unit_milli: option.points_per_currency_unit_milli,
                point_value_milli_cents: option.point_value_milli_cents,
                point_value_assumption: option.point_value_assumption.clone(),
                terms_checked_on: option.terms_checked_on.clone(),
                source_urls: option.source_urls.clone(),
                benefits,
            }
        })
        .collect();
    options.sort_by(|left, right| {
        right
            .annual_net_value_cents
            .cmp(&left.annual_net_value_cents)
            .then(left.label.cmp(&right.label))
    });
    let provisional = !input.usage_reviewed
        || options.is_empty()
        || options.iter().any(|option| {
            option.terms_checked_on.trim().is_empty() || option.source_urls.is_empty()
        });
    CardDecisionResult {
        annual_eligible_spend_cents: spend.cents,
        annual_fx_spend_cents: input.annual_fx_spend_cents,
        usage_reviewed: input.usage_reviewed,
        spend_source: spend.source,
        spend_period_start: spend.start,
        spend_period_end: spend.end,
        options,
        provisional,
    }
}

fn default_currency() -> String {
    "EUR".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::TransactionRow;
    use crate::investment::{DecimalValue, Holding, Quantity};
    use crate::subscription::{BillingCycle, PricePoint, StateChange};

    fn row(month: &str, kind: TransactionKind, category: &str, cents: i64) -> TransactionRow {
        TransactionRow {
            id: format!("{month}-{category}-{cents}"),
            date: format!("{month}-10"),
            description: "Synthetic".into(),
            kind,
            account: "assets:bank:checking".into(),
            category: category.into(),
            amount_cents: cents,
            currency: "EUR".into(),
            source_id: None,
            purpose: None,
            trip_id: None,
            cash_amount_cents: cents,
            shared_cents: 0,
            reimbursement_for: None,
        }
    }

    fn subscription() -> Subscription {
        Subscription {
            id: "synthetic-subscription".into(),
            name: "Synthetic service".into(),
            source_path: "Subscriptions/Synthetic.md".into(),
            category: Some("software".into()),
            value_rating: Some(2),
            prices: vec![
                PricePoint {
                    valid_from: "2026-01-01".into(),
                    amount_cents: 1_000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: "synthetic".into(),
                },
                PricePoint {
                    valid_from: "2026-10-01".into(),
                    amount_cents: 2_000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: "synthetic increase".into(),
                },
            ],
            states: vec![StateChange {
                effective: "2026-01-01".into(),
                state: State::Active,
                note: String::new(),
            }],
        }
    }

    #[test]
    fn baseline_uses_complete_month_medians_and_private_rules() {
        let rows = vec![
            row("2026-05", TransactionKind::Income, "income:salary", 300_000),
            row(
                "2026-05",
                TransactionKind::Expense,
                "expenses:housing",
                50_000,
            ),
            row("2026-05", TransactionKind::Expense, "expenses:food", 20_000),
            row("2026-06", TransactionKind::Income, "income:salary", 300_000),
            row(
                "2026-06",
                TransactionKind::Expense,
                "expenses:housing",
                50_000,
            ),
            row("2026-06", TransactionKind::Expense, "expenses:food", 30_000),
            row("2026-07", TransactionKind::Income, "income:salary", 300_000),
            row(
                "2026-07",
                TransactionKind::Expense,
                "expenses:housing",
                50_000,
            ),
            row("2026-07", TransactionKind::Expense, "expenses:food", 25_000),
        ];
        let config = PlanningConfig {
            spending_rules: vec![
                SpendingRule {
                    account_prefix: "expenses:housing".into(),
                    behavior: ExpenseBehavior::Fixed,
                },
                SpendingRule {
                    account_prefix: "expenses:food".into(),
                    behavior: ExpenseBehavior::Variable,
                },
            ],
            ..PlanningConfig::default()
        };
        let baseline = baseline(&rows, &[], &config, "2026-08-11", "EUR");
        assert_eq!(baseline.months, ["2026-05", "2026-06", "2026-07"]);
        assert_eq!(baseline.monthly_income_cents, 300_000);
        assert_eq!(baseline.monthly_spending_cents, 75_000);
        assert_eq!(baseline.forecast_base_cents, 75_000);
        assert_eq!(baseline.behavior[0].monthly_cents, 50_000);
        assert_eq!(baseline.behavior[1].monthly_cents, 25_000);
        assert_eq!(baseline.classified_value_percent, Some(100.0));
    }

    #[test]
    fn dated_commitments_create_a_future_comparison_without_double_counting_fixed_costs() {
        let baseline = MonthlyBaseline {
            months: vec!["2026-07".into()],
            monthly_income_cents: 300_000,
            monthly_spending_cents: 100_000,
            forecast_base_cents: 80_000,
            monthly_result_cents: 200_000,
            savings_rate_percent: Some(66.6),
            behavior: vec![
                BehaviorAmount {
                    behavior: "fixed".into(),
                    monthly_cents: 20_000,
                },
                BehaviorAmount {
                    behavior: "variable".into(),
                    monthly_cents: 80_000,
                },
                BehaviorAmount {
                    behavior: "discretionary".into(),
                    monthly_cents: 0,
                },
                BehaviorAmount {
                    behavior: "unclassified".into(),
                    monthly_cents: 0,
                },
            ],
            classified_value_percent: Some(100.0),
        };
        let commitments = vec![
            RecurringCommitment {
                id: "old".into(),
                label: "Synthetic old".into(),
                account: "expenses:housing".into(),
                monthly_cents: 20_000,
                currency: "EUR".into(),
                valid_from: "2026-01-01".into(),
                valid_until: Some("2026-08-31".into()),
            },
            RecurringCommitment {
                id: "new".into(),
                label: "Synthetic new".into(),
                account: "expenses:housing".into(),
                monthly_cents: 60_000,
                currency: "EUR".into(),
                valid_from: "2026-09-01".into(),
                valid_until: None,
            },
        ];
        let points = forecasts(
            &baseline,
            &commitments,
            &[],
            &PlanningConfig {
                include_subscription_burn: false,
                ..PlanningConfig::default()
            },
            "2026-08-11",
            "EUR",
        );
        assert_eq!(points[0].projected_spending_cents, 100_000);
        assert_eq!(points[1].as_of, "2026-09-01");
        assert_eq!(points[1].projected_spending_cents, 140_000);
    }

    #[test]
    fn liquidity_keeps_partial_evidence_visible() {
        let balance = ManualBalanceSnapshot {
            schema_version: 1,
            as_of: "2026-08-11".into(),
            updated_at: "2026-08-11T12:00:00Z".into(),
            currency: "EUR".into(),
            coverage: BalanceCoverage::Partial,
            balances: vec![
                crate::balance::ManualBalance {
                    id: "cash".into(),
                    label: "Synthetic cash".into(),
                    kind: BalanceKind::Asset,
                    amount_cents: 600_000,
                },
                crate::balance::ManualBalance {
                    id: "card".into(),
                    label: "Synthetic card".into(),
                    kind: BalanceKind::Liability,
                    amount_cents: 50_000,
                },
            ],
        };
        let holdings = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-08-11".into(),
            coverage: HoldingsCoverage::Partial,
            holdings: vec![
                Holding {
                    instrument: "ACME".into(),
                    quantity: Quantity {
                        mantissa: 10,
                        scale: 0,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: 10_000,
                        scale: 2,
                    }),
                    currency: "EUR".into(),
                },
                Holding {
                    instrument: "FOREIGN".into(),
                    quantity: Quantity {
                        mantissa: 1_000,
                        scale: 0,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: 10_000,
                        scale: 2,
                    }),
                    currency: "USD".into(),
                },
            ],
            sources: vec![],
        };
        let values = vec![PortfolioValuation {
            currency: "EUR".into(),
            value: DecimalValue {
                mantissa: 100_000,
                scale: 2,
            },
            priced_holdings: 1,
            unpriced_holdings: 0,
        }];
        let insight =
            liquidity(Some(&balance), Some(&holdings), &values, 100_000, 6, "EUR").unwrap();
        assert_eq!(insight.runway_months, Some(6.0));
        assert_eq!(insight.cash_buffer_cents, 0);
        assert_eq!(insight.net_worth_cents, Some(650_000));
        assert_eq!(insight.largest_priced_holding_percent, Some(100.0));
        assert!(!insight.complete);

        let cash_only = liquidity(Some(&balance), None, &[], 100_000, 6, "EUR").unwrap();
        assert_eq!(cash_only.invested_cents, None);
        assert_eq!(cash_only.net_worth_cents, Some(550_000));
        assert_eq!(cash_only.cash_share_percent, Some(100.0));
        assert!(!cash_only.complete);
    }

    #[test]
    fn forecast_retains_fixed_costs_without_a_replacement_series() {
        let rows = vec![
            row(
                "2026-07",
                TransactionKind::Expense,
                "expenses:housing",
                50_000,
            ),
            row(
                "2026-07",
                TransactionKind::Expense,
                "expenses:insurance",
                10_000,
            ),
            row("2026-07", TransactionKind::Expense, "expenses:food", 20_000),
        ];
        let commitments = vec![RecurringCommitment {
            id: "housing".into(),
            label: "Synthetic housing".into(),
            account: "expenses:housing".into(),
            monthly_cents: 50_000,
            currency: "EUR".into(),
            valid_from: "2026-01-01".into(),
            valid_until: None,
        }];
        let config = PlanningConfig {
            spending_rules: vec![
                SpendingRule {
                    account_prefix: "expenses:housing".into(),
                    behavior: ExpenseBehavior::Fixed,
                },
                SpendingRule {
                    account_prefix: "expenses:insurance".into(),
                    behavior: ExpenseBehavior::Fixed,
                },
                SpendingRule {
                    account_prefix: "expenses:food".into(),
                    behavior: ExpenseBehavior::Variable,
                },
            ],
            include_subscription_burn: false,
            ..PlanningConfig::default()
        };

        let baseline = baseline(&rows, &commitments, &config, "2026-08-11", "EUR");
        assert_eq!(baseline.forecast_base_cents, 30_000);
        let points = forecasts(&baseline, &commitments, &[], &config, "2026-08-11", "EUR");
        assert_eq!(points[0].projected_spending_cents, 80_000);
    }

    #[test]
    fn subscription_anomalies_flag_low_value_and_a_large_scheduled_increase() {
        let result = subscription_portfolio(&[subscription()], "2026-08-11", "EUR");
        assert_eq!(result.monthly_cents, 1_000);
        assert!(result.anomalies.iter().any(|item| item.kind == "low_value"));
        assert!(result
            .anomalies
            .iter()
            .any(|item| item.kind == "price_jump"));
    }

    #[test]
    fn a_scheduled_price_in_another_currency_is_a_currency_change_not_a_price_jump() {
        // PRD Q103: 10.00 EUR followed by 20.00 USD is not a 100% rise, because the
        // two numbers are not in the same unit. The series is reported for what it is
        // and no ratio is invented from it.
        let mut sub = subscription();
        sub.prices[1].currency = "USD".into();
        let result = subscription_portfolio(&[sub], "2026-08-11", "EUR");
        assert!(
            result
                .anomalies
                .iter()
                .any(|item| item.kind == "currency_change"),
            "the series changes currency and must say so: {:?}",
            result.anomalies
        );
        assert!(
            !result
                .anomalies
                .iter()
                .any(|item| item.kind == "price_jump"),
            "a cross-currency ratio is not a price jump: {:?}",
            result.anomalies
        );
    }

    #[test]
    fn card_math_exposes_point_value_and_break_even_assumptions() {
        let decision = card_decision(
            &CardDecisionInput {
                annual_eligible_spend_cents: 1_200_000,
                annual_fx_spend_cents: 100_000,
                usage_reviewed: true,
                eligible_account_prefixes: Vec::new(),
                ineligible_category_prefixes: Vec::new(),
                options: vec![CardOptionInput {
                    id: "synthetic-card".into(),
                    label: "Synthetic card".into(),
                    annual_fee_cents: 60_000,
                    currency: "EUR".into(),
                    points_per_currency_unit_milli: 1_000,
                    point_value_milli_cents: 500,
                    point_value_assumption: "Synthetic half-cent value".into(),
                    fx_fee_basis_points: 200,
                    benefits: vec![CardBenefitInput {
                        id: "credit".into(),
                        label: "Synthetic credit".into(),
                        annual_face_value_cents: 30_000,
                        annual_personal_value_cents: 20_000,
                    }],
                    terms_checked_on: "2026-08-11".into(),
                    source_urls: vec!["https://example.invalid/terms".into()],
                }],
            },
            DerivedCardSpend {
                cents: 1_200_000,
                source: "manual".into(),
                start: None,
                end: None,
            },
            "EUR",
        );
        let option = &decision.options[0];
        assert_eq!(option.annual_face_value_cents, 30_000);
        assert_eq!(option.annual_unvalued_face_value_cents, 10_000);
        assert_eq!(option.benefits[0].annual_personal_value_cents, 20_000);
        assert_eq!(option.annual_reward_value_cents, 6_000);
        assert_eq!(option.annual_fx_cost_cents, 2_000);
        assert_eq!(option.annual_net_value_cents, -36_000);
        assert_eq!(option.break_even_eligible_spend_cents, Some(8_400_000));
        assert!(!decision.provisional);
    }

    #[test]
    fn reviewed_card_spend_sums_split_postings_once() {
        let mut food = row("2026-07", TransactionKind::Expense, "expenses:food", 600);
        food.account = "liabilities:card:synthetic".into();
        food.cash_amount_cents = 1_000;
        let mut household = row(
            "2026-07",
            TransactionKind::Expense,
            "expenses:household",
            400,
        );
        household.account = "liabilities:card:synthetic".into();
        household.cash_amount_cents = 0;
        let mut input = CardDecisionInput {
            annual_eligible_spend_cents: 0,
            annual_fx_spend_cents: 0,
            usage_reviewed: false,
            eligible_account_prefixes: vec!["liabilities:card:synthetic".into()],
            ineligible_category_prefixes: Vec::new(),
            options: Vec::new(),
        };

        let spend = eligible_card_spend(
            &[food.clone(), household.clone()],
            &input,
            "2026-08-11",
            "EUR",
        );
        assert_eq!(spend.cents, 1_000);

        input.ineligible_category_prefixes = vec!["expenses:household".into()];
        let eligible_food_only =
            eligible_card_spend(&[food, household], &input, "2026-08-11", "EUR");
        assert_eq!(eligible_food_only.cents, 600);
    }

    #[test]
    fn private_planning_config_is_optional_and_has_conservative_defaults() {
        let config: PlanningConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.baseline_months, 3);
        assert_eq!(config.runway_target_months, 6);
        assert_eq!(config.journal_freshness_days, 45);
        assert_eq!(config.snapshot_freshness_days, 14);
        assert!(config.include_subscription_burn);
        assert!(config.card_decision.is_none());
        assert!(config.source_expectations.is_empty());
    }
}

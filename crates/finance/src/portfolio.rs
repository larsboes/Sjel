//! What the portfolio is worth, defined once.
//!
//! `GET /api/dashboard` and `GET /api/portfolio` both come through here, so the
//! two cannot disagree about a total. That is the whole reason this module
//! exists: the valuation was inlined in the dashboard handler, and a second
//! endpoint computing it a second way is exactly how one number becomes two.
//!
//! A market price is never written back into the reviewed holdings snapshot. It
//! is layered over it at read time and labelled with its source and its
//! observation date, so a reader can see which of the two numbers they are
//! looking at. `value_basis` says `market` or `review` for every position.
//!
//! What this cannot compute, and says so rather than guessing: there is no lot
//! and no cost basis anywhere in this capability, so `change_since_review` is a
//! difference against the latest broker activity price. It is not a return and
//! it is not P&L.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::clock;
use crate::config::{InstrumentProfile, TargetPolicy};
use crate::investment::{self, DecimalValue, HoldingsCoverage, Quantity, ReviewedHoldingsSnapshot};
use crate::price::PriceObservation;

/// Ten thousand basis points is the whole portfolio.
pub const FULL_ALLOCATION_BP: i64 = 10_000;

/// How far an active cohort may miss 10,000 bp before drift is refused.
///
/// Not zero: a policy written by hand in whole percentages rounds, and refusing
/// a cohort that sums to 9,999 would be pedantry. Not wide either -- 50 bp is
/// half a percent, past which the reader is measuring drift against a policy
/// that does not describe a whole portfolio.
pub const COHORT_TOLERANCE_BP: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PositionView {
    pub instrument: String,
    pub label: String,
    pub asset_class: String,
    pub quantity: Quantity,
    pub currency: String,
    /// The price the reviewed import carried. Absent for an unpriced holding.
    pub review_price: Option<Quantity>,
    pub market_price: Option<Quantity>,
    pub market_price_source: Option<String>,
    pub market_price_observed_on: Option<String>,
    pub price_age_days: Option<i64>,
    /// `fresh`, `stale` or `none`. Never a boolean: "no price at all" and "a
    /// price nobody refreshed" send a reader to two different places.
    pub price_freshness: String,
    pub value: DecimalValue,
    /// `market` when a market observation priced it, `review` when the broker
    /// activity price did.
    pub value_basis: String,
    pub share_bp: i64,
    pub target_bp: Option<i64>,
    pub band_bp: Option<i64>,
    pub drift_bp: Option<i64>,
    pub outside_band: bool,
    pub change_since_review: Option<DecimalValue>,
    pub change_since_review_bp: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssetClassView {
    pub asset_class: String,
    pub value: DecimalValue,
    pub share_bp: i64,
    pub target_bp: Option<i64>,
    pub band_bp: Option<i64>,
    pub drift_bp: Option<i64>,
    pub outside_band: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortfolioReport {
    pub as_of: String,
    pub currency: String,
    pub coverage: String,
    /// False renders as "no policy declared yet", never as a table of zero
    /// drift. Thirteen rows of 0 bp reads as a portfolio on target.
    pub targets_configured: bool,
    pub total: DecimalValue,
    pub priced_positions: usize,
    pub unpriced_positions: usize,
    pub positions: Vec<PositionView>,
    pub asset_classes: Vec<AssetClassView>,
    pub caveats: Vec<String>,
}

/// Everything the report needs, assembled by the caller.
pub struct PortfolioInputs<'a> {
    pub snapshot: &'a ReviewedHoldingsSnapshot,
    pub latest_prices: &'a [PriceObservation],
    pub instruments: &'a [InstrumentProfile],
    pub targets: Option<&'a TargetPolicy>,
    pub as_of: &'a str,
    pub currency: &'a str,
}

/// True when every holding in a complete snapshot carries a price.
///
/// Lifted out of the dashboard handler unchanged. A share computed over an
/// incomplete portfolio is a wrong number that looks right, which is why the
/// rules refuse to propose against one.
pub fn portfolio_complete(snapshot: &ReviewedHoldingsSnapshot, unpriced: usize) -> bool {
    snapshot.coverage == HoldingsCoverage::Complete && unpriced == 0
}

pub fn report(inputs: PortfolioInputs<'_>) -> Result<PortfolioReport, String> {
    let mut caveats = Vec::new();
    let prices: BTreeMap<&str, &PriceObservation> = inputs
        .latest_prices
        .iter()
        .map(|observation| (observation.instrument.as_str(), observation))
        .collect();
    let profiles: BTreeMap<&str, &InstrumentProfile> = inputs
        .instruments
        .iter()
        .map(|profile| (profile.instrument.as_str(), profile))
        .collect();
    let freshness_days = inputs
        .targets
        .map(|targets| targets.price_freshness_days)
        .unwrap_or(4);

    let mut positions = Vec::new();
    let mut total_cents: i128 = 0;
    let mut priced = 0usize;
    let mut unpriced = 0usize;

    for holding in &inputs.snapshot.holdings {
        if holding.currency != inputs.currency {
            caveats.push(format!(
                "{} is held in {} and this report is in {}; it is listed at its own currency and excluded from the total",
                holding.instrument, holding.currency, inputs.currency
            ));
        }
        let profile = profiles.get(holding.instrument.as_str());
        let observation = prices.get(holding.instrument.as_str());
        // A market price in a different currency from the holding must not value
        // it. Measured 2026-09-05: a EUR holding priced by a USD market quote was
        // valued at the USD number with no conversion and no marker, which is a
        // wrong figure that looks right. Converting here is not the fix either --
        // the FX rate is a second observation with its own date, and layering it
        // in silently would hide two provenance facts behind one number. So the
        // position falls back to its reviewed price and the mismatch is named.
        let currency_matches = observation
            .map(|observation| observation.currency == holding.currency)
            .unwrap_or(true);
        if !currency_matches {
            caveats.push(format!(
                "{} is held in {} and its market price is quoted in {}; it is valued at its reviewed price until an FX conversion with its own dated rate exists",
                holding.instrument,
                holding.currency,
                observation.map(|o| o.currency.as_str()).unwrap_or("")
            ));
        }
        let observation = observation.filter(|_| currency_matches);
        let age_days = observation
            .and_then(|observation| clock::days_between(&observation.observed_on, inputs.as_of));
        let market_price = observation.map(|observation| observation.price.clone());
        let (unit_price, value_basis) = match (&market_price, &holding.latest_unit_price) {
            (Some(price), _) => (Some(price.clone()), "market"),
            (None, Some(price)) => (Some(price.clone()), "review"),
            (None, None) => (None, "none"),
        };
        let value = match &unit_price {
            Some(price) => investment::position_value(&holding.quantity, price)
                .map_err(|error| error.to_string())?,
            None => DecimalValue {
                mantissa: 0,
                scale: 0,
            },
        };
        if unit_price.is_some() {
            priced += 1;
        } else {
            unpriced += 1;
        }
        let cents = investment::to_minor_units(&value).map_err(|error| error.to_string())?;
        if holding.currency == inputs.currency {
            total_cents += cents;
        }
        let (change_since_review, change_since_review_bp) =
            match (&market_price, &holding.latest_unit_price) {
                (Some(market), Some(review)) => {
                    let market_value = investment::position_value(&holding.quantity, market)
                        .map_err(|error| error.to_string())?;
                    let review_value = investment::position_value(&holding.quantity, review)
                        .map_err(|error| error.to_string())?;
                    let market_cents = investment::to_minor_units(&market_value)
                        .map_err(|error| error.to_string())?;
                    let review_cents = investment::to_minor_units(&review_value)
                        .map_err(|error| error.to_string())?;
                    let difference = market_cents - review_cents;
                    let ratio_bp = (review_cents != 0)
                        .then(|| (difference * i128::from(FULL_ALLOCATION_BP)) / review_cents)
                        .and_then(|value| i64::try_from(value).ok());
                    (Some(investment::from_minor_units(difference)), ratio_bp)
                }
                _ => (None, None),
            };
        positions.push(PositionView {
            instrument: holding.instrument.clone(),
            label: profile
                .map(|profile| profile.label.clone())
                .unwrap_or_else(|| holding.instrument.clone()),
            asset_class: profile
                .map(|profile| profile.asset_class.clone())
                .unwrap_or_else(|| "unclassified".into()),
            quantity: holding.quantity.clone(),
            currency: holding.currency.clone(),
            review_price: holding.latest_unit_price.clone(),
            market_price,
            market_price_source: observation.map(|observation| observation.source.clone()),
            market_price_observed_on: observation
                .map(|observation| observation.observed_on.clone()),
            price_age_days: age_days,
            price_freshness: freshness(value_basis, age_days, freshness_days).into(),
            value,
            value_basis: value_basis.into(),
            // Filled in below, once the total exists.
            share_bp: 0,
            target_bp: None,
            band_bp: None,
            drift_bp: None,
            outside_band: false,
            change_since_review,
            change_since_review_bp,
        });
    }

    for position in &mut positions {
        if position.currency != inputs.currency {
            continue;
        }
        let cents =
            investment::to_minor_units(&position.value).map_err(|error| error.to_string())?;
        position.share_bp = share_bp(cents, total_cents);
    }

    let mut class_cents: BTreeMap<String, i128> = BTreeMap::new();
    for position in &positions {
        if position.currency != inputs.currency {
            continue;
        }
        let cents =
            investment::to_minor_units(&position.value).map_err(|error| error.to_string())?;
        *class_cents.entry(position.asset_class.clone()).or_insert(0) += cents;
    }

    let targets_configured = inputs
        .targets
        .is_some_and(|targets| !targets.allocations.is_empty());
    let cohort = inputs
        .targets
        .map(|targets| active_cohort(targets, inputs.as_of))
        .unwrap_or_default();
    let cohort_sum: i64 = cohort.iter().map(|allocation| allocation.target_bp).sum();
    let cohort_sums =
        !cohort.is_empty() && (cohort_sum - FULL_ALLOCATION_BP).abs() <= COHORT_TOLERANCE_BP;
    if !cohort.is_empty() && !cohort_sums {
        caveats.push(format!(
            "the target allocations active on {} sum to {cohort_sum} bp, not {FULL_ALLOCATION_BP}; drift is not measured against a policy that does not add up",
            inputs.as_of
        ));
    }

    if cohort_sums {
        for allocation in &cohort {
            match allocation.scope.as_str() {
                "instrument" => {
                    let Some(instrument) = allocation.instrument.as_deref() else {
                        caveats.push(format!(
                            "target {} is scoped to an instrument and names none",
                            allocation.id
                        ));
                        continue;
                    };
                    let Some(position) = positions
                        .iter_mut()
                        .find(|position| position.instrument == instrument)
                    else {
                        caveats.push(format!(
                            "target {} names an instrument that is not in the reviewed snapshot",
                            allocation.id
                        ));
                        continue;
                    };
                    apply_target(
                        &mut position.target_bp,
                        &mut position.band_bp,
                        &mut position.drift_bp,
                        &mut position.outside_band,
                        position.share_bp,
                        allocation,
                    );
                }
                "asset_class" => {}
                other => caveats.push(format!(
                    "target {} has scope {other:?}; only instrument and asset_class are read",
                    allocation.id
                )),
            }
        }
    }

    let mut asset_classes: Vec<AssetClassView> = class_cents
        .into_iter()
        .map(|(asset_class, cents)| AssetClassView {
            asset_class,
            value: investment::from_minor_units(cents),
            share_bp: share_bp(cents, total_cents),
            target_bp: None,
            band_bp: None,
            drift_bp: None,
            outside_band: false,
        })
        .collect();

    if cohort_sums {
        for allocation in cohort.iter().filter(|a| a.scope == "asset_class") {
            let Some(asset_class) = allocation.asset_class.as_deref() else {
                caveats.push(format!(
                    "target {} is scoped to an asset class and names none",
                    allocation.id
                ));
                continue;
            };
            let Some(view) = asset_classes
                .iter_mut()
                .find(|view| view.asset_class == asset_class)
            else {
                caveats.push(format!(
                    "target {} names asset class {asset_class:?}, which nothing held is in",
                    allocation.id
                ));
                continue;
            };
            apply_target(
                &mut view.target_bp,
                &mut view.band_bp,
                &mut view.drift_bp,
                &mut view.outside_band,
                view.share_bp,
                allocation,
            );
        }
    }
    asset_classes.sort_by(|left, right| left.asset_class.cmp(&right.asset_class));

    if inputs.snapshot.coverage != HoldingsCoverage::Complete {
        caveats.push(
            "the reviewed holdings snapshot is partial, so every share is a share of what was imported".into(),
        );
    }
    if unpriced > 0 {
        caveats.push(format!(
            "{unpriced} holding(s) carry no price at all and count as zero in the total"
        ));
    }

    Ok(PortfolioReport {
        as_of: inputs.as_of.to_string(),
        currency: inputs.currency.to_string(),
        coverage: inputs.snapshot.coverage.as_str().to_string(),
        targets_configured,
        total: investment::from_minor_units(total_cents),
        priced_positions: priced,
        unpriced_positions: unpriced,
        positions,
        asset_classes,
        caveats,
    })
}

fn apply_target(
    target_bp: &mut Option<i64>,
    band_bp: &mut Option<i64>,
    drift_bp: &mut Option<i64>,
    outside_band: &mut bool,
    share_bp: i64,
    allocation: &crate::config::TargetAllocation,
) {
    let drift = share_bp - allocation.target_bp;
    *target_bp = Some(allocation.target_bp);
    *band_bp = Some(allocation.band_bp);
    *drift_bp = Some(drift);
    *outside_band = drift.abs() > allocation.band_bp;
}

/// The allocations active on a date, copying `RecurringCommitment::active_on`'s
/// dating so a cohort is "the allocations active on this date" and nothing else.
pub fn active_cohort(targets: &TargetPolicy, as_of: &str) -> Vec<crate::config::TargetAllocation> {
    targets
        .allocations
        .iter()
        .filter(|allocation| allocation.active_on(as_of))
        .cloned()
        .collect()
}

fn share_bp(cents: i128, total_cents: i128) -> i64 {
    if total_cents == 0 {
        return 0;
    }
    i64::try_from((cents * i128::from(FULL_ALLOCATION_BP)) / total_cents).unwrap_or(0)
}

fn freshness(value_basis: &str, age_days: Option<i64>, threshold: i64) -> &'static str {
    match (value_basis, age_days) {
        ("market", Some(age)) if age <= threshold => "fresh",
        ("market", Some(_)) => "stale",
        ("market", None) => "stale",
        _ => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TargetAllocation;
    use crate::investment::{Holding, ReviewedHoldingsSnapshot};

    fn quantity(mantissa: i64, scale: u32) -> Quantity {
        Quantity { mantissa, scale }
    }

    fn snapshot(holdings: Vec<Holding>) -> ReviewedHoldingsSnapshot {
        ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-09-01".into(),
            coverage: HoldingsCoverage::Complete,
            holdings,
            sources: Vec::new(),
        }
    }

    fn holding(instrument: &str, quantity_units: i64, price: i64) -> Holding {
        Holding {
            instrument: instrument.into(),
            quantity: quantity(quantity_units, 0),
            latest_unit_price: Some(quantity(price, 2)),
            currency: "EUR".into(),
        }
    }

    #[test]
    fn shares_are_basis_points_of_the_total_and_add_up() {
        let holdings = vec![holding("SYN-A", 100, 10_000), holding("SYN-B", 100, 10_000)];
        let report = report(PortfolioInputs {
            snapshot: &snapshot(holdings),
            latest_prices: &[],
            instruments: &[],
            targets: None,
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        assert_eq!(report.positions.len(), 2);
        assert_eq!(report.positions[0].share_bp, 5_000);
        assert_eq!(report.positions[1].share_bp, 5_000);
        assert!(!report.targets_configured);
        assert_eq!(report.positions[0].value_basis, "review");
        assert_eq!(report.positions[0].price_freshness, "none");
    }

    /// One definition of what the portfolio is worth.
    ///
    /// With no market observation, `portfolio::report`'s total must equal
    /// `investment::portfolio_valuations`' figure for the same currency -- the
    /// number `/api/dashboard` serves. With a market observation the two answer
    /// different questions on purpose, and `value_basis` on every position is how
    /// a reader tells which they are looking at.
    #[test]
    fn the_review_basis_total_equals_the_dashboard_valuation() {
        let holdings = vec![holding("SYN-A", 100, 10_000), holding("SYN-B", 7, 12_345)];
        let books = snapshot(holdings);
        let report = report(PortfolioInputs {
            snapshot: &books,
            latest_prices: &[],
            instruments: &[],
            targets: None,
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        let valuations = crate::investment::portfolio_valuations(&books).expect("a valuation");
        let dashboard = valuations
            .iter()
            .find(|value| value.currency == "EUR")
            .expect("a EUR valuation");
        assert_eq!(
            crate::investment::to_minor_units(&report.total).unwrap(),
            crate::investment::to_minor_units(&dashboard.value).unwrap()
        );
    }

    #[test]
    fn a_market_price_overrides_the_review_price_and_says_so() {
        let observation = PriceObservation {
            instrument: "SYN-A".into(),
            observed_on: "2026-09-04".into(),
            price: quantity(12_000, 2),
            currency: "EUR".into(),
            source: "yahoo".into(),
            fetched_at: "2026-09-04T10:00:00Z".into(),
        };
        let report = report(PortfolioInputs {
            snapshot: &snapshot(vec![holding("SYN-A", 10, 10_000)]),
            latest_prices: std::slice::from_ref(&observation),
            instruments: &[],
            targets: None,
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        let position = &report.positions[0];
        assert_eq!(position.value_basis, "market");
        assert_eq!(position.market_price_source.as_deref(), Some("yahoo"));
        assert_eq!(position.price_age_days, Some(1));
        assert_eq!(position.price_freshness, "fresh");
        // 10 units at 120.00 is 1,200.00 against a review value of 1,000.00.
        assert_eq!(position.change_since_review_bp, Some(2_000));
    }

    /// Measured 2026-09-05: a EUR holding was valued at a USD market quote with
    /// no conversion and no marker.
    #[test]
    fn a_market_price_in_another_currency_does_not_value_the_position() {
        let observation = PriceObservation {
            instrument: "SYN-A".into(),
            observed_on: "2026-09-04".into(),
            price: quantity(77_000_000, 4),
            currency: "USD".into(),
            source: "yahoo".into(),
            fetched_at: "2026-09-04T10:00:00Z".into(),
        };
        let report = report(PortfolioInputs {
            snapshot: &snapshot(vec![holding("SYN-A", 10, 10_000)]),
            latest_prices: std::slice::from_ref(&observation),
            instruments: &[],
            targets: None,
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        let position = &report.positions[0];
        assert_eq!(position.value_basis, "review");
        assert_eq!(position.market_price, None);
        assert_eq!(position.price_freshness, "none");
        assert!(report
            .caveats
            .iter()
            .any(|caveat| caveat.contains("quoted in USD")));
    }

    #[test]
    fn a_cohort_that_does_not_sum_to_ten_thousand_measures_no_drift_and_names_the_sum() {
        let targets = TargetPolicy {
            allocations: vec![TargetAllocation {
                id: "synthetic-equity".into(),
                label: "Synthetic equity".into(),
                scope: "instrument".into(),
                instrument: Some("SYN-A".into()),
                asset_class: None,
                target_bp: 9_400,
                band_bp: 500,
                rationale: String::new(),
                valid_from: "2026-01-01".into(),
                valid_until: None,
            }],
            ..TargetPolicy::default()
        };
        let report = report(PortfolioInputs {
            snapshot: &snapshot(vec![holding("SYN-A", 100, 10_000)]),
            latest_prices: &[],
            instruments: &[],
            targets: Some(&targets),
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        assert!(report.targets_configured);
        assert_eq!(report.positions[0].drift_bp, None);
        assert!(report
            .caveats
            .iter()
            .any(|caveat| caveat.contains("9400 bp")));
    }

    #[test]
    fn drift_outside_the_band_names_both_numbers() {
        let targets = TargetPolicy {
            allocations: vec![
                TargetAllocation {
                    id: "synthetic-a".into(),
                    label: "Synthetic A".into(),
                    scope: "instrument".into(),
                    instrument: Some("SYN-A".into()),
                    asset_class: None,
                    target_bp: 4_000,
                    band_bp: 200,
                    rationale: String::new(),
                    valid_from: "2026-01-01".into(),
                    valid_until: None,
                },
                TargetAllocation {
                    id: "synthetic-b".into(),
                    label: "Synthetic B".into(),
                    scope: "instrument".into(),
                    instrument: Some("SYN-B".into()),
                    asset_class: None,
                    target_bp: 6_000,
                    band_bp: 200,
                    rationale: String::new(),
                    valid_from: "2026-01-01".into(),
                    valid_until: None,
                },
            ],
            ..TargetPolicy::default()
        };
        let report = report(PortfolioInputs {
            snapshot: &snapshot(vec![
                holding("SYN-A", 100, 10_000),
                holding("SYN-B", 100, 10_000),
            ]),
            latest_prices: &[],
            instruments: &[],
            targets: Some(&targets),
            as_of: "2026-09-05",
            currency: "EUR",
        })
        .expect("a report");
        let a = &report.positions[0];
        assert_eq!(a.target_bp, Some(4_000));
        assert_eq!(a.band_bp, Some(200));
        assert_eq!(a.drift_bp, Some(1_000));
        assert!(a.outside_band);
        let b = &report.positions[1];
        assert_eq!(b.drift_bp, Some(-1_000));
        assert!(b.outside_band);
    }
}

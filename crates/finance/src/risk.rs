//! Rung 2: volatility, correlation and mean-variance weights.
//!
//! **This rung explains. It does not propose.** Every proposal in the ledger
//! carries `rung = "rule"`, and `decision.rs` attaches this report as evidence
//! rather than reading it as a trigger. That is Principle 1 made structural: the
//! cheapest mechanism decides, and the expensive one is asked to justify.
//!
//! **The floors are refusals, not warnings.** Below 120 daily observations for
//! an instrument, or 60 overlapping dates for a pair, the answer names the actual
//! count and the figure is `null`. Never a zero: a zero volatility is a claim
//! that a price never moved, and a zero correlation claims an independence
//! nobody measured. On the day this ships the `broker` provider has written one
//! observation per instrument per run, so `insufficient_history` is the correct
//! and expected answer for every row until a real history accumulates.
//!
//! **A singular covariance is reported, never regularised.** The Cholesky's
//! non-positive pivot *is* the guard, so the solver and the refusal are one code
//! path. Nothing is added to the diagonal to make a matrix invertible: that would
//! answer a different question and label the answer with this one's name.
//!
//! **Max-Sharpe without a configured risk-free rate is omitted with a reason.**
//! Defaulting it to zero is a claim about the world dressed as a missing value.
//!
//! The linear algebra is hand-rolled, and that is a decision rather than an
//! oversight: `nalgebra`, `ndarray` and `statrs` all have zero entries in
//! `Cargo.lock` (checked 2026-09-05), the problem is a handful of instruments,
//! and `upstreams.toml`'s "No entry, no entry" makes a dependency a decision
//! rather than an import.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::config::TargetPolicy;
use crate::investment::ReviewedHoldingsSnapshot;
use crate::price::PriceObservation;
use crate::store::FinanceStore;

/// Trading days in a year. The conventional 252, named rather than inlined so
/// the annualisation is one number a reader can find.
pub const TRADING_DAYS: f64 = 252.0;

/// Daily observations below which an instrument gets no figure at all.
pub const MIN_OBSERVATIONS: usize = 120;

/// Overlapping dates below which a pair gets no correlation.
pub const MIN_OVERLAP: usize = 60;

/// A Cholesky pivot below this fraction of its diagonal entry is a singular
/// matrix, not a small one. See [`cholesky`] for the measurement behind it.
const PIVOT_TOLERANCE: f64 = 1e-10;

/// Basis points, so the wire carries integers and the frontend does no
/// arithmetic. `1.0` (100%) is 10,000 bp.
fn to_bp(value: f64) -> Option<i64> {
    value
        .is_finite()
        .then(|| (value * 10_000.0).round())
        .filter(|value| value.abs() < 9.0e18)
        .map(|value| value as i64)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstrumentRisk {
    pub instrument: String,
    pub observations: usize,
    /// `null` below the floor. Annualised standard deviation of daily log
    /// returns, in basis points.
    pub annualised_volatility_bp: Option<i64>,
    /// `null` below the floor. Annualised mean of daily log returns.
    pub annualised_mean_bp: Option<i64>,
    /// `ok` or `insufficient_history`.
    pub status: String,
    /// Present only when the figure is refused, and it names the actual count.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairCorrelation {
    pub left: String,
    pub right: String,
    pub overlap: usize,
    /// `null` below the floor, and `null` rather than 0 -- a zero here claims an
    /// independence nobody measured.
    pub correlation_bp: Option<i64>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Weight {
    pub instrument: String,
    pub weight_bp: i64,
    pub current_bp: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RiskReport {
    pub trading_days: i64,
    pub min_observations: usize,
    pub min_overlap: usize,
    pub instruments: Vec<InstrumentRisk>,
    pub correlations: Vec<PairCorrelation>,
    /// Volatility of the portfolio at its current weights, `null` when any
    /// contributing instrument is below the floor or the covariance is singular.
    pub portfolio_volatility_bp: Option<i64>,
    pub minimum_variance_weights: Option<Vec<Weight>>,
    pub max_sharpe_weights: Option<Vec<Weight>>,
    pub caveats: Vec<String>,
}

/// The report, read straight from the price series.
pub fn report(
    store: &FinanceStore,
    snapshot: &ReviewedHoldingsSnapshot,
    targets: Option<&TargetPolicy>,
) -> Result<RiskReport, String> {
    let observations = store.all_prices().map_err(|error| error.to_string())?;
    Ok(from_observations(&observations, snapshot, targets))
}

/// The same report from a series in memory, so the model is testable without a
/// database.
pub fn from_observations(
    observations: &[PriceObservation],
    snapshot: &ReviewedHoldingsSnapshot,
    targets: Option<&TargetPolicy>,
) -> RiskReport {
    let mut caveats = Vec::new();
    let held: BTreeSet<&str> = snapshot
        .holdings
        .iter()
        .map(|holding| holding.instrument.as_str())
        .collect();
    let series = daily_series(observations, &held);
    let returns: BTreeMap<String, BTreeMap<String, f64>> = series
        .iter()
        .map(|(instrument, points)| (instrument.clone(), log_returns(points)))
        .collect();

    let instruments: Vec<InstrumentRisk> = returns
        .iter()
        .map(|(instrument, series)| {
            let count = series.len();
            if count < MIN_OBSERVATIONS {
                return InstrumentRisk {
                    instrument: instrument.clone(),
                    observations: count,
                    annualised_volatility_bp: None,
                    annualised_mean_bp: None,
                    status: "insufficient_history".into(),
                    detail: Some(format!(
                        "{count} daily observation(s) against a floor of {MIN_OBSERVATIONS}"
                    )),
                };
            }
            let values: Vec<f64> = series.values().copied().collect();
            InstrumentRisk {
                instrument: instrument.clone(),
                observations: count,
                annualised_volatility_bp: to_bp(standard_deviation(&values) * TRADING_DAYS.sqrt()),
                annualised_mean_bp: to_bp(mean(&values) * TRADING_DAYS),
                status: "ok".into(),
                detail: None,
            }
        })
        .collect();

    let names: Vec<&String> = returns.keys().collect();
    let mut correlations = Vec::new();
    for (index, left) in names.iter().enumerate() {
        for right in names.iter().skip(index + 1) {
            let (a, b) = aligned(&returns[*left], &returns[*right]);
            let overlap = a.len();
            let (correlation_bp, status) = if overlap < MIN_OVERLAP {
                (None, "insufficient_history")
            } else {
                (to_bp(correlation(&a, &b)), "ok")
            };
            correlations.push(PairCorrelation {
                left: (*left).clone(),
                right: (*right).clone(),
                overlap,
                correlation_bp,
                status: status.into(),
            });
        }
    }

    // Only instruments that cleared the floor may enter the covariance. Mixing a
    // measured series with a guessed one is how a matrix acquires a number no
    // observation supports.
    let usable: Vec<String> = instruments
        .iter()
        .filter(|risk| risk.status == "ok")
        .map(|risk| risk.instrument.clone())
        .collect();
    if usable.len() < 2 {
        caveats.push(format!(
            "{} instrument(s) clear the {MIN_OBSERVATIONS}-observation floor; a covariance needs at least two",
            usable.len()
        ));
        return RiskReport {
            trading_days: TRADING_DAYS as i64,
            min_observations: MIN_OBSERVATIONS,
            min_overlap: MIN_OVERLAP,
            instruments,
            correlations,
            portfolio_volatility_bp: None,
            minimum_variance_weights: None,
            max_sharpe_weights: None,
            caveats,
        };
    }

    let covariance = annualised_covariance(&usable, &returns);
    let Some(cholesky) = cholesky(&covariance) else {
        caveats.push(
            "the covariance matrix is singular: two series move together exactly over the shared window, so no unique minimum-variance portfolio exists. Reported rather than regularised -- adding to the diagonal would answer a different question".into(),
        );
        return RiskReport {
            trading_days: TRADING_DAYS as i64,
            min_observations: MIN_OBSERVATIONS,
            min_overlap: MIN_OVERLAP,
            instruments,
            correlations,
            portfolio_volatility_bp: None,
            minimum_variance_weights: None,
            max_sharpe_weights: None,
            caveats,
        };
    };

    let current: BTreeMap<&str, f64> = current_weights(snapshot);
    let portfolio_volatility_bp = {
        let weights: Vec<f64> = usable
            .iter()
            .map(|name| current.get(name.as_str()).copied().unwrap_or(0.0))
            .collect();
        let total: f64 = weights.iter().sum();
        (total > 0.0)
            .then(|| {
                let normalised: Vec<f64> = weights.iter().map(|w| w / total).collect();
                to_bp(quadratic_form(&covariance, &normalised).max(0.0).sqrt())
            })
            .flatten()
    };
    if portfolio_volatility_bp.is_none() {
        caveats.push(
            "no portfolio volatility: none of the instruments with enough history carries a positive weight today".into(),
        );
    }

    let minimum_variance_weights = long_only_weights(&cholesky, &usable, &vec![1.0; usable.len()])
        .map(|weights| label(&usable, &weights, &current));
    if minimum_variance_weights.is_none() {
        caveats.push("no long-only minimum-variance portfolio: the active set emptied".into());
    }

    let max_sharpe_weights = match targets.and_then(|targets| targets.risk_free_rate_bp) {
        None => {
            caveats.push(
                "max-Sharpe is omitted: no risk_free_rate_bp is configured, and defaulting it to zero would be a claim about the world rather than a missing value".into(),
            );
            None
        }
        Some(rate_bp) => {
            let daily_rf = (rate_bp as f64 / 10_000.0) / TRADING_DAYS;
            let excess: Vec<f64> = usable
                .iter()
                .map(|name| (mean_of(&returns[name]) - daily_rf) * TRADING_DAYS)
                .collect();
            if excess.iter().all(|value| *value <= 0.0) {
                caveats.push(
                    "max-Sharpe is omitted: no instrument's annualised mean exceeds the configured risk-free rate over the measured window".into(),
                );
                None
            } else {
                long_only_weights(&cholesky, &usable, &excess)
                    .map(|weights| label(&usable, &weights, &current))
            }
        }
    };

    RiskReport {
        trading_days: TRADING_DAYS as i64,
        min_observations: MIN_OBSERVATIONS,
        min_overlap: MIN_OVERLAP,
        instruments,
        correlations,
        portfolio_volatility_bp,
        minimum_variance_weights,
        max_sharpe_weights,
        caveats,
    }
}

// ---------------------------------------------------------------------------
// The series
// ---------------------------------------------------------------------------

/// One price per instrument-day, from ONE source per instrument.
///
/// The source rule is the load-bearing half, and it was bought with a live
/// measurement rather than reasoned: on 2026-09-05 an instrument carried 502
/// `yahoo` observations in USD and one `broker` observation in EUR, and the
/// broker row landed inside the yahoo window. Mixing them produced a −87% day
/// followed by a +666% day, an annualised portfolio volatility of 152%, and a
/// minimum-variance portfolio computed from returns that never happened.
///
/// Two sources are two price scales — a different currency, a different
/// adjustment basis, a different close convention — so a switch between them is
/// a fabricated return, not a data point. The series therefore uses the source
/// with the MOST observations for that instrument, ties broken by the newest
/// observed day. A day is never averaged across sources either: an average of two
/// quotes is a third number nobody published.
///
/// The consequence is stated rather than hidden: an instrument priced only by
/// `broker` has a one-point-per-run series and will sit at
/// `insufficient_history` for a long time. That is the correct answer, and it is
/// why the `yahoo` provider exists.
fn daily_series(
    observations: &[PriceObservation],
    held: &BTreeSet<&str>,
) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut counts: BTreeMap<(&str, &str), (usize, &str)> = BTreeMap::new();
    for observation in observations {
        if !held.contains(observation.instrument.as_str()) {
            continue;
        }
        let entry = counts
            .entry((observation.instrument.as_str(), observation.source.as_str()))
            .or_insert((0, ""));
        entry.0 += 1;
        if observation.observed_on.as_str() > entry.1 {
            entry.1 = observation.observed_on.as_str();
        }
    }
    let mut best: BTreeMap<&str, (usize, &str, &str)> = BTreeMap::new();
    for ((instrument, source), (count, newest)) in counts {
        let candidate = (count, newest, source);
        match best.get(instrument) {
            Some((best_count, best_newest, _))
                if (*best_count, *best_newest) >= (count, newest) => {}
            _ => {
                best.insert(instrument, candidate);
            }
        }
    }
    let mut chosen: BTreeMap<(String, String), (&str, f64)> = BTreeMap::new();
    for observation in observations {
        if !held.contains(observation.instrument.as_str()) {
            continue;
        }
        let Some((_, _, source)) = best.get(observation.instrument.as_str()) else {
            continue;
        };
        if *source != observation.source.as_str() {
            continue;
        }
        let Some(price) = decimal_to_f64(observation.price.mantissa, observation.price.scale)
        else {
            continue;
        };
        if price <= 0.0 {
            continue;
        }
        let key = (
            observation.instrument.clone(),
            observation.observed_on.clone(),
        );
        match chosen.get(&key) {
            Some((fetched_at, _)) if *fetched_at >= observation.fetched_at.as_str() => {}
            _ => {
                chosen.insert(key, (observation.fetched_at.as_str(), price));
            }
        }
    }
    let mut series: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for ((instrument, day), (_, price)) in chosen {
        series.entry(instrument).or_default().insert(day, price);
    }
    series
}

fn decimal_to_f64(mantissa: i64, scale: u32) -> Option<f64> {
    let divisor = 10_f64.powi(i32::try_from(scale).ok()?);
    (divisor > 0.0).then(|| mantissa as f64 / divisor)
}

/// Daily log returns, keyed by the later of the two dates.
///
/// Log rather than simple returns because they add across time, which is what
/// makes annualising a mean by multiplication correct rather than approximate.
/// Consecutive stored observations, not consecutive calendar days: a gap in the
/// series is a longer holding period and is not filled in.
fn log_returns(points: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    let mut returns = BTreeMap::new();
    let mut previous: Option<f64> = None;
    for (day, price) in points {
        if let Some(previous) = previous {
            if previous > 0.0 && *price > 0.0 {
                returns.insert(day.clone(), (price / previous).ln());
            }
        }
        previous = Some(*price);
    }
    returns
}

/// The two series restricted to the dates they share, in one order.
fn aligned(left: &BTreeMap<String, f64>, right: &BTreeMap<String, f64>) -> (Vec<f64>, Vec<f64>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for (day, value) in left {
        if let Some(other) = right.get(day) {
            a.push(*value);
            b.push(*other);
        }
    }
    (a, b)
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn mean_of(series: &BTreeMap<String, f64>) -> f64 {
    let values: Vec<f64> = series.values().copied().collect();
    mean(&values)
}

/// Sample standard deviation, `n - 1`. The sample divisor, because these are
/// observations of a process rather than a whole population.
fn standard_deviation(values: &[f64]) -> f64 {
    variance(values).sqrt()
}

fn variance(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    values.iter().map(|value| (value - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64
}

fn covariance_of(a: &[f64], b: &[f64]) -> f64 {
    if a.len() < 2 || a.len() != b.len() {
        return 0.0;
    }
    let (ma, mb) = (mean(a), mean(b));
    a.iter()
        .zip(b)
        .map(|(left, right)| (left - ma) * (right - mb))
        .sum::<f64>()
        / (a.len() - 1) as f64
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let denominator = standard_deviation(a) * standard_deviation(b);
    if denominator == 0.0 {
        return f64::NAN;
    }
    covariance_of(a, b) / denominator
}

/// The annualised covariance over the dates every listed instrument shares.
///
/// One common window rather than pairwise windows: a matrix assembled from
/// entries measured over different periods is not a covariance matrix of
/// anything, and it can fail to be positive semi-definite for that reason alone.
fn annualised_covariance(
    names: &[String],
    returns: &BTreeMap<String, BTreeMap<String, f64>>,
) -> Vec<Vec<f64>> {
    let mut common: Option<BTreeSet<String>> = None;
    for name in names {
        let days: BTreeSet<String> = returns[name].keys().cloned().collect();
        common = Some(match common {
            None => days,
            Some(existing) => existing.intersection(&days).cloned().collect(),
        });
    }
    let common = common.unwrap_or_default();
    let columns: Vec<Vec<f64>> = names
        .iter()
        .map(|name| common.iter().map(|day| returns[name][day]).collect())
        .collect();
    let size = names.len();
    let mut matrix = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..size {
            matrix[row][column] = covariance_of(&columns[row], &columns[column]) * TRADING_DAYS;
        }
    }
    matrix
}

// ---------------------------------------------------------------------------
// The solver
// ---------------------------------------------------------------------------

/// The lower-triangular Cholesky factor, or `None` when the matrix is not
/// positive definite.
///
/// The `None` is the whole guard. A non-positive pivot means the covariance is
/// singular or indefinite, and there is no unique minimum-variance portfolio to
/// report; the caller says so rather than nudging the diagonal until the
/// arithmetic succeeds.
///
/// The pivot is compared against a RELATIVE floor, not against zero, and the
/// reason is measured rather than theoretical: two exactly identical series give
/// a pivot of `v - (v/√v)²`, which is zero in exact arithmetic and a rounding
/// residue near 1e-19 in floating point. Testing `> 0.0` accepted that residue
/// and produced a 48.85/51.15 split out of pure noise -- an answer with the shape
/// of a finding and none of the content. `PIVOT_TOLERANCE` is what makes the
/// refusal real. It is NOT a regularisation: nothing is added to the matrix, and
/// a matrix that fails here gets no portfolio at all.
pub fn cholesky(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let size = matrix.len();
    let mut lower = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..=row {
            let mut sum = matrix[row][column];
            for (left, right) in lower[row][..column].iter().zip(&lower[column][..column]) {
                sum -= left * right;
            }
            if row == column {
                let tolerance = matrix[row][row].abs() * PIVOT_TOLERANCE;
                if !sum.is_finite() || sum <= tolerance || sum <= 0.0 {
                    return None;
                }
                lower[row][column] = sum.sqrt();
            } else {
                if lower[column][column] == 0.0 {
                    return None;
                }
                lower[row][column] = sum / lower[column][column];
            }
        }
    }
    Some(lower)
}

/// Solve `L Lᵀ x = b` by forward then back substitution.
fn solve(lower: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let size = lower.len();
    if b.len() != size {
        return None;
    }
    let mut y = vec![0.0; size];
    for row in 0..size {
        let mut sum = b[row];
        for column in 0..row {
            sum -= lower[row][column] * y[column];
        }
        if lower[row][row] == 0.0 {
            return None;
        }
        y[row] = sum / lower[row][row];
    }
    let mut x = vec![0.0; size];
    for row in (0..size).rev() {
        let mut sum = y[row];
        for column in (row + 1)..size {
            sum -= lower[column][row] * x[column];
        }
        x[row] = sum / lower[row][row];
    }
    Some(x)
}

/// Long-only weights by iterative constraint activation.
///
/// `w ∝ Σ⁻¹ b`, normalised to sum to one; any negative weight has its instrument
/// pinned to zero and the remainder is re-solved. `b = 1` gives the
/// minimum-variance portfolio, `b = μ − r_f` the tangency portfolio.
///
/// Honest about what this is: dropping the negative names and re-solving is the
/// simple active-set heuristic, not a proved-optimal QP. For a handful of
/// instruments it lands on the same answer in practice, and the alternative is a
/// solver dependency `upstreams.toml` would have to justify. It terminates
/// because each pass pins at least one instrument.
fn long_only_weights(cholesky: &[Vec<f64>], names: &[String], b: &[f64]) -> Option<Vec<f64>> {
    let size = names.len();
    let mut active: Vec<usize> = (0..size).collect();
    for _ in 0..size {
        if active.is_empty() {
            return None;
        }
        let sub_l = sub_cholesky(cholesky, &active)?;
        let sub_b: Vec<f64> = active.iter().map(|index| b[*index]).collect();
        let raw = solve(&sub_l, &sub_b)?;
        let total: f64 = raw.iter().sum();
        if total == 0.0 || !total.is_finite() {
            return None;
        }
        let normalised: Vec<f64> = raw.iter().map(|value| value / total).collect();
        if normalised.iter().all(|value| *value >= -1e-9) {
            let mut weights = vec![0.0; size];
            for (slot, index) in active.iter().enumerate() {
                weights[*index] = normalised[slot].max(0.0);
            }
            return Some(weights);
        }
        let worst = normalised
            .iter()
            .enumerate()
            .min_by(|left, right| left.1.total_cmp(right.1))?
            .0;
        active.remove(worst);
    }
    None
}

/// The Cholesky factor of the sub-matrix on `active`.
///
/// Refactored from the full factor rather than sliced out of it: a Cholesky
/// factor's rows are not independent, so `L`'s submatrix is not the submatrix's
/// `L`.
fn sub_cholesky(lower: &[Vec<f64>], active: &[usize]) -> Option<Vec<Vec<f64>>> {
    let full = reconstruct(lower);
    let sub: Vec<Vec<f64>> = active
        .iter()
        .map(|row| active.iter().map(|column| full[*row][*column]).collect())
        .collect();
    cholesky(&sub)
}

fn reconstruct(lower: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let size = lower.len();
    let mut matrix = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..size {
            matrix[row][column] = (0..size).map(|k| lower[row][k] * lower[column][k]).sum();
        }
    }
    matrix
}

fn quadratic_form(matrix: &[Vec<f64>], weights: &[f64]) -> f64 {
    let mut total = 0.0;
    for (row, weight_row) in weights.iter().enumerate() {
        for (column, weight_column) in weights.iter().enumerate() {
            total += weight_row * matrix[row][column] * weight_column;
        }
    }
    total
}

/// Current weights by value, from the reviewed snapshot's own prices.
fn current_weights(snapshot: &ReviewedHoldingsSnapshot) -> BTreeMap<&str, f64> {
    let mut values: BTreeMap<&str, f64> = BTreeMap::new();
    let mut total = 0.0;
    for holding in &snapshot.holdings {
        let (Some(price), Some(quantity)) = (
            holding
                .latest_unit_price
                .as_ref()
                .and_then(|price| decimal_to_f64(price.mantissa, price.scale)),
            decimal_to_f64(holding.quantity.mantissa, holding.quantity.scale),
        ) else {
            continue;
        };
        let value = price * quantity;
        values.insert(holding.instrument.as_str(), value);
        total += value;
    }
    if total <= 0.0 {
        return BTreeMap::new();
    }
    values
        .into_iter()
        .map(|(instrument, value)| (instrument, value / total))
        .collect()
}

fn label(names: &[String], weights: &[f64], current: &BTreeMap<&str, f64>) -> Vec<Weight> {
    names
        .iter()
        .zip(weights)
        .map(|(instrument, weight)| Weight {
            instrument: instrument.clone(),
            weight_bp: to_bp(*weight).unwrap_or(0),
            current_bp: current.get(instrument.as_str()).copied().and_then(to_bp),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investment::{Holding, HoldingsCoverage, Quantity};

    /// A market observation. The source matters: `daily_series` picks ONE source
    /// per instrument, so a fixture that labelled a long series `broker` would be
    /// testing the wrong thing.
    fn observation(instrument: &str, day: &str, price: f64) -> PriceObservation {
        PriceObservation {
            instrument: instrument.into(),
            observed_on: day.into(),
            price: Quantity {
                mantissa: (price * 10_000.0).round() as i64,
                scale: 4,
            },
            currency: "EUR".into(),
            source: "yahoo".into(),
            fetched_at: format!("{day}T00:00:00Z"),
        }
    }

    fn snapshot(instruments: &[&str]) -> ReviewedHoldingsSnapshot {
        ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-09-01".into(),
            coverage: HoldingsCoverage::Complete,
            holdings: instruments
                .iter()
                .map(|instrument| Holding {
                    instrument: (*instrument).into(),
                    quantity: Quantity {
                        mantissa: 100,
                        scale: 0,
                    },
                    latest_unit_price: Some(Quantity {
                        mantissa: 10_000,
                        scale: 2,
                    }),
                    currency: "EUR".into(),
                })
                .collect(),
            sources: Vec::new(),
        }
    }

    /// A deterministic wiggle, so the series is not constant and every run agrees.
    fn synthetic_series(instrument: &str, days: usize, seed: u64) -> Vec<PriceObservation> {
        let mut price = 100.0_f64;
        let mut state = seed;
        (0..days)
            .map(|index| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let step = ((state >> 33) % 200) as f64 / 10_000.0 - 0.01;
                price *= 1.0 + step;
                observation(
                    instrument,
                    &crate::clock::civil_from_days(20_000 + index as i64),
                    price,
                )
            })
            .collect()
    }

    #[test]
    fn one_observation_per_instrument_is_insufficient_history_and_names_the_count() {
        let observations = vec![observation("SYN-A", "2026-09-01", 100.0)];
        let report = from_observations(&observations, &snapshot(&["SYN-A"]), None);
        assert_eq!(report.instruments.len(), 1);
        assert_eq!(report.instruments[0].status, "insufficient_history");
        assert_eq!(report.instruments[0].annualised_volatility_bp, None);
        assert_eq!(report.instruments[0].annualised_mean_bp, None);
        assert!(report.instruments[0]
            .detail
            .as_deref()
            .expect("a detail")
            .contains("floor of 120"));
        assert_eq!(report.minimum_variance_weights, None);
        assert_eq!(report.portfolio_volatility_bp, None);
    }

    /// The defect a live run found: one broker point at a different price scale
    /// inside a long market series produced a −87% day and a +666% day.
    #[test]
    fn one_source_per_instrument_keeps_a_broker_point_out_of_a_market_series() {
        let mut observations = synthetic_series("SYN-A", 300, 7);
        // A broker observation on a day the market series already covers, at a
        // wholly different scale -- a EUR activity price beside USD closes.
        let collision_day = observations[150].observed_on.clone();
        observations.push(PriceObservation {
            instrument: "SYN-A".into(),
            observed_on: collision_day,
            price: Quantity {
                mantissa: 1,
                scale: 0,
            },
            currency: "EUR".into(),
            source: "broker".into(),
            fetched_at: "2026-09-05T00:00:00Z".into(),
        });
        let report = from_observations(&observations, &snapshot(&["SYN-A"]), None);
        let volatility = report.instruments[0]
            .annualised_volatility_bp
            .expect("a volatility");
        // Two fabricated returns of that size would put this in the thousands of
        // basis points; the deterministic series sits far below that.
        assert!(
            volatility < 5_000,
            "a broker point leaked into the market series: {volatility} bp"
        );
        assert_eq!(report.instruments[0].observations, 299);
    }

    #[test]
    fn a_missing_correlation_is_null_and_never_zero() {
        let mut observations = synthetic_series("SYN-A", 200, 7);
        observations.extend(synthetic_series("SYN-B", 10, 11));
        let report = from_observations(&observations, &snapshot(&["SYN-A", "SYN-B"]), None);
        let pair = &report.correlations[0];
        assert_eq!(pair.status, "insufficient_history");
        assert_eq!(pair.correlation_bp, None);
    }

    #[test]
    fn enough_history_produces_a_volatility_and_a_long_only_portfolio() {
        let mut observations = synthetic_series("SYN-A", 300, 7);
        observations.extend(synthetic_series("SYN-B", 300, 99));
        let report = from_observations(&observations, &snapshot(&["SYN-A", "SYN-B"]), None);
        for instrument in &report.instruments {
            assert_eq!(instrument.status, "ok", "{instrument:?}");
            assert!(instrument.annualised_volatility_bp.expect("a volatility") > 0);
        }
        assert_eq!(report.correlations[0].status, "ok");
        let weights = report
            .minimum_variance_weights
            .as_ref()
            .expect("a long-only portfolio");
        assert_eq!(weights.len(), 2);
        for weight in weights {
            assert!(weight.weight_bp >= 0, "long only: {weight:?}");
        }
        let total: i64 = weights.iter().map(|weight| weight.weight_bp).sum();
        assert!((total - 10_000).abs() <= 2, "weights sum to {total} bp");
        assert!(report.portfolio_volatility_bp.expect("a figure") > 0);
    }

    #[test]
    fn max_sharpe_is_omitted_with_a_reason_when_no_risk_free_rate_is_configured() {
        let mut observations = synthetic_series("SYN-A", 300, 7);
        observations.extend(synthetic_series("SYN-B", 300, 99));
        let report = from_observations(&observations, &snapshot(&["SYN-A", "SYN-B"]), None);
        assert!(report.max_sharpe_weights.is_none());
        assert!(report
            .caveats
            .iter()
            .any(|caveat| caveat.contains("risk_free_rate_bp")));
    }

    #[test]
    fn a_singular_covariance_is_reported_and_never_regularised() {
        // Two instruments with identical series: the covariance is exactly
        // singular, and the Cholesky's non-positive pivot is the guard.
        let base = synthetic_series("SYN-A", 300, 7);
        let mirror: Vec<PriceObservation> = base
            .iter()
            .map(|observation| PriceObservation {
                instrument: "SYN-B".into(),
                ..observation.clone()
            })
            .collect();
        let mut observations = base;
        observations.extend(mirror);
        let report = from_observations(&observations, &snapshot(&["SYN-A", "SYN-B"]), None);
        assert_eq!(report.minimum_variance_weights, None);
        assert!(report
            .caveats
            .iter()
            .any(|caveat| caveat.contains("singular")));
    }

    #[test]
    fn cholesky_refuses_a_matrix_that_is_not_positive_definite() {
        assert!(cholesky(&[vec![4.0, 2.0], vec![2.0, 3.0]]).is_some());
        assert!(cholesky(&[vec![1.0, 1.0], vec![1.0, 1.0]]).is_none());
        assert!(cholesky(&[vec![0.0]]).is_none());
        assert!(cholesky(&[vec![-1.0]]).is_none());
    }

    #[test]
    fn the_cholesky_solve_inverts_what_it_factorised() {
        let matrix = vec![vec![4.0, 1.0], vec![1.0, 3.0]];
        let lower = cholesky(&matrix).expect("positive definite");
        let x = solve(&lower, &[1.0, 1.0]).expect("a solution");
        // A x should be [1, 1] again.
        let ax0 = matrix[0][0] * x[0] + matrix[0][1] * x[1];
        let ax1 = matrix[1][0] * x[0] + matrix[1][1] * x[1];
        assert!((ax0 - 1.0).abs() < 1e-9, "{ax0}");
        assert!((ax1 - 1.0).abs() < 1e-9, "{ax1}");
    }
}

//! The one place that says what "in EUR" means, and refuses when it cannot say it.
//!
//! PRD Q103 (2026-09-09) rules that subscription money normalises to EUR. That ruling
//! needs a boundary rather than a sprinkling of `if currency == "EUR"`, because the
//! failure it exists to stop is not arithmetic. It is a figure carrying the wrong
//! three-letter label: a note that declares `currency: USD` while the block Sjel wrote
//! into it says `EUR / month`, which is a number somebody could act on.
//!
//! ## Convert, or refuse. Never assume.
//!
//! A conversion needs a rate, and a rate needs a source. When the ruling was made,
//! `finance_fx_rates` held no rows and `finance_price_fetches` recorded no attempt by
//! the `ecb` provider at all. The reason was structural rather than a missed schedule:
//! [`crate::price::run_named`] took its FX targets from the holdings snapshot alone,
//! so a store whose holdings were all EUR handed the provider an empty target list.
//! [`crate::price::fx_targets`] now adds every currency a subscription is priced in.
//!
//! So [`to_eur`] takes the rates it is given and returns [`NoRate`] when none covers
//! the pair. There is no default, no hard-coded 1.08, and no silent pass-through of a
//! foreign amount under a EUR label. A refusal that names the currency is a smaller
//! error than a plausible number.
//!
//! Everything here is pure. The store supplies the rates; this module does the
//! arithmetic and the declining.

use serde::Serialize;

use crate::investment::Quantity;
use crate::price::FxObservation;
use crate::subscription::{State, Subscription};

/// The currency Q103 declares. Every figure this capability presents as a single
/// total is in this currency or is refused.
pub const DECLARED_CURRENCY: &str = "EUR";

/// Whether a currency code is the declared one, case-insensitively.
pub fn is_declared(currency: &str) -> bool {
    currency.trim().eq_ignore_ascii_case(DECLARED_CURRENCY)
}

/// Which published rate produced a converted figure, so the number stays checkable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RateUsed {
    pub base: String,
    pub quote: String,
    pub observed_on: String,
    pub source: String,
}

/// An amount stated in EUR, and how it got there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EurAmount {
    /// Already EUR. No rate was consulted, so none can be wrong.
    Declared { cents: i64 },
    /// Converted from another currency with a published rate.
    Converted {
        cents: i64,
        from_currency: String,
        from_cents: i64,
        rate: RateUsed,
    },
}

impl EurAmount {
    pub fn cents(&self) -> i64 {
        match self {
            EurAmount::Declared { cents } => *cents,
            EurAmount::Converted { cents, .. } => *cents,
        }
    }
}

/// Why a figure could not be stated in EUR. Carries the amount so a caller can show
/// what it is refusing to convert rather than dropping it silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoRate {
    pub currency: String,
    pub amount_cents: i64,
    pub detail: String,
}

impl std::fmt::Display for NoRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for NoRate {}

/// State `amount_cents` of `currency` in EUR, using only the rates supplied.
///
/// An unknown pair, a rate of zero and a negative rate are all refusals. Zero is not a
/// rate: dividing by it is undefined, and a provider that publishes it has published a
/// gap, not a price.
pub fn to_eur(
    amount_cents: i64,
    currency: &str,
    rates: &[FxObservation],
) -> Result<EurAmount, NoRate> {
    let code = currency.trim().to_ascii_uppercase();
    if is_declared(&code) {
        return Ok(EurAmount::Declared {
            cents: amount_cents,
        });
    }

    let Some(rate) = latest_for_pair(&code, rates) else {
        return Err(NoRate {
            currency: code.clone(),
            amount_cents,
            detail: format!(
                "no published {DECLARED_CURRENCY}/{code} rate is recorded, so this amount cannot be stated in {DECLARED_CURRENCY}"
            ),
        });
    };

    let converted = if is_declared(&rate.base) {
        // Published as quote units per one EUR: divide.
        divide(amount_cents, &rate.rate)
    } else {
        // Published as EUR per one unit of the foreign currency: multiply.
        multiply(amount_cents, &rate.rate)
    };

    match converted {
        Some(cents) => Ok(EurAmount::Converted {
            cents,
            from_currency: code,
            from_cents: amount_cents,
            rate: RateUsed {
                base: rate.base.clone(),
                quote: rate.quote.clone(),
                observed_on: rate.observed_on.clone(),
                source: rate.source.clone(),
            },
        }),
        None => Err(NoRate {
            currency: code,
            amount_cents,
            detail: format!(
                "the recorded {}/{} rate observed on {} is not a usable number, so this amount cannot be stated in {DECLARED_CURRENCY}",
                rate.base, rate.quote, rate.observed_on
            ),
        }),
    }
}

/// The most recently observed rate covering EUR and `code`, in either orientation.
///
/// Only the EUR-base orientation is produced today ([`crate::price::FX_BASE`] is EUR
/// and `parse_ecb_csv` writes `base: "EUR"`), but reading both means a rate imported
/// from anywhere else is used rather than quietly ignored.
fn latest_for_pair<'a>(code: &str, rates: &'a [FxObservation]) -> Option<&'a FxObservation> {
    rates
        .iter()
        .filter(|rate| {
            (is_declared(&rate.base) && rate.quote.eq_ignore_ascii_case(code))
                || (is_declared(&rate.quote) && rate.base.eq_ignore_ascii_case(code))
        })
        .max_by(|a, b| {
            a.observed_on
                .cmp(&b.observed_on)
                .then_with(|| a.fetched_at.cmp(&b.fetched_at))
        })
}

/// `amount / rate`, rounded half away from zero. `None` when the rate is not positive.
fn divide(amount_cents: i64, rate: &Quantity) -> Option<i64> {
    if rate.mantissa <= 0 {
        return None;
    }
    let scale = 10i128.checked_pow(rate.scale)?;
    let numerator = (amount_cents as i128).checked_mul(scale)?;
    round_div(numerator, rate.mantissa as i128)?.try_into().ok()
}

/// `amount * rate`, rounded half away from zero. `None` when the rate is not positive.
fn multiply(amount_cents: i64, rate: &Quantity) -> Option<i64> {
    if rate.mantissa <= 0 {
        return None;
    }
    let scale = 10i128.checked_pow(rate.scale)?;
    let numerator = (amount_cents as i128).checked_mul(rate.mantissa as i128)?;
    round_div(numerator, scale)?.try_into().ok()
}

/// Integer division rounding half away from zero, matching
/// [`crate::subscription::BillingCycle::monthly_cents`] so two paths to one cent
/// cannot disagree.
fn round_div(numerator: i128, denominator: i128) -> Option<i128> {
    if denominator == 0 {
        return None;
    }
    let sign = if (numerator < 0) != (denominator < 0) {
        -1
    } else {
        1
    };
    let n = numerator.checked_abs()?;
    let d = denominator.checked_abs()?;
    Some(sign * ((n.checked_mul(2)?.checked_add(d)?) / (d * 2)))
}

/// One billing subscription whose price is in a currency that cannot be stated in EUR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unconverted {
    pub subscription_id: String,
    pub subscription_name: String,
    pub monthly_cents: i64,
    pub currency: String,
    pub detail: String,
}

/// Recurring personal spend as one EUR figure, plus everything left out of it.
///
/// `monthly_cents` is a total a person can act on precisely because it never contains
/// an unconverted foreign amount. What could not be converted is in `not_convertible`,
/// itemised, so the total is never quietly short by an unknown amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EurBurn {
    pub currency: &'static str,
    pub monthly_cents: i64,
    pub annual_cents: i64,
    pub billing_count: usize,
    pub covered_count: usize,
    pub unknown_price_count: usize,
    /// How many of the summed subscriptions needed a rate.
    pub converted_count: usize,
    pub not_convertible: Vec<Unconverted>,
}

impl EurBurn {
    /// Whether the total accounts for every billing subscription that has a price.
    pub fn is_complete(&self) -> bool {
        self.not_convertible.is_empty() && self.unknown_price_count == 0
    }
}

/// The Q103 total: monthly burn in EUR on `date`, with refusals listed rather than
/// folded in.
pub fn burn_in_eur(subscriptions: &[Subscription], date: &str, rates: &[FxObservation]) -> EurBurn {
    let mut monthly = 0i64;
    let mut billing_count = 0usize;
    let mut covered_count = 0usize;
    let mut unknown_price_count = 0usize;
    let mut converted_count = 0usize;
    let mut not_convertible = Vec::new();

    for subscription in subscriptions {
        match subscription.state_at(date) {
            State::Active | State::Trial => {
                billing_count += 1;
                let Some(price) = subscription.price_at(date) else {
                    unknown_price_count += 1;
                    continue;
                };
                let monthly_equivalent = price.cycle.monthly_cents(price.amount_cents);
                match to_eur(monthly_equivalent, &price.currency, rates) {
                    Ok(EurAmount::Declared { cents }) => monthly = monthly.saturating_add(cents),
                    Ok(EurAmount::Converted { cents, .. }) => {
                        converted_count += 1;
                        monthly = monthly.saturating_add(cents);
                    }
                    Err(refusal) => not_convertible.push(Unconverted {
                        subscription_id: subscription.id.clone(),
                        subscription_name: subscription.name.clone(),
                        monthly_cents: monthly_equivalent,
                        currency: refusal.currency.clone(),
                        detail: refusal.detail,
                    }),
                }
            }
            State::Covered => covered_count += 1,
            State::Considering | State::Paused | State::Cancelled => {}
        }
    }

    EurBurn {
        currency: DECLARED_CURRENCY,
        monthly_cents: monthly,
        annual_cents: monthly.saturating_mul(12),
        billing_count,
        covered_count,
        unknown_price_count,
        converted_count,
        not_convertible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::{BillingCycle, PricePoint, StateChange};

    fn rate(quote: &str, mantissa: i64, scale: u32, observed_on: &str) -> FxObservation {
        FxObservation {
            base: "EUR".into(),
            quote: quote.into(),
            observed_on: observed_on.into(),
            rate: Quantity { mantissa, scale },
            source: "ecb".into(),
            fetched_at: format!("{observed_on}T12:00:00Z"),
        }
    }

    fn sub(id: &str, cents: i64, currency: &str, state: State) -> Subscription {
        Subscription {
            id: id.into(),
            name: id.into(),
            source_path: format!("Subscriptions/{id}.md"),
            category: None,
            value_rating: None,
            prices: vec![PricePoint {
                valid_from: "2026-01-01".into(),
                amount_cents: cents,
                currency: currency.into(),
                cycle: BillingCycle::Monthly,
                plan: None,
                reason: "fixture".into(),
            }],
            states: vec![StateChange {
                effective: "2026-01-01".into(),
                state,
                note: "fixture".into(),
            }],
        }
    }

    #[test]
    fn eur_needs_no_rate_and_consults_none() {
        assert_eq!(
            to_eur(2000, "EUR", &[]),
            Ok(EurAmount::Declared { cents: 2000 })
        );
        assert_eq!(
            to_eur(2000, " eur ", &[]),
            Ok(EurAmount::Declared { cents: 2000 })
        );
    }

    #[test]
    fn a_foreign_amount_with_no_rate_is_refused_rather_than_relabelled() {
        // A USD price and an empty finance_fx_rates.
        let refusal = to_eur(4_500, "USD", &[]).unwrap_err();
        assert_eq!(refusal.currency, "USD");
        assert_eq!(refusal.amount_cents, 4_500);
        assert!(
            refusal.detail.contains("no published EUR/USD rate"),
            "the refusal must name the pair: {}",
            refusal.detail
        );
    }

    #[test]
    fn a_published_rate_converts_and_says_where_it_came_from() {
        // ECB publishes quote-per-EUR, so 1 EUR = 1.0850 USD and 20.00 USD is 18.43 EUR.
        let rates = vec![rate("USD", 10_850, 4, "2026-09-08")];
        let converted = to_eur(2000, "USD", &rates).unwrap();
        assert_eq!(converted.cents(), 1843);
        let EurAmount::Converted {
            from_currency,
            from_cents,
            rate: used,
            ..
        } = converted
        else {
            panic!("a foreign amount with a rate must report as converted");
        };
        assert_eq!(from_currency, "USD");
        assert_eq!(from_cents, 2000);
        assert_eq!(used.source, "ecb");
        assert_eq!(used.observed_on, "2026-09-08");
    }

    #[test]
    fn the_newest_observation_of_a_pair_wins() {
        let rates = vec![
            rate("USD", 12_000, 4, "2026-01-02"),
            rate("USD", 10_000, 4, "2026-09-08"),
            rate("CHF", 9_400, 4, "2026-09-08"),
        ];
        assert_eq!(to_eur(1000, "USD", &rates).unwrap().cents(), 1000);
    }

    #[test]
    fn an_inverted_publication_is_read_rather_than_ignored() {
        // base = USD, quote = EUR: EUR per one USD. 20.00 USD at 0.92 is 18.40 EUR.
        let inverted = FxObservation {
            base: "USD".into(),
            quote: "EUR".into(),
            observed_on: "2026-09-08".into(),
            rate: Quantity {
                mantissa: 9_200,
                scale: 4,
            },
            source: "manual".into(),
            fetched_at: "2026-09-08T12:00:00Z".into(),
        };
        assert_eq!(to_eur(2000, "USD", &[inverted]).unwrap().cents(), 1840);
    }

    #[test]
    fn a_zero_or_negative_rate_is_a_gap_not_a_price() {
        assert!(to_eur(2000, "USD", &[rate("USD", 0, 4, "2026-09-08")]).is_err());
        assert!(to_eur(2000, "USD", &[rate("USD", -10_000, 4, "2026-09-08")]).is_err());
    }

    #[test]
    fn conversion_rounds_half_away_from_zero() {
        // 1 EUR = 3 USD exactly: 10.00 USD is 3.333... EUR, which rounds to 3.33.
        let rates = vec![rate("USD", 30_000, 4, "2026-09-08")];
        assert_eq!(to_eur(1000, "USD", &rates).unwrap().cents(), 333);
        // 1 EUR = 2 USD: 5.01 USD is 2.505 EUR, which rounds away from zero to 2.51.
        let rates = vec![rate("USD", 20_000, 4, "2026-09-08")];
        assert_eq!(to_eur(501, "USD", &rates).unwrap().cents(), 251);
    }

    #[test]
    fn the_eur_burn_never_contains_an_unconverted_amount() {
        // Two active USD subscriptions, one covered EUR one, and no rate for USD.
        let subs = vec![
            sub("Studio Lite", 1500, "USD", State::Active),
            sub("Pixel Relay", 800, "USD", State::Active),
            sub("Vault Storage", 1200, "EUR", State::Covered),
        ];
        let burn = burn_in_eur(&subs, "2026-09-09", &[]);
        assert_eq!(burn.monthly_cents, 0, "nothing convertible, nothing summed");
        assert_eq!(burn.billing_count, 2);
        assert_eq!(burn.covered_count, 1);
        assert_eq!(burn.converted_count, 0);
        assert_eq!(burn.not_convertible.len(), 2);
        assert!(!burn.is_complete());
        assert_eq!(burn.not_convertible[0].monthly_cents, 1500);
        assert_eq!(burn.not_convertible[0].currency, "USD");
    }

    #[test]
    fn the_same_burn_with_a_rate_present_is_summed_and_counted_as_converted() {
        let subs = vec![
            sub("Studio Lite", 1500, "USD", State::Active),
            sub("Rent", 50_000, "EUR", State::Active),
        ];
        let rates = vec![rate("USD", 10_000, 4, "2026-09-08")];
        let burn = burn_in_eur(&subs, "2026-09-09", &rates);
        assert_eq!(burn.monthly_cents, 51_500);
        assert_eq!(burn.annual_cents, 618_000);
        assert_eq!(burn.converted_count, 1);
        assert!(burn.is_complete());
    }

    #[test]
    fn a_billing_subscription_with_no_price_is_counted_not_assumed_zero() {
        let mut priceless = sub("Meridian Card", 0, "EUR", State::Active);
        priceless.prices.clear();
        let burn = burn_in_eur(&[priceless], "2026-09-09", &[]);
        assert_eq!(burn.unknown_price_count, 1);
        assert_eq!(burn.monthly_cents, 0);
        assert!(!burn.is_complete());
    }
}

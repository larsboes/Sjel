//! Wish comparison: the rules the comparison view draws, kept here so every client shares them.
//!
//! The data is `GET /inventory/api/wish-comparison`: per wish group, its needs and its
//! candidates with numeric specs and dated prices. This module decides what to show. Which spec
//! is the chart's x axis, a candidate's latest price, the value score in cents per unit of each
//! spec, whether a candidate meets the group's needs, and the domains the chart scales to.
//!
//! A score is a ratio per spec, never one combined number. "Cents per GB" and "cents per GB/s"
//! stay separate, because a weighted sum would decide the trade-off between them in code.

use serde::Deserialize;

use crate::shell::{Shell, ShellResult};

pub use inventory_api::{Candidate, Fit, GroupComparison, PriceObservation, Score, Spec};

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    groups: Vec<GroupComparison>,
}

impl Shell {
    pub async fn inventory_wish_comparison(&self) -> ShellResult<Vec<GroupComparison>> {
        let envelope: Envelope = self
            .json(
                reqwest::Method::GET,
                "/inventory/api/wish-comparison",
                None,
                10,
            )
            .await?;
        Ok(envelope.groups)
    }

    /// Mark a wish group as alternatives or not. The title and constraint go along because the
    /// route saves the whole group.
    pub async fn inventory_set_alternatives(
        &self,
        id: &str,
        title: &str,
        constraint_text: &str,
        alternatives: bool,
    ) -> ShellResult<()> {
        self.send(
            reqwest::Method::POST,
            "/inventory/api/wish-groups",
            Some(serde_json::json!({
                "id": id,
                "title": title,
                "constraint_text": constraint_text,
                "alternatives": alternatives,
            })),
            10,
        )
        .await
        .map(|_| ())
    }
}



/// A closed range for one chart axis, padded so no point sits on the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    pub min: f64,
    pub max: f64,
}

impl Domain {
    /// The smallest padded range over `values`. One value, or equal ones, get a range around it.
    pub fn over(values: impl IntoIterator<Item = f64>) -> Option<Domain> {
        let mut values = values.into_iter().filter(|v| v.is_finite()).peekable();
        values.peek()?;
        let (min, max) = values.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let pad = if max > min {
            (max - min) * 0.08
        } else {
            min.abs().max(1.0) * 0.1
        };
        Some(Domain {
            min: min - pad,
            max: max + pad,
        })
    }

    /// 0 at `min`, 1 at `max`.
    pub fn fraction(&self, value: f64) -> f64 {
        (value - self.min) / (self.max - self.min)
    }
}

/// The scatter's two domains: the axis spec, and price in cents including each limit and need,
/// so the lines the chart draws are always inside it.
pub fn scatter_domains(group: &GroupComparison, axis: &str) -> Option<(Domain, Domain)> {
    let points: Vec<(f64, f64)> = group
        .candidates
        .iter()
        .filter_map(|c| Some((c.spec(axis)?, c.latest()?.price_cents as f64)))
        .collect();
    if points.is_empty() {
        return None;
    }
    let x = Domain::over(points.iter().map(|p| p.0).chain(group.need(axis)))?;
    let y = Domain::over(
        points.iter().map(|p| p.1).chain(
            group
                .candidates
                .iter()
                .filter_map(|c| c.cost_max_cents.map(|v| v as f64)),
        ),
    )?;
    Some((x, y))
}

/// Days since 1970-01-01 for a `YYYY-MM-DD...` stamp, for the history's time axis.
pub fn day_number(stamp: &str) -> Option<i64> {
    let y: i64 = stamp.get(0..4)?.parse().ok()?;
    let m: i64 = stamp.get(5..7)?.parse().ok()?;
    let d: i64 = stamp.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The history's domains: days across every observation, and every price and limit.
pub fn history_domains(group: &GroupComparison) -> Option<(Domain, Domain)> {
    let days: Vec<f64> = group
        .candidates
        .iter()
        .flat_map(|c| &c.observations)
        .filter_map(|o| day_number(&o.observed_at).map(|d| d as f64))
        .collect();
    let x = Domain::over(days)?;
    let y = Domain::over(
        group
            .candidates
            .iter()
            .flat_map(|c| c.observations.iter().map(|o| o.price_cents as f64))
            .chain(
                group
                    .candidates
                    .iter()
                    .filter_map(|c| c.cost_max_cents.map(|v| v as f64)),
            ),
    )?;
    Some((x, y))
}

/// `1 401,02 €` style is the page's; here cents per unit read as euros with up to two decimals.
pub fn format_per_unit(score: &Score) -> String {
    let unit = if score.unit.is_empty() {
        score.key.as_str()
    } else {
        score.unit.as_str()
    };
    format!("{:.2} €/{unit}", score.cents_per_unit / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(key: &str, value: f64, unit: &str) -> Spec {
        Spec {
            key: key.into(),
            value,
            unit: unit.into(),
        }
    }

    fn seen(cents: i64, at: &str, source: &str) -> PriceObservation {
        PriceObservation {
            price_cents: cents,
            source: source.into(),
            configuration: String::new(),
            observed_at: at.into(),
            alert: false,
        }
    }

    fn candidate(
        id: &str,
        specs: Vec<Spec>,
        prices: Vec<PriceObservation>,
        limit: Option<i64>,
    ) -> Candidate {
        Candidate {
            id: id.into(),
            label: id.into(),
            link: None,
            cost_max_cents: limit,
            specs,
            observations: prices,
        }
    }

    fn group() -> GroupComparison {
        GroupComparison {
            id: "server".into(),
            title: "Home server".into(),
            constraint_text: String::new(),
            needs: vec![spec("memory_gb", 29.0, "GB")],
            candidates: vec![
                candidate(
                    "m6",
                    vec![
                        spec("memory_gb", 32.0, "GB"),
                        spec("bandwidth_gb_s", 170.0, "GB/s"),
                    ],
                    vec![
                        seen(158_900, "2026-11-03", "gh"),
                        seen(140_102, "2026-11-04", "gh"),
                        seen(139_000, "2026-11-04T09:00Z", "manual"),
                    ],
                    Some(150_000),
                ),
                candidate(
                    "m5pro",
                    vec![
                        spec("memory_gb", 64.0, "GB"),
                        spec("bandwidth_gb_s", 307.0, "GB/s"),
                    ],
                    vec![seen(291_387, "2026-11-03", "gh")],
                    None,
                ),
                candidate("m4", vec![spec("memory_gb", 24.0, "GB")], vec![], None),
            ],
        }
    }

    #[test]
    fn the_latest_price_is_the_cheapest_of_the_newest_day() {
        let g = group();
        assert_eq!(g.candidates[0].latest().unwrap().price_cents, 139_000);
        assert_eq!(g.candidates[0].within_limit(), Some(true));
        assert_eq!(g.candidates[1].within_limit(), None);
        assert!(g.candidates[2].latest().is_none());
    }

    #[test]
    fn a_score_is_cents_per_unit_of_each_spec() {
        let g = group();
        let scores = g.candidates[1].scores();
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].key, "memory_gb");
        assert!((scores[0].cents_per_unit - 291_387.0 / 64.0).abs() < 1e-9);
        assert_eq!(format_per_unit(&scores[0]), "45.53 €/GB");
        assert!(g.candidates[2].scores().is_empty(), "no price, no score");
    }

    #[test]
    fn needs_are_met_short_or_unknown() {
        let g = group();
        assert_eq!(g.candidates[0].fits(&g.needs), vec![Fit::Meets]);
        assert_eq!(
            g.candidates[2].fits(&g.needs),
            vec![Fit::Short {
                key: "memory_gb".into(),
                have: 24.0,
                need: 29.0
            }]
        );
        let unknown = candidate("x", vec![], vec![], None);
        assert_eq!(
            unknown.fits(&g.needs),
            vec![Fit::Unknown {
                key: "memory_gb".into()
            }]
        );
    }

    #[test]
    fn the_axis_is_a_shared_spec_and_a_need_comes_first() {
        let g = group();
        assert_eq!(g.axis_keys(), vec!["bandwidth_gb_s", "memory_gb"]);
        assert_eq!(g.default_axis().as_deref(), Some("memory_gb"));
        assert_eq!(g.unit_of("bandwidth_gb_s"), "GB/s");
        assert!(g.is_comparable());
    }

    #[test]
    fn the_scatter_domain_holds_points_needs_and_limits() {
        let g = group();
        let (x, y) = scatter_domains(&g, "memory_gb").unwrap();
        assert!(x.min < 29.0 && x.max > 64.0);
        assert!(y.min < 139_000.0 && y.max > 291_387.0);
        assert!(x.fraction(x.min).abs() < 1e-12 && (x.fraction(x.max) - 1.0).abs() < 1e-12);
        let one = Domain::over([5.0]).unwrap();
        assert!(one.min < 5.0 && one.max > 5.0);
        assert!(Domain::over([]).is_none());
    }

    #[test]
    fn days_count_from_the_epoch() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2026-11-03T08:00:00Z"), Some(20_760));
        assert_eq!(
            day_number("2026-11-04").unwrap() - day_number("2026-10-31").unwrap(),
            4
        );
        assert_eq!(day_number("2026-13-01"), None);
        let (x, _) = history_domains(&group()).unwrap();
        assert!(x.min < 20_760.0 && x.max > 20_761.0);
    }
}

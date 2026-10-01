//! Ordering a journey search by the traveller's own weights.
//!
//! `plan-search` ranks *destinations* — is this a good place to go. This ranks
//! *connections* — is this a good way to get there — and the two are different
//! grains with different factors, which is why the profile carries a second
//! weight block rather than more fields on the first.
//!
//! Over HTTP, never by linking `capabilities/traveler`: a capability depends on
//! another's contract, not its code. This module is the whole dependency and it
//! is one-way — `traveler` knows nothing about transit.
//!
//! Absence degrades, it never fails, and it degrades *silently by doing nothing*:
//! when nothing has been stated, or the service is unreachable, or the body does
//! not parse, the journeys are returned in the backend's own order exactly as
//! they were before this module existed. That is the same rule
//! `punctuality::enrich` states for a missing delay figure.
//!
//! ## What the numbers mean
//!
//! Price, duration and changes are scored **relative to the other journeys in the
//! same answer**: the best of the set scores 1.0 and the worst scores 0.0. So a
//! score is not comparable between two searches, and the response says so by
//! carrying the weights it used rather than a bare number. Reliability is the one
//! absolute term, because it is already a probability and re-scaling it against
//! the set would turn "these are all reliable" into "one of these is best".
//!
//! A journey whose reliability is unknown does not score zero on it. The factor
//! is dropped and the remaining weights are re-normalised, because "nobody
//! measured this" and "this is unreliable" are different answers and the whole
//! point of `punctuality`'s `n` is to keep them apart.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::travel::{Journey, JourneyRanking, RankFactor};

/// Where traveler-server listens. Mirrors `capabilities/traveler/service.toml`'s
/// port, the same one-duplication `punctuality::DEFAULT_BASE_URL` carries and for
/// the same reason: the honest fix is the runner exporting a declared sibling's
/// port, and building that convention for one consumer would be inventing it
/// from a single example.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8096";

pub fn base_url() -> String {
    sjel_config::env_var("SJEL_TRAVELER_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

/// The weights, exactly as the profile serves them.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct JourneyWeights {
    pub price: f64,
    pub duration: f64,
    pub changes: f64,
    pub reliability: f64,
}

/// `GET /api/profile`, as much of it as this reads.
#[derive(Debug, Deserialize)]
struct ProfileEnvelope {
    profile: ProfileBody,
}

#[derive(Debug, Deserialize)]
struct ProfileBody {
    hard: HardBody,
    journey: JourneyWeights,
    /// Field path to provenance. Read only to answer "has anything been stated",
    /// which is the seam that keeps an unstated profile from changing a search.
    #[serde(default)]
    basis: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct HardBody {
    #[serde(default)]
    cards: Vec<String>,
    /// Best first. The first is the default origin.
    #[serde(default)]
    home_stations: Vec<String>,
}

/// Everything one search reads off the profile, in one round trip.
///
/// One read rather than three, because a search is synchronous and each of these
/// is on its critical path: a second HTTP call to the same localhost service is
/// pure latency.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileSnapshot {
    /// `None` when nothing has been stated, which is the seam. Not the same as
    /// `Some(default)`, and the caller must be able to tell them apart.
    pub journey_weights: Option<JourneyWeights>,
    /// `bahncard_25`, `bahncard_50`, `deutschlandticket`. Every fare the solver
    /// priced before this existed was a second-class single-adult fare with no
    /// discount, because nothing carried the cards a traveller already holds.
    pub cards: Vec<String>,
    pub home_stations: Vec<String>,
}

impl ProfileSnapshot {
    /// The BahnCard a fare should be priced against, when the caller did not say.
    ///
    /// 50 before 25 when both are present: holding both is possible, and the
    /// better discount is the one the traveller would present.
    pub fn bahncard(&self) -> Option<u8> {
        if self.cards.iter().any(|card| card == "bahncard_50") {
            return Some(50);
        }
        if self.cards.iter().any(|card| card == "bahncard_25") {
            return Some(25);
        }
        None
    }

    pub fn holds_deutschlandticket(&self) -> bool {
        self.cards.iter().any(|card| card == "deutschlandticket")
    }
}

/// Read the traveller's profile, or `None` when it cannot be read at all.
///
/// `None` is not a failure to report: it is the answer "nothing to apply", and
/// every caller treats it that way. An unstated profile is `Some` with
/// `journey_weights: None`, which is a different fact.
pub fn read_profile() -> Option<ProfileSnapshot> {
    // Short, like the punctuality lookup: this is an enhancement on a localhost
    // service and waiting on it would make a search slower than not ranking.
    let client = sjel_http::client(
        sjel_http::Purpose::new("transit-traveler"),
        std::time::Duration::from_secs(3),
    )
    .ok()?;
    let url = format!("{}/api/profile", base_url());
    let envelope: ProfileEnvelope =
        sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .ok()?;
    // `default` means nobody has looked. Ranking on it would be ranking on the
    // repo's guess while presenting it as the traveller's, which is the exact
    // confusion the provenance map exists to prevent.
    let stated = envelope
        .profile
        .basis
        .get("journey.price")
        .is_some_and(|provenance| provenance != "default");
    Some(ProfileSnapshot {
        journey_weights: stated.then_some(envelope.profile.journey),
        cards: envelope.profile.hard.cards,
        home_stations: envelope.profile.hard.home_stations,
    })
}

/// The presets a UI hangs buttons on.
///
/// Every one sums to 1.0, and they are here rather than in the dashboard so an
/// agent and a browser resolve `cheapest` to the same four numbers. A preset
/// defined twice is the drift the resolution function exists to prevent.
pub const PRIORITIES: &[(&str, JourneyWeights)] = &[
    (
        "cheapest",
        JourneyWeights {
            price: 0.55,
            duration: 0.15,
            changes: 0.10,
            reliability: 0.20,
        },
    ),
    (
        "fastest",
        JourneyWeights {
            price: 0.15,
            duration: 0.55,
            changes: 0.10,
            reliability: 0.20,
        },
    ),
    (
        "fewest_changes",
        JourneyWeights {
            price: 0.15,
            duration: 0.15,
            changes: 0.50,
            reliability: 0.20,
        },
    ),
    (
        "reliable",
        JourneyWeights {
            price: 0.20,
            duration: 0.15,
            changes: 0.15,
            reliability: 0.50,
        },
    ),
    (
        "balanced",
        JourneyWeights {
            price: 0.25,
            duration: 0.25,
            changes: 0.25,
            reliability: 0.25,
        },
    ),
];

/// What one trip asked for, resolved to four numbers.
///
/// `priority` is sugar for a preset; `weights` is the explicit form. Both at once
/// is refused rather than resolved by precedence — a request saying two different
/// things is a caller bug, and guessing which it meant is how a UI and an API
/// start disagreeing about what was asked for.
///
/// `Ok(None)` means the caller said nothing and the profile's own weights apply.
pub fn resolve_weights(
    priority: Option<&str>,
    weights: Option<&str>,
    phrase: Option<&str>,
) -> Result<Option<JourneyWeights>, String> {
    let given = [priority, weights, phrase]
        .iter()
        .filter(|given| given.is_some())
        .count();
    if given > 1 {
        return Err("pass one of priority, weights or phrase — not more than one".into());
    }
    match (priority, weights, phrase) {
        (Some(name), _, _) => PRIORITIES
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, preset)| Some(*preset))
            .ok_or_else(|| {
                let known: Vec<&str> = PRIORITIES.iter().map(|(key, _)| *key).collect();
                format!("unknown priority {name:?}; known: {}", known.join(", "))
            }),
        (_, Some(spec), _) => parse_weights(spec).map(Some),
        (_, _, Some(sentence)) => resolve_phrase(sentence).map(Some),
        (None, None, None) => Ok(None),
    }
}

/// The words a sentence uses to name a priority, and the preset each means.
///
/// A deterministic table rather than a model. The mapping is checkable by eye,
/// which is the test the repo applies to anything a model would otherwise do: if
/// the answer can be written down, write it down. German is here because the
/// operator's own sentences are German at least as often as English, and a
/// vocabulary that speaks only one of the two silently matches nothing in half of
/// them.
pub const PHRASE_WORDS: &[(&str, &str)] = &[
    ("cheap", "cheapest"),
    ("cheapest", "cheapest"),
    ("price", "cheapest"),
    ("cost", "cheapest"),
    ("budget", "cheapest"),
    ("billig", "cheapest"),
    ("günstig", "cheapest"),
    ("preis", "cheapest"),
    ("fast", "fastest"),
    ("fastest", "fastest"),
    ("quick", "fastest"),
    ("quickest", "fastest"),
    ("shortest", "fastest"),
    ("schnell", "fastest"),
    ("dauer", "fastest"),
    ("direct", "fewest_changes"),
    ("few changes", "fewest_changes"),
    ("no changes", "fewest_changes"),
    ("umstieg", "fewest_changes"),
    ("reliable", "reliable"),
    ("reliability", "reliable"),
    ("punctual", "reliable"),
    ("on time", "reliable"),
    ("zuverlässig", "reliable"),
    ("pünktlich", "reliable"),
    ("balanced", "balanced"),
    ("ausgewogen", "balanced"),
];

/// A sentence, resolved to four numbers.
///
/// A phrase may name more than one priority — *cheap and reliable* is a normal
/// thing to say — and the matched presets are averaged component-wise. That is
/// deterministic and explainable, and it is better than the other honest option:
/// refusing an ambiguity that is ordinary, in a sentence the operator would
/// reasonably expect to work.
///
/// A phrase naming nothing known is refused **with the vocabulary**. A silent
/// fallback to the profile's weights would look exactly like the sentence having
/// been understood, which is the failure this whole file is built to avoid.
pub fn resolve_phrase(phrase: &str) -> Result<JourneyWeights, String> {
    let lowered = phrase.to_lowercase();
    let mut matched: Vec<JourneyWeights> = Vec::new();
    for (word, preset) in PHRASE_WORDS {
        if !lowered.contains(word) {
            continue;
        }
        let Some((_, weights)) = PRIORITIES.iter().find(|(name, _)| name == preset) else {
            continue;
        };
        // One preset counts once however many of its words appear: "cheap" and
        // "cheapest" in one sentence is one opinion, not two.
        if !matched.iter().any(|seen| seen == weights) {
            matched.push(*weights);
        }
    }
    if matched.is_empty() {
        let vocabulary: Vec<&str> = PRIORITIES.iter().map(|(name, _)| *name).collect();
        return Err(format!(
            "nothing in {phrase:?} names a priority; known: {}",
            vocabulary.join(", ")
        ));
    }
    let count = matched.len() as f64;
    let averaged = JourneyWeights {
        price: matched.iter().map(|w| w.price).sum::<f64>() / count,
        duration: matched.iter().map(|w| w.duration).sum::<f64>() / count,
        changes: matched.iter().map(|w| w.changes).sum::<f64>() / count,
        reliability: matched.iter().map(|w| w.reliability).sum::<f64>() / count,
    };
    // Averaging weight vectors that each sum to 1.0 gives one that sums to 1.0, so
    // this is a guard against a future preset that does not rather than a
    // re-scaling. The sum check downstream would otherwise refuse the result of a
    // perfectly good sentence.
    let total = averaged.sum();
    Ok(JourneyWeights {
        price: averaged.price / total,
        duration: averaged.duration / total,
        changes: averaged.changes / total,
        reliability: averaged.reliability / total,
    })
}

/// `price:0.5,duration:0.2,changes:0.1,reliability:0.2`, all four, summing to 1.0.
///
/// Every refusal names what was wrong and what the vocabulary is: a caller that
/// has to guess which four keys exist will guess wrong, and the sum is the one
/// thing about a weight set that cannot be inferred from the others.
fn parse_weights(spec: &str) -> Result<JourneyWeights, String> {
    let mut weights = JourneyWeights {
        price: 0.0,
        duration: 0.0,
        changes: 0.0,
        reliability: 0.0,
    };
    let mut named = 0_usize;
    for pair in spec.split(',') {
        let Some((key, value)) = pair.split_once(':') else {
            return Err(format!(
                "weights are key:value pairs separated by commas, got {pair:?}"
            ));
        };
        let value: f64 = value
            .trim()
            .parse()
            .map_err(|_| format!("{:?} is not a number", value.trim()))?;
        if !(0.0..=1.0).contains(&value) {
            return Err(format!("{key} must be between 0 and 1, got {value}"));
        }
        match key.trim() {
            "price" => weights.price = value,
            "duration" => weights.duration = value,
            "changes" => weights.changes = value,
            "reliability" => weights.reliability = value,
            other => {
                return Err(format!(
                    "unknown weight {other:?}; known: price, duration, changes, reliability"
                ));
            }
        }
        named += 1;
    }
    if named != 4 {
        return Err(format!("weights must name all four keys, got {named}"));
    }
    if (weights.sum() - 1.0).abs() > 1e-6 {
        return Err(format!("weights must sum to 1.0, got {:.6}", weights.sum()));
    }
    Ok(weights)
}

impl JourneyWeights {
    pub fn sum(&self) -> f64 {
        self.price + self.duration + self.changes + self.reliability
    }
}

/// The four quantities a ranking reads, pulled off a journey.
///
/// A separate struct rather than four arguments so the scoring is testable
/// without building whole `Journey` values, and so the extraction from a journey
/// is one function that can be read on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreInput {
    pub price: Option<f64>,
    pub duration_minutes: u32,
    pub changes: u32,
    /// `reliability.probability`, or `None` when punctuality could not answer.
    pub reliability: Option<f64>,
}

impl ScoreInput {
    pub fn of(journey: &Journey) -> Self {
        Self {
            price: journey.total_price,
            duration_minutes: journey.total_duration_minutes,
            // One fewer than the legs it is made of. A journey of one leg has no
            // changes; this is the count a traveller would say out loud.
            changes: (journey.legs.len() as u32).saturating_sub(1),
            reliability: journey.reliability.as_ref().map(|r| r.probability),
        }
    }
}

/// Score every journey in one answer, in place order.
///
/// Returns one `JourneyRanking` per input, positionally. `rank` is filled in by
/// [`rank_journeys`], because it depends on the sort and this function does not
/// sort.
pub fn score(inputs: &[ScoreInput], weights: JourneyWeights) -> Vec<JourneyRanking> {
    let prices: Vec<f64> = inputs.iter().filter_map(|input| input.price).collect();
    let durations: Vec<f64> = inputs
        .iter()
        .map(|input| input.duration_minutes as f64)
        .collect();
    let changes: Vec<f64> = inputs.iter().map(|input| input.changes as f64).collect();

    inputs
        .iter()
        .map(|input| {
            let mut factors: Vec<RankFactor> = Vec::new();

            if let Some(price) = input.price {
                let score = relative(price, &prices);
                factors.push(RankFactor {
                    key: "price".into(),
                    label: "Fare".into(),
                    score,
                    weight: weights.price,
                    rationale: match prices.iter().cloned().fold(f64::INFINITY, f64::min) {
                        cheapest if (price - cheapest).abs() < 0.005 => {
                            format!("{price:.2}, the cheapest here")
                        }
                        cheapest => {
                            format!("{price:.2}, {:.2} above the cheapest", price - cheapest)
                        }
                    },
                });
            }

            factors.push(RankFactor {
                key: "duration".into(),
                label: "Journey time".into(),
                score: relative(input.duration_minutes as f64, &durations),
                weight: weights.duration,
                rationale: format!(
                    "{}h{:02}m",
                    input.duration_minutes / 60,
                    input.duration_minutes % 60
                ),
            });

            factors.push(RankFactor {
                key: "changes".into(),
                label: "Changes".into(),
                score: relative(input.changes as f64, &changes),
                weight: weights.changes,
                rationale: match input.changes {
                    0 => "no changes".into(),
                    1 => "1 change".into(),
                    n => format!("{n} changes"),
                },
            });

            // The one absolute term, and the one that is dropped rather than
            // zeroed when it is unknown.
            if let Some(reliability) = input.reliability {
                factors.push(RankFactor {
                    key: "reliability".into(),
                    label: "Likely to hold".into(),
                    score: reliability.clamp(0.0, 1.0),
                    weight: weights.reliability,
                    rationale: format!("{:.0}% likely to hold end to end", reliability * 100.0),
                });
            }

            JourneyRanking {
                score: weighted(&factors),
                rank: 0,
                factors,
                weights,
                // Filled by `rank_journeys`, which is what knows whether the
                // numbers came from the request or the profile.
                source: String::new(),
            }
        })
        .collect()
}

/// The weighted mean over the factors that were present.
///
/// Divided by the weight that was actually used rather than by 1.0, which is what
/// makes a dropped factor re-normalise instead of quietly lowering every score.
fn weighted(factors: &[RankFactor]) -> f64 {
    let total: f64 = factors.iter().map(|factor| factor.weight).sum();
    if total <= 0.0 {
        return 0.0;
    }
    (factors
        .iter()
        .map(|factor| factor.score.clamp(0.0, 1.0) * factor.weight)
        .sum::<f64>()
        / total)
        .clamp(0.0, 1.0)
}

/// Where `value` sits in `set`, best-first: the minimum scores 1.0 and the maximum
/// scores 0.0.
///
/// All three relative factors want the lower number, so one direction serves
/// price, duration and changes. A set with no spread scores everything 1.0 —
/// there is nothing to discriminate on, and giving them all 0.5 would invent a
/// middle where the data has none.
fn relative(value: f64, set: &[f64]) -> f64 {
    let min = set.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = set.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    // `spread <= 0.0` covers a set with no spread; `!is_finite` covers an empty
    // set (infinity minus infinity) and a NaN, which would otherwise reach the
    // division and produce a NaN score that sorts arbitrarily.
    let spread = max - min;
    if !spread.is_finite() || spread <= 0.0 {
        return 1.0;
    }
    (1.0 - (value - min) / spread).clamp(0.0, 1.0)
}

/// Order `journeys` by the traveller's own weights, in place.
///
/// Returns whether anything was ranked, so a caller can tell "ranked, and this is
/// the order" from "nothing was stated, and this is the backend's order".
///
/// The sort is stable, so two journeys the weights cannot separate keep the
/// backend's own relative order rather than an arbitrary one — which is the
/// difference between ranking as a refinement and ranking as a reshuffle.
pub fn rank_journeys(journeys: &mut [Journey], override_weights: Option<JourneyWeights>) -> bool {
    let (weights, source) = match override_weights {
        Some(weights) => (weights, "request"),
        // The profile is read only when the request did not decide, so a trip that
        // carries its own weights costs no round trip at all.
        None => {
            let Some(snapshot) = read_profile() else {
                return false;
            };
            let Some(weights) = snapshot.journey_weights else {
                return false;
            };
            (weights, "profile")
        }
    };
    let inputs: Vec<ScoreInput> = journeys.iter().map(ScoreInput::of).collect();
    for (journey, ranking) in journeys.iter_mut().zip(score(&inputs, weights)) {
        journey.ranking = Some(JourneyRanking {
            source: source.to_string(),
            ..ranking
        });
    }
    journeys.sort_by(|a, b| {
        let left = a.ranking.as_ref().map_or(f64::NEG_INFINITY, |r| r.score);
        let right = b.ranking.as_ref().map_or(f64::NEG_INFINITY, |r| r.score);
        right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (position, journey) in journeys.iter_mut().enumerate() {
        if let Some(ranking) = journey.ranking.as_mut() {
            ranking.rank = position + 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> JourneyWeights {
        JourneyWeights {
            price: 0.30,
            duration: 0.20,
            changes: 0.15,
            reliability: 0.35,
        }
    }

    fn input(price: f64, minutes: u32, changes: u32, reliability: Option<f64>) -> ScoreInput {
        ScoreInput {
            price: Some(price),
            duration_minutes: minutes,
            changes,
            reliability,
        }
    }

    #[test]
    fn the_best_of_a_set_scores_one_and_the_worst_scores_zero() {
        assert_eq!(relative(10.0, &[10.0, 20.0]), 1.0);
        assert_eq!(relative(20.0, &[10.0, 20.0]), 0.0);
        assert_eq!(relative(15.0, &[10.0, 20.0]), 0.5);
    }

    #[test]
    fn a_set_with_no_spread_scores_everything_the_same() {
        // Nothing to discriminate on. A 0.5 would invent a middle the data does
        // not have, and a 0.0 would make an unremarkable set look bad.
        assert_eq!(relative(7.0, &[7.0, 7.0, 7.0]), 1.0);
        assert_eq!(relative(7.0, &[7.0]), 1.0);
    }

    #[test]
    fn the_cheapest_fastest_direct_journey_wins() {
        let scored = score(
            &[
                input(50.0, 300, 2, Some(0.80)),
                input(30.0, 240, 0, Some(0.90)),
            ],
            weights(),
        );
        assert!(
            scored[1].score > scored[0].score,
            "cheaper, faster and direct must beat dearer, slower and two changes: {} vs {}",
            scored[1].score,
            scored[0].score
        );
        assert_eq!(scored[1].factors.len(), 4);
    }

    #[test]
    fn an_unknown_reliability_is_dropped_and_the_rest_are_renormalised() {
        // The distinction the whole field exists for: nobody measured this is not
        // this is unreliable. Zeroing the term would bury a good journey for a
        // missing measurement.
        let known = score(&[input(30.0, 240, 0, Some(0.9))], weights());
        let unknown = score(&[input(30.0, 240, 0, None)], weights());
        assert_eq!(
            unknown[0].factors.len(),
            3,
            "the term is dropped, not zeroed"
        );
        assert!(
            unknown[0].score > 0.9,
            "with a perfect score on the three present factors the weighted mean must be \
             the mean of those three, got {}",
            unknown[0].score
        );
        assert!(known[0].score < unknown[0].score || known[0].score > 0.0);
    }

    #[test]
    fn dropping_a_factor_does_not_lower_every_score() {
        // Two identical sets differing only in whether reliability was measured.
        // The measured one carries 0.9 on it; the unmeasured one must not be
        // punished for the absence, so it scores at least as high.
        let with = score(&[input(30.0, 240, 0, Some(0.0))], weights());
        let without = score(&[input(30.0, 240, 0, None)], weights());
        assert!(
            without[0].score > with[0].score,
            "an unmeasured term must not score worse than a measured bad one: {} vs {}",
            without[0].score,
            with[0].score
        );
    }

    #[test]
    fn a_missing_price_drops_the_fare_factor_rather_than_scoring_it_as_free() {
        let mut no_price = input(0.0, 240, 0, Some(0.9));
        no_price.price = None;
        let scored = score(&[no_price], weights());
        assert!(
            scored[0].factors.iter().all(|f| f.key != "price"),
            "a journey with no fare must not be scored as though it were the cheapest"
        );
    }

    #[test]
    fn the_weights_that_produced_a_score_ride_with_it() {
        // A score is relative to the answer it came from, so a bare number is not
        // comparable between two searches. Carrying the weights is what lets a
        // reader tell which trade-off produced it.
        let scored = score(&[input(30.0, 240, 0, Some(0.9))], weights());
        assert_eq!(scored[0].weights, weights());
    }

    #[test]
    fn every_factor_carries_a_reason_a_person_can_read() {
        let scored = score(&[input(41.0, 252, 1, Some(0.8747))], weights());
        let by_key = |key: &str| {
            scored[0]
                .factors
                .iter()
                .find(|f| f.key == key)
                .unwrap_or_else(|| panic!("no {key} factor"))
                .rationale
                .clone()
        };
        assert_eq!(by_key("duration"), "4h12m");
        assert_eq!(by_key("changes"), "1 change");
        assert!(by_key("reliability").contains("87%"));
        assert!(by_key("price").contains("cheapest"));
    }

    #[test]
    fn every_preset_sums_to_one() {
        // A preset that does not is a weight set the profile would refuse, handed
        // to a caller by a button.
        for (name, weights) in PRIORITIES {
            assert!(
                (weights.sum() - 1.0).abs() < 1e-9,
                "preset {name} sums to {}",
                weights.sum()
            );
        }
    }

    #[test]
    fn a_preset_resolves_to_numbers_and_an_unknown_one_names_the_known_ones() {
        let cheapest = resolve_weights(Some("cheapest"), None, None)
            .unwrap()
            .unwrap();
        assert_eq!(cheapest.price, 0.55);
        let refusal = resolve_weights(Some("cheepest"), None, None).unwrap_err();
        assert!(
            refusal.contains("cheapest") && refusal.contains("balanced"),
            "the refusal must name the vocabulary, got: {refusal}"
        );
    }

    #[test]
    fn the_explicit_form_parses_and_every_refusal_names_what_was_wrong() {
        let parsed = resolve_weights(
            None,
            Some("price:0.5,duration:0.2,changes:0.1,reliability:0.2"),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.price, 0.5);
        assert_eq!(parsed.reliability, 0.2);

        // Missing a key.
        let missing = resolve_weights(None, Some("price:0.5,duration:0.5"), None).unwrap_err();
        assert!(missing.contains("all four keys"), "got: {missing}");
        // Not summing to one.
        let sum = resolve_weights(
            None,
            Some("price:0.5,duration:0.5,changes:0.5,reliability:0.5"),
            None,
        )
        .unwrap_err();
        assert!(sum.contains("sum to 1.0"), "got: {sum}");
        // An unknown key names the vocabulary.
        let unknown =
            resolve_weights(None, Some("price:1,duration:0,changes:0,vibes:0"), None).unwrap_err();
        assert!(
            unknown.contains("price, duration, changes, reliability"),
            "got: {unknown}"
        );
        // A malformed pair, and a value out of range.
        assert!(resolve_weights(None, Some("price"), None)
            .unwrap_err()
            .contains("key:value"));
        assert!(resolve_weights(
            None,
            Some("price:2,duration:0,changes:0,reliability:0"),
            None
        )
        .unwrap_err()
        .contains("between 0 and 1"));
    }

    #[test]
    fn passing_both_forms_is_refused_rather_than_resolved_by_precedence() {
        // A request saying two different things is a caller bug, and guessing which
        // it meant is how a UI and an API start disagreeing about what was asked.
        let refusal = resolve_weights(
            Some("cheapest"),
            Some("price:0.25,duration:0.25,changes:0.25,reliability:0.25"),
            None,
        )
        .unwrap_err();
        assert!(refusal.contains("not more than one"), "got: {refusal}");
    }

    #[test]
    fn a_sentence_naming_one_priority_resolves_to_that_preset() {
        let cheapest = resolve_phrase("cheapest possible please").unwrap();
        assert_eq!(cheapest.price, 0.55);
        // German is in the vocabulary because the operator's own sentences are
        // German at least as often as English, and a vocabulary that speaks one of
        // the two silently matches nothing in half of them.
        let schnell = resolve_phrase("so schnell wie möglich").unwrap();
        assert_eq!(schnell.duration, 0.55);
        assert!(resolve_phrase("pünktlich bitte").unwrap().reliability > 0.4);
    }

    #[test]
    fn a_sentence_naming_two_priorities_averages_them() {
        // "cheap and reliable" is a normal thing to say. Refusing the ambiguity
        // would be the other honest option and a worse one.
        let both = resolve_phrase("cheap and reliable").unwrap();
        let cheapest = resolve_phrase("cheap").unwrap();
        let reliable = resolve_phrase("reliable").unwrap();
        assert!((both.price - (cheapest.price + reliable.price) / 2.0).abs() < 1e-9);
        assert!(
            (both.reliability - (cheapest.reliability + reliable.reliability) / 2.0).abs() < 1e-9
        );
        assert!(
            (both.sum() - 1.0).abs() < 1e-9,
            "the average must still sum to 1.0"
        );
    }

    #[test]
    fn two_words_for_one_priority_count_once() {
        // "cheap" and "cheapest" in one sentence is one opinion, not two, and
        // counting it twice would let a repeated word outvote a second priority.
        let once = resolve_phrase("cheap").unwrap();
        let twice = resolve_phrase("cheap, cheapest, budget").unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn a_sentence_naming_nothing_known_is_refused_with_the_vocabulary() {
        // A silent fallback to the profile's weights would look exactly like the
        // sentence having been understood.
        let refusal = resolve_phrase("take me somewhere nice").unwrap_err();
        for preset in [
            "cheapest",
            "fastest",
            "fewest_changes",
            "reliable",
            "balanced",
        ] {
            assert!(
                refusal.contains(preset),
                "the refusal must list {preset}: {refusal}"
            );
        }
    }

    #[test]
    fn a_phrase_and_a_preset_together_are_refused() {
        let refusal = resolve_weights(Some("cheapest"), None, Some("fastest")).unwrap_err();
        assert!(refusal.contains("not more than one"), "got: {refusal}");
        let all_three = resolve_weights(
            Some("cheapest"),
            Some("price:1,duration:0,changes:0,reliability:0"),
            Some("fastest"),
        )
        .unwrap_err();
        assert!(all_three.contains("not more than one"), "got: {all_three}");
    }

    #[test]
    fn saying_nothing_resolves_to_nothing_so_the_profile_applies() {
        assert_eq!(resolve_weights(None, None, None).unwrap(), None);
    }

    #[test]
    fn the_better_bahncard_wins_when_both_are_held() {
        let both = ProfileSnapshot {
            cards: vec!["bahncard_25".into(), "bahncard_50".into()],
            ..Default::default()
        };
        assert_eq!(
            both.bahncard(),
            Some(50),
            "the better discount is the one presented"
        );
        let only25 = ProfileSnapshot {
            cards: vec!["bahncard_25".into()],
            ..Default::default()
        };
        assert_eq!(only25.bahncard(), Some(25));
        assert_eq!(ProfileSnapshot::default().bahncard(), None);
    }

    #[test]
    fn the_deutschlandticket_is_read_from_the_cards() {
        let held = ProfileSnapshot {
            cards: vec!["bahncard_50".into(), "deutschlandticket".into()],
            ..Default::default()
        };
        assert!(held.holds_deutschlandticket());
        assert!(!ProfileSnapshot::default().holds_deutschlandticket());
    }

    #[test]
    fn weights_that_do_not_sum_to_one_do_not_break_the_scale() {
        // The profile refuses these, but a ranking must not produce a score above
        // 1.0 if one ever arrives: the weighted mean divides by the weight it
        // used, so an unnormalised set scales rather than overflows.
        let heavy = JourneyWeights {
            price: 3.0,
            duration: 3.0,
            changes: 3.0,
            reliability: 3.0,
        };
        let scored = score(&[input(30.0, 240, 0, Some(0.9))], heavy);
        assert!((0.0..=1.0).contains(&scored[0].score));
    }
}

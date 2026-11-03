//! Where to base yourself between two fixed points.
//!
//! The question the travel model had no home for: not "where should I go" and not
//! "which connection", but *"I have these days between being here and needing to
//! be there — where should I stay?"* `plan-search` answers the first and
//! `transit` the second; neither composes a base out of a way in, a way on, and
//! what is waiting at the other end.
//!
//! ## What this prices, and what it deliberately does not
//!
//! It prices the two legs that bound a stay — reaching the base and leaving it for
//! the next fixed point — and reports who is already there. Those are measurable
//! today, so they are here.
//!
//! **It does not price the stay.** No accommodation source exists: Booking.com's
//! Demand API needs Managed Affiliate Partner status, Amadeus' self-service portal
//! is gone, and Duffel was declined for its search-to-book ratio. `stay_cents` is
//! therefore `null` with a reason attached, never a zero and never an estimate —
//! an invented nightly rate would be indistinguishable from a found one, which is
//! the failure this whole domain is built to avoid.
//!
//! So a base is ranked on **what it costs to get there and away again**, and the
//! things a reader can weigh for themselves are reported beside it. That is a real
//! answer to a real question; it is not the whole answer, and it says so in the
//! body rather than in a comment.

use serde::{Deserialize, Serialize};

use crate::plan_search::{DestinationCandidate, Sources};
use crate::store::{PlaceKind, PlaceRef};

/// What a caller sends. `request_schema` publishes this shape on `/routes`.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BaseRequest {
    /// Where the traveller is now.
    pub from: PlaceRef,
    /// The fixed point the stay has to end at — an event, a flight, someone waiting.
    pub anchor: PlaceRef,
    /// `YYYY-MM-DD`. The day the stay could begin.
    pub from_date: String,
    /// `YYYY-MM-DD`. The day the anchor is due, so the onward leg leaves on it.
    pub anchor_date: String,
    /// How many bases get a fare probe. Pricing is the only expensive step, so this
    /// is the number that decides the wall time: two searches per candidate.
    #[serde(default)]
    pub max_candidates: Option<usize>,
}

/// Default and bound for the fare probes. Lower than `plan-search`'s eight because
/// every candidate here costs TWO searches, not one.
pub const DEFAULT_MAX_CANDIDATES: usize = 5;
pub const MAX_MAX_CANDIDATES: usize = 10;

/// One leg's measured cost, or the reason it could not be measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegQuote {
    pub cents: i64,
    pub currency: String,
}

/// One candidate base, with what it costs to use it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseCandidate {
    pub place_id: String,
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Getting there.
    pub reach: Option<LegQuote>,
    /// Leaving for the anchor.
    pub onward: Option<LegQuote>,
    /// `reach + onward`, or `None` when either leg could not be priced. Never a
    /// partial sum: half a round trip is not a cheaper round trip.
    pub travel_cents: Option<i64>,
    /// How many people the companion register already puts near here, and for how
    /// many days of the stay. `None` when places could not be asked — different
    /// from a measured zero.
    pub known_companions: Option<u32>,
    pub overlap_days: Option<u32>,
    /// **Always null today.** No accommodation source exists; see the module note.
    pub stay_cents: Option<i64>,
    pub stay_reason: String,
    /// Why this base placed where it did, in the reader's words.
    pub why: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseResult {
    pub from: String,
    pub anchor: String,
    pub from_date: String,
    pub anchor_date: String,
    /// Ranked cheapest-total first. Unpriced candidates sort last rather than being
    /// dropped: "could not be priced" is not "expensive".
    pub candidates: Vec<BaseCandidate>,
    pub considered: usize,
    pub priced: usize,
    /// Everything this answer does not know, in the body.
    pub degraded: Vec<String>,
    pub observed_at: String,
}

/// Why `stay_cents` is null. Served rather than documented so a consumer that never
/// reads this file still learns what the blank means.
pub const NO_ACCOMMODATION_SOURCE: &str =
    "no accommodation source: Booking.com needs Managed Affiliate Partner status, Amadeus' \
     self-service portal is gone, and Duffel was declined for its search-to-book ratio. The \
     stay's price is not estimated here -- an invented nightly rate is indistinguishable \
     from a found one";

/// Rank candidate bases for a stay between two fixed points.
///
/// Synchronous on purpose. A transit search answers in well under a second warm, so
/// ten candidates cost about ten seconds — which is inside what a request may
/// reasonably take, and avoids a second copy of the job machinery `plan-search`
/// needs for a two-hundred-second worst case this does not have.
pub fn rank_bases(request: &BaseRequest, sources: &impl Sources) -> Result<BaseResult, String> {
    let limit = request
        .max_candidates
        .unwrap_or(DEFAULT_MAX_CANDIDATES)
        .clamp(1, MAX_MAX_CANDIDATES);

    let mut degraded: Vec<String> = Vec::new();
    let mut cities = sources
        .cities()
        .map_err(|reason| format!("places: {reason}"))?;
    // The two fixed points are not candidates: basing yourself where you already are,
    // or where you have to be, is not a base between them.
    //
    // Matched by NAME as well as by id, and that is not belt-and-braces. The place
    // registry holds one city twice under two ids -- measured 2026-09-23, Berlin
    // exists as both `place_43988ddf9fd5d690` and `place_78297627661fcce0` -- so an
    // id comparison lets the duplicate through and offers the origin as a base. That
    // is the identity problem `places` already reports as `merge_candidates`, seen
    // from the consumer's side.
    let fixed = [
        normalize(&request.from.name),
        normalize(&request.anchor.name),
    ];
    cities.retain(|city| {
        !fixed.contains(&normalize(&city.destination.name))
            && city.destination.id != request.from.id
            && city.destination.id != request.anchor.id
    });
    let considered = cities.len();
    cities.truncate(limit);

    let mut candidates: Vec<BaseCandidate> = Vec::new();
    for city in cities {
        candidates.push(base_for(request, sources, &city, &mut degraded));
    }

    // Cheapest total first, and unpriced last. Stable, so two bases the fare cannot
    // separate keep the registry's order rather than an arbitrary one.
    candidates.sort_by(|a, b| {
        let left = a.travel_cents.unwrap_or(i64::MAX);
        let right = b.travel_cents.unwrap_or(i64::MAX);
        left.cmp(&right)
    });
    let priced = candidates
        .iter()
        .filter(|c| c.travel_cents.is_some())
        .count();

    if priced < candidates.len() {
        degraded.push(format!(
            "{} of {} base(s) could not be priced on both legs and are ranked last",
            candidates.len() - priced,
            candidates.len()
        ));
    }
    degraded.push(NO_ACCOMMODATION_SOURCE.to_string());

    Ok(BaseResult {
        from: request.from.name.clone(),
        anchor: request.anchor.name.clone(),
        from_date: request.from_date.clone(),
        anchor_date: request.anchor_date.clone(),
        candidates,
        considered,
        priced,
        degraded,
        observed_at: sources.observed_at(),
    })
}

fn base_for(
    request: &BaseRequest,
    sources: &impl Sources,
    city: &DestinationCandidate,
    degraded: &mut Vec<String>,
) -> BaseCandidate {
    let reach = sources
        .price(&request.from, city, &request.from_date)
        .map_err(|reason| degraded.push(format!("{} reach: {reason}", city.destination.name)))
        .ok()
        .flatten()
        .map(|cents| LegQuote {
            cents,
            currency: "EUR".to_string(),
        });
    let onward = sources
        .price(
            &city.destination,
            &anchor_as_candidate(request),
            &request.anchor_date,
        )
        .map_err(|reason| degraded.push(format!("{} onward: {reason}", city.destination.name)))
        .ok()
        .flatten()
        .map(|cents| LegQuote {
            cents,
            currency: "EUR".to_string(),
        });

    // Half a round trip is not a cheaper round trip: a total needs both legs.
    let travel_cents = match (&reach, &onward) {
        (Some(a), Some(b)) => Some(a.cents + b.cents),
        _ => None,
    };

    let (known_companions, overlap_days) =
        match (city.destination.latitude, city.destination.longitude) {
            (Some(latitude), Some(longitude)) => {
                match sources.presence(
                    latitude,
                    longitude,
                    &request.from_date,
                    &request.anchor_date,
                ) {
                    Ok((companions, days)) => (Some(companions), Some(days)),
                    Err(reason) => {
                        degraded.push(format!("{} presence: {reason}", city.destination.name));
                        (None, None)
                    }
                }
            }
            _ => (None, None),
        };

    let mut why: Vec<String> = Vec::new();
    match travel_cents {
        Some(total) => {
            why.push(format!(
                "{:.2} EUR to get there and away again",
                total as f64 / 100.0
            ));
            if let Some(reach) = &reach {
                why.push(format!("in: {:.2} EUR", reach.cents as f64 / 100.0));
            }
            if let Some(onward) = &onward {
                why.push(format!("on: {:.2} EUR", onward.cents as f64 / 100.0));
            }
        }
        None => why.push("one of the two legs could not be priced".into()),
    }
    match known_companions {
        Some(0) => why.push("nobody the register knows is there".into()),
        Some(count) => why.push(format!(
            "{count} known companion(s) nearby for {overlap_days:?} day(s)"
        )),
        None => why.push("presence was not asked".into()),
    }

    BaseCandidate {
        place_id: city.destination.id.clone(),
        name: city.destination.name.clone(),
        latitude: city.destination.latitude,
        longitude: city.destination.longitude,
        reach,
        onward,
        travel_cents,
        known_companions,
        overlap_days,
        stay_cents: None,
        stay_reason: NO_ACCOMMODATION_SOURCE.to_string(),
        why,
    }
}

/// A city name reduced to what two spellings of it agree on.
///
/// Lowercased and stripped of everything that is not a letter or a digit, because
/// the registry holds `St. Gallen` and `St Gallen`, and a name with a trailing space.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The anchor as a candidate, because `Sources::price` takes a candidate and the
/// anchor is a `PlaceRef`. One conversion, named, rather than a second pricing path.
fn anchor_as_candidate(request: &BaseRequest) -> DestinationCandidate {
    DestinationCandidate {
        place_id: request.anchor.id.clone(),
        destination: PlaceRef {
            id: request.anchor.id.clone(),
            name: request.anchor.name.clone(),
            kind: PlaceKind::City,
            address: None,
            latitude: request.anchor.latitude,
            longitude: request.anchor.longitude,
        },
        sources: vec!["request".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn place(id: &str, name: &str) -> PlaceRef {
        PlaceRef {
            id: id.into(),
            name: name.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: Some(52.0),
            longitude: Some(13.0),
        }
    }

    fn request() -> BaseRequest {
        BaseRequest {
            from: place("place:berlin", "Berlin"),
            anchor: place("place:hamburg", "Hamburg"),
            from_date: "2026-10-12".into(),
            anchor_date: "2026-10-16".into(),
            max_candidates: None,
        }
    }

    /// A source that prices by name, so a test can say what a leg costs without a
    /// network. `None` means the leg could not be priced.
    struct FakeSources {
        cities: Vec<DestinationCandidate>,
        prices: BTreeMap<String, Option<i64>>,
        presence: Option<(u32, u32)>,
        asked: RefCell<Vec<String>>,
    }

    impl Sources for FakeSources {
        fn calendar_windows(
            &self,
            _from: &str,
            _to: &str,
            _min_days: u32,
        ) -> Result<Vec<crate::plan_search::FeasibleWindow>, String> {
            // A base search is bounded by two given dates, not by calendar windows --
            // it never asks. Empty rather than an error, so the fake stays honest
            // about what was exercised.
            Ok(Vec::new())
        }
        fn cities(&self) -> Result<Vec<DestinationCandidate>, String> {
            Ok(self.cities.clone())
        }
        fn opportunities(&self) -> Result<Vec<crate::plan_search::CandidateEvent>, String> {
            Ok(Vec::new())
        }
        fn climate(
            &self,
            _ids: &[String],
            _month: u32,
        ) -> Result<std::collections::HashMap<String, crate::plan_search::SeasonScore>, String>
        {
            Ok(std::collections::HashMap::new())
        }
        fn price(
            &self,
            origin: &PlaceRef,
            destination: &DestinationCandidate,
            _depart_iso: &str,
        ) -> Result<Option<i64>, String> {
            let key = format!("{}->{}", origin.name, destination.destination.name);
            self.asked.borrow_mut().push(key.clone());
            Ok(self.prices.get(&key).copied().flatten())
        }
        fn presence(
            &self,
            _lat: f64,
            _lon: f64,
            _from: &str,
            _to: &str,
        ) -> Result<(u32, u32), String> {
            self.presence.ok_or_else(|| "places down".to_string())
        }
        fn deadline_spent(&self) -> bool {
            false
        }
        fn observed_at(&self) -> String {
            "2026-09-23T00:00:00Z".into()
        }
    }

    fn candidate(name: &str) -> DestinationCandidate {
        DestinationCandidate {
            place_id: format!("place:{}", name.to_lowercase()),
            destination: place(&format!("place:{}", name.to_lowercase()), name),
            sources: vec!["places".into()],
        }
    }

    fn sources() -> FakeSources {
        let mut prices = BTreeMap::new();
        // Bonn: cheap in, expensive on. Hamburg-early: the reverse.
        prices.insert("Berlin->Bonn".into(), Some(2_174));
        prices.insert("Bonn->Hamburg".into(), Some(5_549));
        prices.insert("Berlin->Leipzig".into(), Some(3_449));
        prices.insert("Leipzig->Hamburg".into(), Some(1_000));
        FakeSources {
            cities: vec![
                candidate("Bonn"),
                candidate("Leipzig"),
                candidate("Berlin"),
                candidate("Hamburg"),
            ],
            prices,
            presence: Some((2, 3)),
            asked: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_base_is_ranked_on_both_legs_together_and_never_on_one() {
        let result = rank_bases(&request(), &sources()).unwrap();
        let names: Vec<&str> = result.candidates.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Leipzig", "Bonn"],
            "Leipzig totals 44.49 against Bonn's 77.23 -- a base that is cheap to reach and \
             dear to leave is not cheap"
        );
        assert_eq!(result.candidates[0].travel_cents, Some(4_449));
        assert_eq!(result.candidates[1].travel_cents, Some(7_723));
    }

    #[test]
    fn the_two_fixed_points_are_not_candidates() {
        // Basing yourself where you already are, or where you must be, is not a base
        // between them -- and pricing those two would burn a fifth of the probes.
        let result = rank_bases(&request(), &sources()).unwrap();
        let names: Vec<&str> = result.candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(!names.contains(&"Berlin"), "the origin is not a base");
        assert!(!names.contains(&"Hamburg"), "the anchor is not a base");
    }

    #[test]
    fn a_half_priced_base_has_no_total_rather_than_a_partial_one() {
        let mut fake = sources();
        fake.prices.insert("Berlin->Bonn".into(), None);
        let result = rank_bases(&request(), &fake).unwrap();
        let bonn = result.candidates.iter().find(|c| c.name == "Bonn").unwrap();
        assert_eq!(
            bonn.travel_cents, None,
            "half a round trip is not a cheaper round trip"
        );
        assert!(
            bonn.onward.is_some(),
            "the leg that did price is still reported"
        );
        assert!(
            result
                .degraded
                .iter()
                .any(|line| line.contains("ranked last")),
            "an unpriced base is stated, not silently dropped: {:?}",
            result.degraded
        );
    }

    #[test]
    fn the_stay_is_never_guessed() {
        // The one thing this must never do. An invented nightly rate is
        // indistinguishable from a found one.
        let result = rank_bases(&request(), &sources()).unwrap();
        for candidate in &result.candidates {
            assert_eq!(candidate.stay_cents, None, "no source exists, so no number");
            assert!(candidate.stay_reason.contains("no accommodation source"));
        }
        assert!(result
            .degraded
            .iter()
            .any(|line| line.contains("no accommodation source")));
    }

    #[test]
    fn a_measured_zero_companions_is_not_the_same_as_not_asked() {
        let mut fake = sources();
        fake.presence = Some((0, 0));
        let result = rank_bases(&request(), &fake).unwrap();
        assert_eq!(result.candidates[0].known_companions, Some(0));
        assert!(result.candidates[0]
            .why
            .iter()
            .any(|w| w.contains("nobody")));

        fake.presence = None;
        let result = rank_bases(&request(), &fake).unwrap();
        assert_eq!(
            result.candidates[0].known_companions, None,
            "places being down is an absent measurement, not an empty register"
        );
        assert!(result.candidates[0]
            .why
            .iter()
            .any(|w| w.contains("not asked")));
    }

    #[test]
    fn the_probe_count_is_bounded_and_the_bound_is_served() {
        let mut many = sources();
        many.cities = (0..40).map(|i| candidate(&format!("City{i}"))).collect();
        let mut ask = request();
        ask.max_candidates = Some(3);
        let result = rank_bases(&ask, &many).unwrap();
        assert_eq!(result.candidates.len(), 3, "three candidates, not forty");
        assert_eq!(
            result.considered, 40,
            "and the pool it drew from is reported"
        );
        assert_eq!(
            many.asked.borrow().len(),
            6,
            "two fare searches per candidate, which is why the default is lower than plan-search's"
        );
    }
}

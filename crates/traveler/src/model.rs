//! The traveller profile: who is travelling, as data a solver reads.
//!
//! Sjel's travel stack could search and could rank, and the ranking weights were
//! the same four constants for every person and every trip. The interests a plan
//! carried were echoed back to the page and read by nothing. This module is the
//! missing half: a stated profile with hard limits a search must obey, weights
//! that are the traveller's rather than the repo's, and a `basis` map that says
//! for every single field whether the operator typed it, something derived it, or
//! it is still the built-in default.
//!
//! The `basis` map is not decoration. `soft.budget_fit` set to `0.30` because a
//! person decided that, and `0.30` because nobody has looked yet, produce
//! identical ranking behaviour and completely different confidence. A profile
//! that cannot tell those apart is a profile that will be trusted further than
//! it deserves, which is the failure `capabilities/punctuality` refuses by
//! returning `None` rather than a zero.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The profile every consumer reads unless it names another.
pub const DEFAULT_PROFILE_ID: &str = "default";

/// How far the five weights may miss 1.0 before a write is refused.
///
/// A tolerance rather than an equality because `0.30 + 0.25 + 0.15 + 0.20 + 0.10`
/// is not 1.0 in binary floating point, and a gate that rejected the arithmetic
/// a person would write by hand would be rejecting the wrong thing.
pub const WEIGHT_SUM_TOLERANCE: f64 = 1e-6;

/// Every field `basis` may name, in the order the profile declares them.
///
/// One list, so a test can assert the map covers it exactly. A `basis` that
/// silently omits a field is how "I set this" and "nobody has looked" start
/// reading the same.
pub const BASIS_KEYS: &[&str] = &[
    "hard.earliest_departure",
    "hard.latest_arrival",
    "hard.max_changes",
    "hard.min_transfer_buffer_min",
    "hard.modes",
    "hard.avoid_overnight_travel",
    "hard.home_stations",
    "hard.home_airports",
    "hard.cards",
    "soft.budget_fit",
    "soft.feasibility",
    "soft.season",
    "soft.events",
    "soft.retrospective",
    "journey.price",
    "journey.duration",
    "journey.changes",
    "journey.reliability",
    "interests",
    "pace",
    "anchors",
];

/// Where one profile value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// The built-in value. Nobody has established this yet, and it is the only
    /// provenance a reader should treat as provisional.
    Default,
    /// The operator typed it. Nothing derived may overwrite it.
    Stated,
    /// Computed from stored rows — plans, offered options, retrospectives.
    Derived,
    /// Proposed from the vault's TELOS notes and not yet confirmed.
    Vault,
}

/// The limits a search must obey, not trade off.
///
/// Every field is optional and every absent field means "no limit", so a profile
/// with nothing stated constrains nothing and the search behaves exactly as it
/// did before this capability existed. That is the same rule
/// `capabilities/punctuality` states for an unscored leg: absence degrades, it
/// never fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HardConstraints {
    /// `HH:MM`, local to the boarding station. No departure before this.
    pub earliest_departure: Option<String>,
    /// `HH:MM`, local to the arriving station. No arrival after this.
    pub latest_arrival: Option<String>,
    /// Changes beyond this are refused outright.
    pub max_changes: Option<u32>,
    /// A transfer below this many minutes is refused. Minutes, not a risk
    /// figure: the risk belongs to `punctuality` and is read per leg, while
    /// this is the floor a person wants regardless of what the history says.
    pub min_transfer_buffer_min: Option<u32>,
    /// Allowed modes. Empty means no restriction.
    pub modes: Vec<String>,
    /// Overnight travel is refused rather than merely penalised.
    pub avoid_overnight_travel: bool,
    /// The stations a search may default its origin to, best first.
    ///
    /// A list rather than one station because a home is not one platform: the
    /// operator named three, and a single value would have silently dropped two
    /// of them. The FIRST is the default origin; the rest are the alternates a
    /// caller may ask for by name. Order is the operator's, and nothing here
    /// re-ranks it.
    pub home_stations: Vec<String>,
    /// The airports a flight search may default its origin to, best first. Same
    /// shape and the same reason: the operator named a second airport that is
    /// easy for them, and a `Option<String>` cannot hold it.
    pub home_airports: Vec<String>,
    /// Discount cards the fare lookups must price against, e.g.
    /// `bahncard_25`, `deutschlandticket`.
    pub cards: Vec<String>,
}

impl Default for HardConstraints {
    fn default() -> Self {
        Self {
            earliest_departure: None,
            latest_arrival: None,
            max_changes: None,
            min_transfer_buffer_min: None,
            // Rail and flight, because those are the two the stack can actually
            // search. An empty list would mean "no restriction", which is not
            // what an unconfigured profile should claim.
            modes: vec!["rail".into(), "flight".into()],
            avoid_overnight_travel: false,
            home_stations: Vec::new(),
            home_airports: Vec::new(),
            cards: Vec::new(),
        }
    }
}

/// The weights a ranking reads, keyed exactly as `plan_search`'s factors are.
///
/// The keys are not decoration either: `capabilities/trips/src/plan_search.rs`
/// builds `ScoreFactor`s with `key: "budget_fit"` and friends, drops any factor
/// it could not compute, and re-normalises the rest. Naming the same keys here
/// is what lets that file read a profile weight by factor key without a
/// translation table — and a translation table is where the two would drift.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SoftWeights {
    pub budget_fit: f64,
    pub feasibility: f64,
    pub season: f64,
    pub events: f64,
    /// The slot `plan_search.rs` declares and does not yet compute
    /// (`FACTOR_RETROSPECTIVE`). Carried here now so the profile shape does not
    /// change on the day the factor lands.
    pub retrospective: f64,
}

impl Default for SoftWeights {
    /// The defaults equal `plan-search-v2`'s constants
    /// (`capabilities/trips/src/plan_search.rs`, `WEIGHT_BUDGET_FIT` and the four
    /// after it), so an unwritten profile ranks exactly as having no profile does.
    /// The numbers do not tell a personalised profile from an unpersonalised one;
    /// `basis` does, because it marks each of these `default` until a person
    /// states a value.
    fn default() -> Self {
        Self {
            budget_fit: 0.30,
            feasibility: 0.25,
            season: 0.15,
            events: 0.20,
            retrospective: 0.10,
        }
    }
}

impl SoftWeights {
    pub fn sum(&self) -> f64 {
        self.budget_fit + self.feasibility + self.season + self.events + self.retrospective
    }

    /// Whether the five weights sum to 1.0 within [`WEIGHT_SUM_TOLERANCE`].
    pub fn is_normalised(&self) -> bool {
        (self.sum() - 1.0).abs() <= WEIGHT_SUM_TOLERANCE
    }

    /// The first weight below 0 (or NaN), by key. A sum of 1 does not exclude
    /// `-0.5` and `1.5`, and a negative weight turns a good factor into a penalty.
    pub fn first_negative(&self) -> Option<(&'static str, f64)> {
        first_negative(&[
            ("budget_fit", self.budget_fit),
            ("feasibility", self.feasibility),
            ("season", self.season),
            ("events", self.events),
            ("retrospective", self.retrospective),
        ])
    }
}

fn first_negative(weights: &[(&'static str, f64)]) -> Option<(&'static str, f64)> {
    weights
        .iter()
        .copied()
        .find(|(_, value)| value.is_nan() || *value < 0.0)
}

/// How full a day is meant to be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    Slow,
    #[default]
    Balanced,
    Packed,
}

/// The weights a *journey* ranking reads, keyed to what a connection can be
/// scored on.
///
/// A second block rather than four more fields on [`SoftWeights`], because the
/// two rank different things: `soft` ranks destinations — is this a good place
/// to go — and this ranks connections — is this a good way to get there. One
/// block would have to carry keys that mean nothing at one of the two grains,
/// and `plan_search` re-normalises over the factors it could compute, so a key
/// that never applies is not merely unused, it silently takes weight away from
/// the ones that do.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct JourneyWeights {
    /// Cheaper is better, scored against the cheapest journey in the same answer.
    pub price: f64,
    /// Shorter door to door.
    pub duration: f64,
    /// Fewer changes. A penalty, never a refusal: the operator declined a change
    /// ceiling, and `min_transfer_buffer_min` is null for the same reason.
    pub changes: f64,
    /// `reliability.probability` when punctuality has it. A journey whose
    /// reliability is unknown scores neither well nor badly — it is dropped from
    /// the weighted sum and the remaining weights are re-normalised, because
    /// "nobody measured this" is not "this is unreliable".
    pub reliability: f64,
}

impl Default for JourneyWeights {
    /// Deliberately NOT stated on anyone's behalf. These are a starting point for
    /// a profile that has never been written, and `basis` says so — the ranking
    /// only runs once `journey.price` is `stated`, so nothing here changes a
    /// search until the operator makes it theirs.
    fn default() -> Self {
        Self {
            price: 0.30,
            duration: 0.20,
            changes: 0.15,
            reliability: 0.35,
        }
    }
}

impl JourneyWeights {
    pub fn sum(&self) -> f64 {
        self.price + self.duration + self.changes + self.reliability
    }

    pub fn is_normalised(&self) -> bool {
        (self.sum() - 1.0).abs() <= WEIGHT_SUM_TOLERANCE
    }

    /// The first weight below 0 (or NaN), by key. See [`SoftWeights::first_negative`].
    pub fn first_negative(&self) -> Option<(&'static str, f64)> {
        first_negative(&[
            ("price", self.price),
            ("duration", self.duration),
            ("changes", self.changes),
            ("reliability", self.reliability),
        ])
    }
}

/// What a trip is anchored on. The order is the preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// A dated event — a conference, a hackathon, a show.
    Event,
    /// Visiting someone.
    Social,
    /// Doing something: a hike, a climb, water.
    Activity,
    /// Being somewhere to work.
    Work,
    /// Being somewhere to do nothing.
    Rest,
}

/// The profile as stored: the input plus the facts the server owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TravelProfile {
    pub id: String,
    pub hard: HardConstraints,
    pub soft: SoftWeights,
    /// The weights a connection ranking reads. Separate grain, separate block.
    pub journey: JourneyWeights,
    pub interests: Vec<String>,
    pub pace: Pace,
    pub anchors: Vec<Anchor>,
    /// One entry per field in [`BASIS_KEYS`]. A missing key is a bug, not a
    /// default — `ProfileInput::validate` refuses a map that does not cover the
    /// set exactly.
    pub basis: BTreeMap<String, Provenance>,
    /// 0 means nothing has ever been stored. Bumped on every write, and the
    /// value a conditional write compares against.
    pub revision: u32,
    pub updated_at: String,
}

impl TravelProfile {
    /// What a consumer gets before anything has been stored.
    ///
    /// Returned rather than a 404, because every consumer has to work with no
    /// profile at all and a `404` would make each of them invent the same
    /// fallback. `revision: 0` is how a caller tells this apart from a stored
    /// profile that happens to hold the defaults.
    pub fn unstated() -> Self {
        let mut basis = BTreeMap::new();
        for key in BASIS_KEYS {
            basis.insert((*key).to_string(), Provenance::Default);
        }
        Self {
            id: DEFAULT_PROFILE_ID.to_string(),
            hard: HardConstraints::default(),
            soft: SoftWeights::default(),
            journey: JourneyWeights::default(),
            interests: Vec::new(),
            pace: Pace::default(),
            anchors: Vec::new(),
            basis,
            revision: 0,
            updated_at: String::new(),
        }
    }

    /// The destination weights `plan_search` should use, or `None` when nothing
    /// has been stated and the caller should keep its own defaults.
    ///
    /// This is the seam that makes the upgrade safe: an unstated profile returns
    /// `None`, so a search against a fresh install ranks exactly as
    /// `plan-search-v1` did.
    pub fn stated_weights(&self) -> Option<SoftWeights> {
        self.stated("soft.budget_fit").then_some(self.soft)
    }

    /// The journey weights a connection ranking should use, or `None` when nothing
    /// has been stated.
    ///
    /// The same seam at the other grain, and the same consequence: with nothing
    /// stated a journey search returns the backend's own order, exactly as it did
    /// before this block existed.
    pub fn stated_journey_weights(&self) -> Option<JourneyWeights> {
        self.stated("journey.price").then_some(self.journey)
    }

    /// Fill in any basis key this profile does not carry.
    ///
    /// A row written before a block existed has no provenance for that block's
    /// fields, and the honest reading of an absent entry is `default`: nobody
    /// stated it. Without this such a row is readable but not writable — a GET
    /// followed by a PUT is refused for a field the operator never touched,
    /// which makes a migration that adds a block break the round trip. Found
    /// live: the journey column arrived on an existing row and every write of
    /// that profile was then rejected.
    pub fn with_complete_basis(mut self) -> Self {
        // A key that is no longer declared is dropped. Renaming a field leaves its
        // old name behind in every stored row, and `validate` refuses an unknown
        // key, so a row written before a rename would be readable and not
        // writable — the same failure the journey block caused from the other
        // direction, which is why both are handled here rather than one.
        self.basis
            .retain(|key, _| BASIS_KEYS.contains(&key.as_str()));
        for key in BASIS_KEYS {
            self.basis
                .entry((*key).to_string())
                .or_insert(Provenance::Default);
        }
        self
    }

    /// Whether one field has been established rather than left at its default.
    fn stated(&self, key: &str) -> bool {
        self.basis
            .get(key)
            .is_some_and(|provenance| *provenance != Provenance::Default)
    }
}

/// The write body. The server owns `id`, `revision` and `updated_at`, so a
/// caller cannot set them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileInput {
    pub hard: HardConstraints,
    pub soft: SoftWeights,
    pub journey: JourneyWeights,
    pub interests: Vec<String>,
    pub pace: Pace,
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub basis: BTreeMap<String, Provenance>,
}

impl ProfileInput {
    /// What a caller with nothing to say sends. Every field at its default and
    /// every basis entry `default`.
    pub fn unstated() -> Self {
        let profile = TravelProfile::unstated();
        Self {
            hard: profile.hard,
            soft: profile.soft,
            journey: profile.journey,
            interests: profile.interests,
            pace: profile.pace,
            anchors: profile.anchors,
            basis: profile.basis,
        }
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        for (block, negative) in [
            ("soft", self.soft.first_negative()),
            ("journey", self.journey.first_negative()),
        ] {
            if let Some((key, value)) = negative {
                return Err(ProfileError::NegativeWeight { block, key, value });
            }
        }
        if !self.soft.is_normalised() {
            return Err(ProfileError::WeightSum {
                block: "soft",
                sum: self.soft.sum(),
            });
        }
        if !self.journey.is_normalised() {
            return Err(ProfileError::WeightSum {
                block: "journey",
                sum: self.journey.sum(),
            });
        }
        for (field, value) in [
            ("earliest_departure", &self.hard.earliest_departure),
            ("latest_arrival", &self.hard.latest_arrival),
        ] {
            if let Some(value) = value {
                if !is_clock(value) {
                    return Err(ProfileError::Clock {
                        field,
                        value: value.clone(),
                    });
                }
            }
        }
        for key in self.basis.keys() {
            if !BASIS_KEYS.contains(&key.as_str()) {
                return Err(ProfileError::UnknownBasisKey(key.clone()));
            }
        }
        for key in BASIS_KEYS {
            if !self.basis.contains_key(*key) {
                return Err(ProfileError::MissingBasisKey((*key).to_string()));
            }
        }
        Ok(())
    }
}

/// `HH:MM`, 24-hour, zero-padded. Minutes past the hour must be under 60 and
/// hours under 24; `7:5` and `25:00` are both refused rather than repaired.
pub fn is_clock(value: &str) -> bool {
    let Some((hours, minutes)) = value.split_once(':') else {
        return false;
    };
    if hours.len() != 2 || minutes.len() != 2 {
        return false;
    }
    if !hours.bytes().all(|b| b.is_ascii_digit()) || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (Ok(hours), Ok(minutes)) = (hours.parse::<u32>(), minutes.parse::<u32>()) else {
        return false;
    };
    hours < 24 && minutes < 60
}

/// Why a profile write was refused. Each variant carries the value that was
/// wrong, because "invalid profile" tells a caller nothing it can act on.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileError {
    /// The block that failed and the sum it reached, because the two blocks have
    /// different keys and "weights must sum to 1.0" alone does not say which four.
    WeightSum {
        block: &'static str,
        sum: f64,
    },
    /// A weight below 0, or NaN. Checked before the sum, because `-0.5` and `1.5`
    /// sum to 1.
    NegativeWeight {
        block: &'static str,
        key: &'static str,
        value: f64,
    },
    Clock {
        field: &'static str,
        value: String,
    },
    UnknownBasisKey(String),
    MissingBasisKey(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::WeightSum { block, sum } => {
                let keys = match *block {
                    "journey" => "price, duration, changes, reliability",
                    _ => "budget_fit, feasibility, season, events, retrospective",
                };
                write!(f, "{block} weights must sum to 1.0, got {sum:.6} ({keys})")
            }
            ProfileError::NegativeWeight { block, key, value } => {
                write!(f, "{block}.{key} must be 0 or more, got {value}")
            }
            ProfileError::Clock { field, value } => write!(
                f,
                "hard.{field} must be HH:MM in 24-hour form, got {value:?}"
            ),
            ProfileError::UnknownBasisKey(key) => {
                write!(f, "basis names a field the profile does not have: {key:?}")
            }
            ProfileError::MissingBasisKey(key) => write!(
                f,
                "basis is missing {key:?}; every field must say where its value came from"
            ),
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unstated_profile_constrains_nothing_and_claims_no_weights() {
        let profile = TravelProfile::unstated();
        assert_eq!(profile.revision, 0);
        assert!(profile.hard.earliest_departure.is_none());
        assert!(profile.hard.latest_arrival.is_none());
        assert!(profile.hard.max_changes.is_none());
        // The seam the upgrade rests on: nothing stated, so a search keeps its
        // own v1 defaults rather than reading these.
        assert!(profile.stated_weights().is_none());
    }

    #[test]
    fn a_stated_weight_makes_the_profile_the_source() {
        let mut profile = TravelProfile::unstated();
        profile
            .basis
            .insert("soft.budget_fit".into(), Provenance::Stated);
        assert!(profile.stated_weights().is_some());
    }

    #[test]
    fn every_declared_field_has_a_basis_entry_and_none_are_invented() {
        let input = ProfileInput::unstated();
        input.validate().unwrap();
        assert_eq!(input.basis.len(), BASIS_KEYS.len());
        for key in BASIS_KEYS {
            assert!(
                input.basis.contains_key(*key),
                "{key} has no provenance, so a reader cannot tell a decision from a default"
            );
        }
    }

    #[test]
    fn a_row_written_before_a_block_existed_is_readable_and_writable_again() {
        // The shape a deployed row has after a new block's column is added: the
        // block's values are present, its provenance is not.
        let mut profile = TravelProfile::unstated();
        profile.basis.remove("journey.price");
        profile.basis.remove("journey.reliability");
        assert!(profile.stated_journey_weights().is_none());

        let completed = profile.clone().with_complete_basis();
        assert_eq!(
            completed.basis.get("journey.price"),
            Some(&Provenance::Default),
            "an absent entry means nobody stated it"
        );

        // And it now survives the round trip that used to be refused.
        let input = ProfileInput {
            hard: completed.hard,
            soft: completed.soft,
            journey: completed.journey,
            interests: completed.interests,
            pace: completed.pace,
            anchors: completed.anchors,
            basis: completed.basis,
        };
        input.validate().unwrap();
    }

    #[test]
    fn a_renamed_field_leaves_no_unknown_key_behind() {
        // A row written before `home_station` became `home_stations` carries the
        // old name, and an unknown basis key is refused by `validate`. Without the
        // retain above, that row reads fine and cannot be written back at all.
        let mut profile = TravelProfile::unstated();
        profile
            .basis
            .insert("hard.home_station".into(), Provenance::Stated);
        profile
            .basis
            .insert("hard.home_airport".into(), Provenance::Stated);

        let completed = profile.with_complete_basis();
        assert!(
            !completed.basis.contains_key("hard.home_station"),
            "the old name must not survive into a body that validate will refuse"
        );
        assert!(completed.basis.contains_key("hard.home_stations"));
        assert!(completed.basis.contains_key("hard.home_airports"));

        let input = ProfileInput {
            hard: completed.hard,
            soft: completed.soft,
            journey: completed.journey,
            interests: completed.interests,
            pace: completed.pace,
            anchors: completed.anchors,
            basis: completed.basis,
        };
        input.validate().unwrap();
    }

    #[test]
    fn a_basis_that_omits_a_field_is_refused_rather_than_defaulted() {
        let mut input = ProfileInput::unstated();
        input.basis.remove("hard.max_changes");
        assert_eq!(
            input.validate(),
            Err(ProfileError::MissingBasisKey("hard.max_changes".into()))
        );
    }

    #[test]
    fn a_basis_naming_a_field_that_does_not_exist_is_refused() {
        let mut input = ProfileInput::unstated();
        input.basis.insert("soft.vibes".into(), Provenance::Stated);
        assert_eq!(
            input.validate(),
            Err(ProfileError::UnknownBasisKey("soft.vibes".into()))
        );
    }

    #[test]
    fn weights_that_do_not_sum_to_one_are_refused_and_the_sum_is_named() {
        let mut input = ProfileInput::unstated();
        input.soft.events = 0.90;
        match input.validate() {
            Err(ProfileError::WeightSum { block, sum }) => {
                assert_eq!(block, "soft");
                assert!((sum - 1.70).abs() < 1e-9);
            }
            other => panic!("expected a weight-sum refusal, got {other:?}"),
        }
    }

    /// A sum of 1 is not enough: `-0.5` and `1.5` sum to 1 and make a factor a penalty.
    #[test]
    fn a_negative_weight_is_refused_even_when_the_sum_is_one() {
        let mut input = ProfileInput::unstated();
        input.soft.budget_fit = -0.30;
        input.soft.events = 0.80;
        assert!((input.soft.sum() - 1.0).abs() < 1e-9);
        match input.validate() {
            Err(ProfileError::NegativeWeight { block, key, value }) => {
                assert_eq!((block, key), ("soft", "budget_fit"));
                assert!((value + 0.30).abs() < 1e-9);
            }
            other => panic!("expected a negative-weight refusal, got {other:?}"),
        }

        let mut journey = ProfileInput::unstated();
        journey.journey.price = -0.10;
        journey.journey.reliability = 0.75;
        assert!(matches!(
            journey.validate(),
            Err(ProfileError::NegativeWeight {
                block: "journey",
                key: "price",
                ..
            })
        ));

        // Zero is a legal weight: it switches a factor off.
        let mut zero = ProfileInput::unstated();
        zero.soft.retrospective = 0.0;
        zero.soft.events = 0.30;
        assert_eq!(zero.validate(), Ok(()));
    }

    #[test]
    fn the_default_weights_are_normalised() {
        assert!(SoftWeights::default().is_normalised());
        assert!(JourneyWeights::default().is_normalised());
    }

    #[test]
    fn the_journey_block_is_refused_with_its_own_keys_named() {
        // Two blocks with different keys means "weights must sum to 1.0" alone
        // does not tell a caller which four numbers to fix.
        let mut input = ProfileInput::unstated();
        input.journey.changes = 0.5;
        match input.validate() {
            Err(ProfileError::WeightSum { block, .. }) => assert_eq!(block, "journey"),
            other => panic!("expected a journey weight refusal, got {other:?}"),
        }
        let message = ProfileError::WeightSum {
            block: "journey",
            sum: 1.3,
        }
        .to_string();
        assert!(
            message.contains("price, duration, changes, reliability"),
            "the refusal must name the four keys, got: {message}"
        );
    }

    #[test]
    fn an_unstated_journey_block_ranks_nothing() {
        // The same seam as the destination weights, at the other grain: with
        // nothing stated a journey search keeps the backend's own order.
        let mut profile = TravelProfile::unstated();
        assert!(profile.stated_journey_weights().is_none());
        profile
            .basis
            .insert("journey.price".into(), Provenance::Stated);
        assert!(profile.stated_journey_weights().is_some());
    }

    #[test]
    fn both_weight_blocks_are_declared_in_basis() {
        // A block whose keys are absent from BASIS_KEYS would be written with no
        // provenance at all, and `validate` would refuse every write.
        for key in [
            "journey.price",
            "journey.duration",
            "journey.changes",
            "journey.reliability",
        ] {
            assert!(BASIS_KEYS.contains(&key), "{key} is not in BASIS_KEYS");
        }
        let input = ProfileInput::unstated();
        input.validate().unwrap();
        assert_eq!(input.basis.len(), BASIS_KEYS.len());
    }

    #[test]
    fn a_clock_is_hh_mm_and_everything_else_is_refused() {
        for good in ["00:00", "07:00", "23:59", "09:05"] {
            assert!(is_clock(good), "{good} should be a clock");
        }
        for bad in [
            "7:00", "07:0", "24:00", "23:60", "0700", "", "07:00:00", "aa:bb",
        ] {
            assert!(!is_clock(bad), "{bad} should not be a clock");
        }
    }

    #[test]
    fn a_bad_clock_is_refused_and_the_field_is_named() {
        let mut input = ProfileInput::unstated();
        input.hard.latest_arrival = Some("24:30".into());
        match input.validate() {
            Err(ProfileError::Clock { field, value }) => {
                assert_eq!(field, "latest_arrival");
                assert_eq!(value, "24:30");
            }
            other => panic!("expected a clock refusal, got {other:?}"),
        }
    }
}

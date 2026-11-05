//! Pure wire models, contracts, and route constants for the Sjel traveler capability.
//!
//! Shared across server capability (`sjel-traveler`), travel solver consumers (`sjel-trips`),
//! and native UI clients.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use transit_api::JourneyWeights;

// ============================================================================
// Route Constants
// ============================================================================

pub const DEFAULT_TRAVELER_PORT: u16 = 8096;

pub const ROUTE_TRAVELER_HEALTH: &str = "/health";
pub const ROUTE_TRAVELER_READY: &str = "/ready";
pub const ROUTE_TRAVELER_ROUTES: &str = "/routes";
pub const ROUTE_TRAVELER_PROFILE: &str = "/api/profile";
pub const ROUTE_TRAVELER_PROFILE_DERIVED: &str = "/api/profile/derived";
pub const ROUTE_TRAVELER_PROFILE_PROPOSALS: &str = "/api/profile/proposals";

// ============================================================================
// Profile Constants
// ============================================================================

pub const DEFAULT_PROFILE_ID: &str = "default";
pub const WEIGHT_SUM_TOLERANCE: f64 = 1e-6;

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

// ============================================================================
// Traveller Profile Models
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Default,
    Stated,
    Derived,
    Vault,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HardConstraints {
    pub earliest_departure: Option<String>,
    pub latest_arrival: Option<String>,
    pub max_changes: Option<u32>,
    pub min_transfer_buffer_min: Option<u32>,
    pub modes: Vec<String>,
    pub avoid_overnight_travel: bool,
    pub home_stations: Vec<String>,
    pub home_airports: Vec<String>,
    pub cards: Vec<String>,
}

impl Default for HardConstraints {
    fn default() -> Self {
        Self {
            earliest_departure: None,
            latest_arrival: None,
            max_changes: None,
            min_transfer_buffer_min: None,
            modes: vec!["rail".into(), "flight".into()],
            avoid_overnight_travel: false,
            home_stations: Vec::new(),
            home_airports: Vec::new(),
            cards: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SoftWeights {
    pub budget_fit: f64,
    pub feasibility: f64,
    pub season: f64,
    pub events: f64,
    pub retrospective: f64,
}

impl Default for SoftWeights {
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

    pub fn is_normalised(&self) -> bool {
        (self.sum() - 1.0).abs() <= WEIGHT_SUM_TOLERANCE
    }

    pub fn first_negative(&self) -> Option<(&'static str, f64)> {
        for (key, val) in [
            ("budget_fit", self.budget_fit),
            ("feasibility", self.feasibility),
            ("season", self.season),
            ("events", self.events),
            ("retrospective", self.retrospective),
        ] {
            if val.is_nan() || val < 0.0 {
                return Some((key, val));
            }
        }
        None
    }

    pub fn clamped(self) -> Self {
        let floor = |w: f64| if w > 0.0 { w } else { 0.0 };
        Self {
            budget_fit: floor(self.budget_fit),
            feasibility: floor(self.feasibility),
            season: floor(self.season),
            events: floor(self.events),
            retrospective: floor(self.retrospective),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    Slow,
    #[default]
    Balanced,
    Packed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Event,
    Social,
    Activity,
    Work,
    Rest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TravelProfile {
    pub id: String,
    pub hard: HardConstraints,
    pub soft: SoftWeights,
    pub journey: JourneyWeights,
    pub interests: Vec<String>,
    pub pace: Pace,
    pub anchors: Vec<Anchor>,
    pub basis: BTreeMap<String, Provenance>,
    pub revision: u32,
    pub updated_at: String,
}

impl TravelProfile {
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

    pub fn stated_weights(&self) -> Option<SoftWeights> {
        self.stated("soft.budget_fit").then_some(self.soft)
    }

    pub fn stated_journey_weights(&self) -> Option<JourneyWeights> {
        self.stated("journey.price").then_some(self.journey)
    }

    pub fn with_complete_basis(mut self) -> Self {
        self.basis
            .retain(|key, _| BASIS_KEYS.contains(&key.as_str()));
        for key in BASIS_KEYS {
            self.basis
                .entry((*key).to_string())
                .or_insert(Provenance::Default);
        }
        self
    }

    fn stated(&self, key: &str) -> bool {
        self.basis
            .get(key)
            .is_some_and(|provenance| *provenance != Provenance::Default)
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileError {
    WeightSum {
        block: &'static str,
        sum: f64,
    },
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct PutProfileRequest {
    pub profile: ProfileInput,
    #[serde(default)]
    pub expected_revision: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ProfileEnvelope {
    pub profile: TravelProfile,
    #[serde(default)]
    pub stored: bool,
}

// ============================================================================
// Derived Travel Baseline Models
// ============================================================================

pub const TRIPS_PREFIX: &str = "trips";

pub const ATTENDANCE_NOTE: &str =
    "counts plans, not confirmed trips: a plan is excluded only when its retrospective records \
     not_taken, and nothing records that any other plan was attended";

pub const LEAD_TIME_NOTE: &str =
    "lead_time_days covers only plans whose row was created before the trip started: the vault \
     import stamps created_at with the import date, so an imported trip's row age is how long \
     ago the import ran, not how far ahead it was booked";

pub const DESTINATION_NOTE: &str =
    "destinations are as stored, so one city typed two ways counts as two and a multi-stop plan \
     is a single name; repeat_destinations under-counts for the same reason";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Spread {
    pub min: i64,
    pub median: i64,
    pub max: i64,
}

impl Spread {
    pub fn of(mut values: Vec<i64>) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        values.sort_unstable();
        let middle = values.len() / 2;
        let median = if values.len() % 2 == 1 {
            values[middle]
        } else {
            (values[middle - 1] + values[middle]) / 2
        };
        Some(Self {
            min: values[0],
            median,
            max: values[values.len() - 1],
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DestinationCount {
    pub name: String,
    pub trips: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Bucket {
    pub key: String,
    pub trips: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Company {
    pub unrecorded: usize,
    #[serde(default)]
    pub solo: usize,
    pub pair: usize,
    pub group: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompanionPattern {
    pub person_id: String,
    pub trips: usize,
    pub pair_trips: usize,
    pub group_trips: usize,
    pub destinations: Vec<DestinationCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DerivedTravel {
    pub considered: usize,
    pub excluded_not_taken: Vec<String>,
    pub length_days: Option<Spread>,
    pub lead_time_days: Option<Spread>,
    pub lead_time_plans: usize,
    pub lead_time_skipped: usize,
    pub destinations: Vec<DestinationCount>,
    pub repeat_destinations: Vec<DestinationCount>,
    pub company: Company,
    #[serde(default)]
    pub companion_patterns: Vec<CompanionPattern>,
    pub months: Vec<Bucket>,
    pub modes: Vec<Bucket>,
    pub basis: Vec<String>,
    pub notes: Vec<String>,
}

// ============================================================================
// Vault Proposal Models
// ============================================================================

pub const ADMISSIBLE_SECTIONS: &[&str] = &[
    "frontmatter:summary",
    "frontmatter:current_focus",
    "frontmatter:category_affinity",
    "callout:Charter",
    "heading:Interests",
    "heading:Preferences",
    "heading:Constraints",
    "heading:Travel",
];

pub const EXCLUDED_HEADINGS: &[&str] = &["CURRENT_STATE", "Current State", "Draft", "History"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldProposal {
    pub field: String,
    pub current: Value,
    pub proposed: Value,
    pub source_file: String,
    pub section: String,
    pub quoted_span: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedSection {
    pub source_file: String,
    pub section: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VaultProposals {
    pub available: bool,
    pub scanned_files: Vec<String>,
    pub proposals: Vec<FieldProposal>,
    pub skipped_sections: Vec<SkippedSection>,
}

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
    fn clock_validation_matches_format() {
        assert!(is_clock("08:30"));
        assert!(is_clock("23:59"));
        assert!(!is_clock("24:00"));
        assert!(!is_clock("7:30"));
        assert!(!is_clock("invalid"));
    }
}

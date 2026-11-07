//! Pure wire models and route constants for transit.
//!
//! Shared across `sjel-transit` (backend) and `sjel-gpui-core` (client/macOS/iOS).
//! Zero server dependencies.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ROUTE_HEALTH: &str = "/health";
pub const ROUTE_ROUTES: &str = "/routes";
pub const ROUTE_API_HEALTH: &str = "/api/health";
pub const ROUTE_SUGGEST: &str = "/api/suggest";
pub const ROUTE_SEARCH: &str = "/api/search";
pub const ROUTE_SPLIT: &str = "/api/split";
pub const ROUTE_TRIPS: &str = "/api/trips";
pub const ROUTE_TICKETS_EXTRACT: &str = "/api/tickets/extract";

pub const PROXY_ROUTE_TRANSIT_HEALTH: &str = "/transit/health";
pub const PROXY_ROUTE_TRANSIT_API_HEALTH: &str = "/transit/api/health";
pub const PROXY_ROUTE_TRANSIT_SUGGEST: &str = "/transit/api/suggest";
pub const PROXY_ROUTE_TRANSIT_SEARCH: &str = "/transit/api/search";
pub const PROXY_ROUTE_TRANSIT_SPLIT: &str = "/transit/api/split";
pub const PROXY_ROUTE_TRANSIT_TRIPS: &str = "/transit/api/trips";
pub const PROXY_ROUTE_TRANSIT_TICKETS_EXTRACT: &str = "/transit/api/tickets/extract";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Station {
    /// EVA code, e.g. "8000105"
    pub id: String,
    /// e.g. "Frankfurt(Main)Hbf"
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Leg {
    pub origin: Station,
    pub destination: Station,
    /// The time to plan around: real-time when HAFAS has one, scheduled otherwise.
    pub departure_time: String,
    pub arrival_time: String,
    #[serde(default)]
    pub scheduled_departure: Option<String>,
    #[serde(default)]
    pub realtime_departure: Option<String>,
    #[serde(default)]
    pub scheduled_arrival: Option<String>,
    #[serde(default)]
    pub realtime_arrival: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_utc: Option<String>,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub train_name: String,
    #[serde(default)]
    pub train_number: String,
    #[serde(default)]
    pub train_category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub is_regional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_time_probability: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ArrivalPunctuality {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_name: Option<String>,
    #[serde(default)]
    pub train_type: String,
    #[serde(default)]
    pub hour: i16,
    #[serde(default)]
    pub weekend: bool,
    #[serde(default)]
    pub n: i64,
    #[serde(default)]
    pub mean_delay: f32,
    #[serde(default)]
    pub p50: i16,
    #[serde(default)]
    pub p90: i16,
    #[serde(default)]
    pub share_late_6: f32,
    #[serde(default)]
    pub cancel_rate: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct TransferReliability {
    pub station: Station,
    pub buffer_minutes: i64,
    pub catch_probability: f64,
    pub n: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct JourneyReliability {
    pub probability: f64,
    #[serde(default)]
    pub threshold_minutes: i32,
    #[serde(default)]
    pub final_leg_on_time: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfers: Vec<TransferReliability>,
    #[serde(default)]
    pub min_sample: i64,
}

pub type Reliability = JourneyReliability;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RankFactor {
    #[serde(default)]
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub score: f64,
    pub weight: f64,
    pub rationale: String,
}

pub const WEIGHT_SUM_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct JourneyWeights {
    pub price: f64,
    pub duration: f64,
    pub changes: f64,
    pub reliability: f64,
}

impl Default for JourneyWeights {
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

    pub fn first_negative(&self) -> Option<(&'static str, f64)> {
        for (key, val) in [
            ("price", self.price),
            ("duration", self.duration),
            ("changes", self.changes),
            ("reliability", self.reliability),
        ] {
            if val.is_nan() || val < 0.0 {
                return Some((key, val));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JourneyRanking {
    #[serde(default)]
    pub score: f64,
    pub rank: usize,
    pub factors: Vec<RankFactor>,
    #[serde(default)]
    pub weights: JourneyWeights,
    pub source: String,
}

pub type Ranking = JourneyRanking;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UnscoredLeg {
    pub leg_index: usize,
    pub train_name: String,
    pub train_category: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Journey {
    pub id: String,
    pub start_station: Station,
    pub end_station: Station,
    pub legs: Vec<Leg>,
    pub total_duration_minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_punctuality: Option<ArrivalPunctuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability: Option<JourneyReliability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unscored_legs: Vec<UnscoredLeg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<JourneyRanking>,
}

/// `HH:MM` out of an ISO 8601 date-time, as transit's local time says it.
pub fn clock(iso: &str) -> &str {
    iso.get(11..16).unwrap_or(iso)
}

impl Journey {
    pub fn changes(&self) -> usize {
        self.legs.len().saturating_sub(1)
    }

    pub fn departs(&self) -> &str {
        self.legs.first().map_or("", |l| clock(&l.departure_time))
    }

    pub fn arrives(&self) -> &str {
        self.legs.last().map_or("", |l| clock(&l.arrival_time))
    }

    /// Minutes the first leg leaves after its timetable, when HAFAS sent both times.
    pub fn delay_minutes(&self) -> Option<i64> {
        let leg = self.legs.first()?;
        let parse = |t: &Option<String>| chrono::DateTime::parse_from_rfc3339(t.as_deref()?).ok();
        let late = parse(&leg.realtime_departure)? - parse(&leg.scheduled_departure)?;
        Some(late.num_minutes()).filter(|m| *m != 0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrainMatch {
    #[default]
    Exact,
    Partial,
    Different,
    Unknown,
}

impl TrainMatch {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Exact => None,
            Self::Partial => Some("partial train match"),
            Self::Different => Some("different train"),
            Self::Unknown => Some("train unknown"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct SplitSegment {
    pub journey: Journey,
    pub train_match: TrainMatch,
    #[serde(default)]
    pub expected_trains: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SplitConfidence {
    #[default]
    Exact,
    Partial,
    Low,
}

pub type Confidence = SplitConfidence;

impl SplitConfidence {
    pub fn note(self) -> Option<&'static str> {
        match self {
            Self::Exact => None,
            Self::Partial => Some(
                "Some tickets could not be matched to the exact train on this route, or a fare \
                 lookup failed. Check each ticket before buying.",
            ),
            Self::Low => Some(
                "At least one ticket is priced for a different train than this journey uses. \
                 Buying this chain does not buy a seat on this trip.",
            ),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ContractBoundary {
    pub station: Station,
    pub same_train: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming_share_late_6: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct SplitResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_price: Option<f64>,
    pub split_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub savings: Option<f64>,
    pub segments: Vec<SplitSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contract_boundaries: Vec<ContractBoundary>,
    pub confidence: SplitConfidence,
    #[serde(default)]
    pub unpriced_pairs: usize,
    #[serde(default)]
    pub queried_pairs: usize,
}

impl SplitResult {
    pub fn saved_percent(&self) -> Option<i64> {
        let (savings, original) = (self.savings?, self.original_price?);
        (original > 0.0).then(|| (savings / original * 100.0).round() as i64)
    }
}

/// The presets transit knows (`crates/transit/src/ranking.rs`, `PRIORITIES`): the id is the
/// contract, the label and hint are for display.
pub const PRIORITIES: [(&str, &str, &str); 5] = [
    ("balanced", "Balanced", "no term dominates"),
    ("cheapest", "Cheapest", "fare first"),
    ("fastest", "Fastest", "door to door"),
    ("fewest_changes", "Fewest changes", "direct where possible"),
    ("reliable", "Most reliable", "measured, not fitted"),
];

pub const PRIORITY_WEIGHTS: &[(&str, JourneyWeights)] = &[
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_serde_roundtrip() {
        let station = Station {
            id: "8000105".into(),
            name: "Frankfurt(Main)Hbf".into(),
            latitude: Some(50.107),
            longitude: Some(8.663),
        };
        let json = serde_json::to_string(&station).unwrap();
        let parsed: Station = serde_json::from_str(&json).unwrap();
        assert_eq!(station, parsed);
    }

    #[test]
    fn split_result_saved_percent() {
        let split = SplitResult {
            original_price: Some(100.0),
            split_price: 75.0,
            savings: Some(25.0),
            segments: vec![],
            contract_boundaries: vec![],
            confidence: SplitConfidence::Exact,
            unpriced_pairs: 0,
            queried_pairs: 1,
        };
        assert_eq!(split.saved_percent(), Some(25));
    }
}

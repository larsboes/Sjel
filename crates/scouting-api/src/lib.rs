//! Typed API contract, wire models, and route constants for the Sjel scouting capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-scouting`)
//! share these definitions to eliminate wire model drift. Zero platform dependencies.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_DISCOVER: &str = "/discover";
pub const ROUTE_SOURCES: &str = "/sources";
pub const ROUTE_OPPORTUNITIES: &str = "/opportunities";
pub const ROUTE_OPPORTUNITY_STATUS: &str = "/opportunities/{id}/status";

pub const PROXY_ROUTE_SCOUTING_DISCOVER: &str = "/scouting/discover";
pub const PROXY_ROUTE_SCOUTING_SOURCES: &str = "/scouting/sources";
pub const PROXY_ROUTE_SCOUTING_OPPORTUNITIES: &str = "/scouting/opportunities";

// ============================================================================
// Status and Routing Enums
// ============================================================================

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStatus {
    New,
    Saved,
    Dismissed,
}

impl OpportunityStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Saved => "saved",
            Self::Dismissed => "dismissed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventRouteKind {
    Local,
    TravelCandidate,
    Online,
    Unresolved,
}

impl EventRouteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::TravelCandidate => "travel_candidate",
            Self::Online => "online",
            Self::Unresolved => "unresolved",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::TravelCandidate => "Travel candidate",
            Self::Online => "Online",
            Self::Unresolved => "Route unresolved",
        }
    }

    pub fn can_add_to_calendar(&self) -> bool {
        matches!(self, Self::Local | Self::Online)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventRouteBasis {
    SourceMetadata,
    LocationText,
    Coordinates,
    Country,
    Timezone,
    OperatorOverride,
    MissingPolicy,
    MissingEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct EventRoute {
    pub route: EventRouteKind,
    pub basis: EventRouteBasis,
    pub reason: String,
    #[serde(default)]
    pub distance_km: Option<f64>,
}

// ============================================================================
// Sources
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ScoutingSource {
    pub id: String,
    pub adapter: String,
    pub enabled: bool,
    pub configured: bool,
    #[serde(default)]
    pub root_path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub opportunity_type: String,
    #[serde(default)]
    pub data_class: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ScoutingSources {
    pub sources: Vec<ScoutingSource>,
    pub count: usize,
}

// ============================================================================
// Opportunities
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct ScoutingOpportunity {
    pub id: String,
    pub opportunity_type: String,
    pub source: String,
    pub title: String,
    pub city: String,
    pub starts_at: String,
    pub ends_at: String,
    pub location: String,
    pub score: f64,
    pub matched_focus: String,
    pub rationale: String,
    pub url: String,
    #[serde(default)]
    pub vault_link: Option<String>,
    pub status: OpportunityStatus,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub event_route: Option<EventRoute>,
    #[serde(default)]
    pub data_class: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct ScoutingOpportunities {
    pub opportunities: Vec<ScoutingOpportunity>,
    pub count: usize,
    pub store_total: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct ScoredOpportunity {
    pub id: String,
    pub rank: usize,
    pub score: f64,
    pub source: String,
    pub title: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub matched_focus: Option<String>,
    pub rationale: String,
    pub url: String,
    pub opportunity_type: String,
    pub status: String,
    #[serde(default)]
    pub vault_link: Option<String>,
    #[serde(default)]
    pub event_route: Option<EventRoute>,
}

pub type ScoredResult = ScoredOpportunity;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DiscoverResponse {
    pub adapter: String,
    pub opportunity_type: String,
    pub total_scored: usize,
    pub new_count: usize,
    pub vault_links: usize,
    pub store_total: usize,
    pub results: Vec<ScoredOpportunity>,
}

// ============================================================================
// Calendar Promotion Contracts
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarCandidate {
    pub id: String,
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarEvidence {
    pub entry_id: String,
    pub kind: String,
    pub commitment: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub impact: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarCandidateVerdict {
    pub id: String,
    pub verdict: String,
    pub starts_at: String,
    pub ends_at: String,
    pub already_in_calendar: bool,
    pub evidence: Vec<CalendarEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarVerdicts {
    pub verdicts: Vec<CalendarCandidateVerdict>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opportunity_status_roundtrip() {
        assert_eq!(OpportunityStatus::New.as_str(), "new");
        assert_eq!(OpportunityStatus::Saved.as_str(), "saved");
        assert_eq!(OpportunityStatus::Dismissed.as_str(), "dismissed");

        let serialized = serde_json::to_string(&OpportunityStatus::Saved).unwrap();
        assert_eq!(serialized, "\"saved\"");
        let deserialized: OpportunityStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, OpportunityStatus::Saved);
    }

    #[test]
    fn test_event_route_kind() {
        assert!(EventRouteKind::Local.can_add_to_calendar());
        assert!(EventRouteKind::Online.can_add_to_calendar());
        assert!(!EventRouteKind::TravelCandidate.can_add_to_calendar());
        assert!(!EventRouteKind::Unresolved.can_add_to_calendar());

        assert_eq!(EventRouteKind::Local.as_str(), "local");
        assert_eq!(EventRouteKind::TravelCandidate.as_str(), "travel_candidate");
    }

    #[test]
    fn test_event_route_serialization() {
        let route = EventRoute {
            route: EventRouteKind::Local,
            basis: EventRouteBasis::Coordinates,
            reason: "Within 25km".into(),
            distance_km: Some(12.5),
        };
        let serialized = serde_json::to_string(&route).unwrap();
        let deserialized: EventRoute = serde_json::from_str(&serialized).unwrap();
        assert_eq!(route, deserialized);
    }
}

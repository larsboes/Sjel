//! Typed API contract, wire models, and route constants for the Sjel trips capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-trips`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_PLANS: &str = "/api/plans";
pub const ROUTE_PLAN_BY_ID: &str = "/api/plans/{id}";
pub const ROUTE_PLAN_ITEMS: &str = "/api/plans/{id}/items";
pub const ROUTE_PLAN_ITEM_BY_ID: &str = "/api/plans/{plan_id}/items/{item_id}";
pub const ROUTE_PLAN_ITEM_FIELDS: &str = "/api/plans/{plan_id}/items/{item_id}/fields";
pub const ROUTE_PLAN_RESOLVE_PLACES: &str = "/api/plans/{id}/resolve-places";
pub const ROUTE_PLAN_RETROSPECTIVE: &str = "/api/plans/{id}/retrospective";
pub const ROUTE_PLAN_OUTCOME: &str = "/api/plans/{id}/outcome";
pub const ROUTE_BASES: &str = "/api/bases";
pub const ROUTE_PEOPLE_OPEN: &str = "/api/people/open";
pub const ROUTE_PEOPLE_DECIDE: &str = "/api/people/decide";
pub const ROUTE_LINKS: &str = "/api/links";

// ============================================================================
// Core Trip and Place Types
// ============================================================================

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Address,
    Airport,
    City,
    #[default]
    Station,
    Venue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlaceRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: PlaceKind,
    #[serde(default)]
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl PlaceRef {
    pub fn coordinate(&self) -> Option<(f64, f64)> {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Bike,
    Bus,
    Car,
    Ferry,
    Flight,
    Train,
    Walk,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Booked,
    Completed,
    OptionSelected,
    #[default]
    Planning,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct TripStage {
    pub id: String,
    pub sequence: usize,
    pub origin: PlaceRef,
    pub destination: PlaceRef,
    pub date: Option<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub status: StageStatus,
    #[serde(default)]
    pub selected_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlanSource {
    pub kind: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlan {
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub stages: Vec<TripStage>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub source: Option<PlanSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct TripPlan {
    pub id: String,
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    pub interests: String,
    pub status: String,
    pub travelers: Vec<String>,
    pub transport_modes: Vec<TransportMode>,
    pub stages: Vec<TripStage>,
    pub cover_image_url: Option<String>,
    pub source: Option<PlanSource>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub budget_cents: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlanItem {
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlanItem {
    pub id: String,
    #[serde(default)]
    pub plan_id: String,
    pub item_type: String,
    pub day: Option<String>,
    #[serde(default)]
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Retrospective {
    pub plan_id: String,
    pub cost_cents: Option<i64>,
    pub currency: Option<String>,
    pub again: String,
    pub change_note: String,
    pub filled_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlanDetails {
    #[serde(flatten)]
    pub plan: TripPlan,
    pub items: Vec<PlanItem>,
    #[serde(default)]
    pub retrospective: Option<Retrospective>,
}

// ============================================================================
// Place Resolution
// ============================================================================

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ResolvePlacesRequest {
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ResolvePlacesResult {
    pub plan_id: String,
    pub updated_at: String,
    pub resolved: Vec<ResolvedPlace>,
    pub unresolved: Vec<UnresolvedPlace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ResolvedPlace {
    pub place_id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct UnresolvedPlace {
    pub place_id: String,
    pub name: String,
    pub reason: String,
}

// ============================================================================
// Plan Item Mutations
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct SetItemDay {
    pub day: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct EditItemPayload {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Map<String, Value>,
}

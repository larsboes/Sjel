//! Pure wire models and route constants for Interior.
//!
//! Shared across `sjel-interior` (backend) and `sjel-gpui-core` (client/macOS/iOS).
//! Zero server dependencies.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ROUTE_HEALTH: &str = "/health";
pub const ROUTE_READY: &str = "/ready";
pub const ROUTE_ROUTES: &str = "/routes";
pub const ROUTE_MODEL: &str = "/api/model";
pub const ROUTE_FLATS: &str = "/api/flats";
pub const ROUTE_LAYOUTS: &str = "/api/layouts";
pub const ROUTE_SEARCH: &str = "/api/search";
pub const ROUTE_COMPOSE: &str = "/api/compose";
pub const ROUTE_JOBS: &str = "/api/jobs";
pub const ROUTE_PLACEMENTS: &str = "/api/placements";
pub const ROUTE_ROOMPLAN_REFERENCE: &str = "/api/roomplan/reference";
pub const ROUTE_ROOMPLAN_ASSET: &str = "/api/roomplan/asset";
pub const ROUTE_ROOMPLAN_REVISIONS: &str = "/api/roomplan/revisions";
pub const ROUTE_ROOMPLAN_CAPTURES: &str = "/api/roomplan/captures";
pub const ROUTE_ROOMPLAN_RECONCILE: &str = "/api/roomplan/reconcile";
pub const ROUTE_ROOMPLAN_PROPOSE_PLACEMENT: &str = "/api/roomplan/propose-placement";
pub const ROUTE_ROOMPLAN_SPACES: &str = "/api/roomplan/spaces";

pub const PROXY_ROUTE_INTERIOR_MODEL: &str = "/interior/api/model";
pub const PROXY_ROUTE_INTERIOR_FLATS: &str = "/interior/api/flats";
pub const PROXY_ROUTE_INTERIOR_LAYOUTS: &str = "/interior/api/layouts";
pub const PROXY_ROUTE_INTERIOR_SEARCH: &str = "/interior/api/search";
pub const PROXY_ROUTE_INTERIOR_COMPOSE: &str = "/interior/api/compose";
pub const PROXY_ROUTE_INTERIOR_JOBS: &str = "/interior/api/jobs";
pub const PROXY_ROUTE_INTERIOR_PLACEMENTS: &str = "/interior/api/placements";
pub const PROXY_ROUTE_INTERIOR_ROOMPLAN_REVISIONS: &str = "/interior/api/roomplan/revisions";
pub const PROXY_ROUTE_INTERIOR_ROOMPLAN_CAPTURES: &str = "/interior/api/roomplan/captures";
pub const PROXY_ROUTE_INTERIOR_ROOMPLAN_RECONCILE: &str = "/interior/api/roomplan/reconcile";
pub const PROXY_ROUTE_INTERIOR_ROOMPLAN_PROPOSE_PLACEMENT: &str = "/interior/api/roomplan/propose-placement";
pub const PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES: &str = "/interior/api/roomplan/spaces";

pub fn valid_roomplan_layout_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanDimensions {
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub depth: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanWall {
    #[serde(default)]
    pub from: [i32; 2],
    #[serde(default)]
    pub to: [i32; 2],
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanOpening {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub wall: String,
    #[serde(default)]
    pub from: i32,
    #[serde(default)]
    pub to: i32,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanFixture {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub x: [i32; 2],
    #[serde(default)]
    pub y: [i32; 2],
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanUncertainty {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanModel {
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub area_m2: f64,
    #[serde(default)]
    pub dimensions: RoomPlanDimensions,
    #[serde(default)]
    pub polygon: Vec<[i32; 2]>,
    #[serde(default)]
    pub walls: BTreeMap<String, RoomPlanWall>,
    #[serde(default)]
    pub openings: Vec<RoomPlanOpening>,
    #[serde(default)]
    pub fixed_furniture: Vec<RoomPlanFixture>,
    #[serde(default)]
    pub unmeasured: Vec<RoomPlanUncertainty>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PlacedItem {
    #[serde(rename = "ref")]
    pub reference: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[i32; 2]>,
    #[serde(default, alias = "typ", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

pub type RoomPlanSavedLayoutItem = PlacedItem;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSavedLayout {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "item", alias = "items")]
    pub items: Vec<RoomPlanSavedLayoutItem>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanSavedLayoutPreview {
    #[serde(default)]
    pub layout: RoomPlanSavedLayout,
    #[serde(default)]
    pub check: serde_json::Value,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSavedAlignment {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub revision_id: String,
    #[serde(default)]
    pub reference_revision_id: String,
    #[serde(default)]
    pub report: serde_json::Value,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanObjectLink {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub revision_id: String,
    #[serde(default)]
    pub observation_id: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpace {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub revision_ids: Vec<String>,
    #[serde(default)]
    pub alignments: Vec<RoomPlanSavedAlignment>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpaceEntry {
    #[serde(default)]
    pub space: RoomPlanSpace,
    #[serde(default)]
    pub object_links: Vec<RoomPlanObjectLink>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpaceSet {
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub spaces: Vec<RoomPlanSpaceEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct Placement {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub since: String,
}

pub type RoomPlanPlacement = Placement;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PlacementHistory {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
    pub revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_observation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<serde_json::Value>,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PlacementBody {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct LayoutBody {
    pub items: Vec<PlacedItem>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanPositionProposal {
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
    #[serde(default)]
    pub center_x_cm: i32,
    #[serde(default)]
    pub center_y_cm: i32,
    #[serde(default)]
    pub observed_width_cm: i32,
    #[serde(default)]
    pub observed_depth_cm: i32,
    #[serde(default)]
    pub alignment_error_m: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanPlacementProposalResponse {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub revision_id: String,
    #[serde(default)]
    pub observation_id: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub flat_alignment: serde_json::Value,
    #[serde(default)]
    pub proposal: RoomPlanPositionProposal,
    #[serde(default)]
    pub requires_review: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpaceReceipt {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub revision_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanAlignment {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub matching_walls: usize,
    #[serde(default)]
    pub error_m: Option<f64>,
    #[serde(default)]
    pub ambiguous: bool,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanObjectMatch {
    #[serde(default)]
    pub before_observation_id: String,
    #[serde(default)]
    pub after_observation_id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub distance_m: f64,
    #[serde(default)]
    pub suggestion: String,
    #[serde(default)]
    pub competing_candidates: usize,
    #[serde(default)]
    pub human_confirmation_required: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanReconciliation {
    #[serde(default)]
    pub alignment: RoomPlanAlignment,
    #[serde(default)]
    pub matches: Vec<RoomPlanObjectMatch>,
    #[serde(default)]
    pub unmatched_before: Vec<String>,
    #[serde(default)]
    pub unmatched_after: Vec<String>,
    #[serde(default)]
    pub human_confirmation_required: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct RoomPlanReconciliationResponse {
    #[serde(default)]
    pub before_revision_id: String,
    #[serde(default)]
    pub after_revision_id: String,
    #[serde(default)]
    pub reconciliation: RoomPlanReconciliation,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanElement {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanRoom {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub surfaces: Vec<RoomPlanElement>,
    #[serde(default)]
    pub openings: Vec<RoomPlanElement>,
    #[serde(default)]
    pub objects: Vec<RoomPlanElement>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanProvenance {
    #[serde(default)]
    pub capture_mode: Option<String>,
    #[serde(default)]
    pub parent_revision_id: Option<String>,
    #[serde(default)]
    pub capture_started_at: Option<String>,
    #[serde(default)]
    pub capture_finished_at: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanReview {
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
pub enum RoomPlanDecision {
    Accept,
    Reject,
}

impl RoomPlanDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanDraft {
    #[serde(default)]
    pub draft_id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub room: RoomPlanRoom,
    #[serde(default)]
    pub provenance: RoomPlanProvenance,
    #[serde(default)]
    pub review: Option<RoomPlanReview>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanRevisionSet {
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub revisions: Vec<RoomPlanDraft>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanLocalRecord {
    pub draft: RoomPlanDraft,
    #[serde(default)]
    pub sync_state: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanLiveUpdate {
    #[serde(default)]
    pub draft_id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub room: RoomPlanRoom,
    #[serde(default)]
    pub guidance: String,
    #[serde(default)]
    pub added_count: usize,
    #[serde(default)]
    pub changed_count: usize,
    #[serde(default)]
    pub removed_count: usize,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running { since_ms: u128 },
    Done { outcome: serde_json::Value },
    Failed { reason: String },
}

pub type RoomPlanJobState = JobState;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanJob {
    pub id: u64,
    pub state: RoomPlanJobState,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanJobReceipt {
    pub job: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanReconcileRequest {
    pub before_revision_id: String,
    pub after_revision_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanPlacementProposalRequest {
    pub space_id: String,
    pub revision_id: String,
    pub observation_id: String,
    pub item_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpaceCreateRequest {
    pub revision_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanSpaceRevisionRequest {
    pub revision_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanObjectLinkRequest {
    pub revision_id: String,
    pub observation_id: String,
    pub item_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanPlacementRequest {
    pub revision_id: String,
    pub observation_id: String,
    pub expected_revision: i64,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RoomPlanReviewRequest {
    pub decision: String,
    #[serde(default)]
    #[serde(alias = "notiz")]
    pub note: Option<String>,
}

fn raster_standard() -> i32 {
    20
}

fn search_limit() -> usize {
    6
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SearchRequest {
    pub layout: String,
    #[serde(default)]
    pub move_refs: Vec<String>,
    #[serde(default = "raster_standard")]
    pub step: i32,
    #[serde(default = "search_limit")]
    pub limit: usize,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            layout: String::new(),
            move_refs: Vec::new(),
            step: raster_standard(),
            limit: search_limit(),
        }
    }
}

fn compose_raster() -> i32 {
    25
}

fn compose_beam() -> usize {
    60
}

fn compose_limit() -> usize {
    5
}

fn compose_rotations() -> Vec<i32> {
    vec![0, 90]
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ComposeRequest {
    pub refs: Vec<String>,
    #[serde(default = "compose_raster")]
    pub step: i32,
    #[serde(default = "compose_beam")]
    pub beam: usize,
    #[serde(default = "compose_limit")]
    pub limit: usize,
    #[serde(default = "compose_rotations")]
    pub rotations: Vec<i32>,
}

impl Default for ComposeRequest {
    fn default() -> Self {
        Self {
            refs: Vec::new(),
            step: compose_raster(),
            beam: compose_beam(),
            limit: compose_limit(),
            rotations: compose_rotations(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_layout_ids() {
        assert!(valid_roomplan_layout_id("living-room"));
        assert!(valid_roomplan_layout_id("layout_1"));
        assert!(!valid_roomplan_layout_id("invalid id!"));
        assert!(!valid_roomplan_layout_id(""));
    }

    #[test]
    fn placement_serde_roundtrip() {
        let placement = Placement {
            item_id: "desk-1".into(),
            flat: "berlin".into(),
            x: 100,
            y: 200,
            rot: 90,
            revision: 1,
            since: "2026-10-25".into(),
        };
        let json = serde_json::to_string(&placement).unwrap();
        let parsed: Placement = serde_json::from_str(&json).unwrap();
        assert_eq!(placement, parsed);
    }
}

//! Room measurements and RoomPlan revisions owned by `capabilities/interior`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::device::DeviceClient;
use crate::shell::{Shell, ShellError, ShellResult};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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
    pub walls: std::collections::BTreeMap<String, RoomPlanWall>,
    #[serde(default)]
    pub openings: Vec<RoomPlanOpening>,
    #[serde(default)]
    pub fixed_furniture: Vec<RoomPlanFixture>,
    #[serde(default)]
    pub unmeasured: Vec<RoomPlanUncertainty>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanDimensions {
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub depth: i32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanWall {
    #[serde(default)]
    pub from: [i32; 2],
    #[serde(default)]
    pub to: [i32; 2],
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanFixture {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub x: [i32; 2],
    #[serde(default)]
    pub y: [i32; 2],
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanUncertainty {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanSavedLayoutPreview {
    #[serde(default)]
    pub layout: RoomPlanSavedLayout,
    #[serde(default)]
    pub check: serde_json::Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanSavedLayout {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "item", alias = "items")]
    pub items: Vec<RoomPlanSavedLayoutItem>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RoomPlanSavedLayoutItem {
    #[serde(default, rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
    #[serde(default)]
    pub size: Option<[i32; 2]>,
    #[serde(default)]
    pub kind: Option<String>,
}

pub(crate) fn valid_roomplan_layout_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanRevisionSet {
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub revisions: Vec<RoomPlanDraft>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanSpaceSet {
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub spaces: Vec<RoomPlanSpaceEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanSpaceEntry {
    #[serde(default)]
    pub space: RoomPlanSpace,
    #[serde(default)]
    pub object_links: Vec<RoomPlanObjectLink>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanPlacement {
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub flat: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub since: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanSpaceReceipt {
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub revision_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct RoomPlanReconciliationResponse {
    #[serde(default)]
    pub before_revision_id: String,
    #[serde(default)]
    pub after_revision_id: String,
    #[serde(default)]
    pub reconciliation: RoomPlanReconciliation,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RoomPlanLocalRecord {
    pub draft: RoomPlanDraft,
    #[serde(default)]
    pub sync_state: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
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
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
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
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RoomPlanElement {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
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
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// The latest on-device model and coverage hints while RoomPlan is still capturing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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

/// An asynchronous search or compose request owned by the Interior capability.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RoomPlanJob {
    pub id: u64,
    pub state: RoomPlanJobState,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RoomPlanJobState {
    Running { since_ms: u128 },
    Done { outcome: serde_json::Value },
    Failed { reason: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RoomPlanJobReceipt {
    pub job: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RoomPlanReview {
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomPlanDecision {
    Accept,
    Reject,
}

impl RoomPlanDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
        }
    }
}

#[derive(Clone)]
pub enum RoomPlanSource {
    Shell(Arc<Shell>),
    Device(Arc<DeviceClient>),
}

impl RoomPlanSource {
    pub async fn layouts(&self) -> ShellResult<Vec<crate::inventory::InteriorLayoutSummary>> {
        match self {
            Self::Shell(shell) => shell.interior_layouts().await,
            Self::Device(client) => client
                .get("/interior/api/layouts")
                .await
                .map_err(ShellError::Transport),
        }
    }

    pub async fn layout(&self, id: &str) -> ShellResult<RoomPlanSavedLayoutPreview> {
        if !valid_roomplan_layout_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not an interior layout id.".into()),
            ));
        }
        match self {
            Self::Shell(shell) => shell.interior_layout_preview(id).await,
            Self::Device(client) => client
                .get(&format!("/interior/api/layouts/{id}"))
                .await
                .map_err(ShellError::Transport),
        }
    }

    /// Checks a proposed arrangement without saving it, including access and sunlight context.
    pub async fn preview_placements(
        &self,
        items: &[serde_json::Value],
    ) -> ShellResult<serde_json::Value> {
        match self {
            Self::Shell(shell) => shell.interior_preview_placements(items).await,
            Self::Device(client) => {
                let body = serde_json::to_vec(&serde_json::json!({ "items": items }))
                    .map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/placements/preview", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    /// Searches positions for every item in a saved layout, returning a background job receipt.
    pub async fn search_layout(
        &self,
        layout: &str,
        move_refs: &[String],
    ) -> ShellResult<RoomPlanJobReceipt> {
        if !valid_roomplan_layout_id(layout) || move_refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose a saved layout with movable items.".into()),
            ));
        }
        match self {
            Self::Shell(shell) => shell.interior_search_layout(layout, move_refs).await,
            Self::Device(client) => {
                let body = serde_json::json!({ "layout": layout, "move_refs": move_refs, "step": 20, "limit": 6 });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/search", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    /// Composes arrangements from the explicitly selected inventory items.
    pub async fn compose_layout(&self, refs: &[String]) -> ShellResult<RoomPlanJobReceipt> {
        if refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose at least one item to arrange.".into()),
            ));
        }
        match self {
            Self::Shell(shell) => shell.interior_compose_layout(refs).await,
            Self::Device(client) => {
                let body = serde_json::json!({ "refs": refs, "step": 25, "beam": 60, "limit": 5, "rotations": [0, 90] });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/compose", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn planning_job(&self, id: u64) -> ShellResult<RoomPlanJob> {
        match self {
            Self::Shell(shell) => shell.interior_planning_job(id).await,
            Self::Device(client) => client
                .get(&format!("/interior/api/jobs/{id}"))
                .await
                .map_err(ShellError::Transport),
        }
    }

    /// Saves a reviewed algorithm result as a new named layout, leaving current placements intact.
    pub async fn save_proposal(
        &self,
        id: &str,
        name: &str,
        items: Vec<serde_json::Value>,
        note: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_roomplan_layout_id(id) || items.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A valid layout name and at least one placed item are required.".into()),
            ));
        }
        match self {
            Self::Shell(shell) => shell.interior_save_proposal(id, name, items, note).await,
            Self::Device(client) => {
                let body = serde_json::json!({ "id": id, "name": name, "items": items, "note_text": note });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/layouts", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn model(&self) -> ShellResult<RoomPlanModel> {
        match self {
            Self::Shell(shell) => shell.interior_model().await,
            Self::Device(client) => client
                .get("/interior/api/model")
                .await
                .map_err(ShellError::Transport),
        }
    }

    /// Reads all archived semantic scans from the interior capability.
    pub async fn revisions(&self) -> ShellResult<RoomPlanRevisionSet> {
        match self {
            Self::Shell(shell) => shell.interior_roomplan_revisions().await,
            Self::Device(client) => client
                .get("/interior/api/roomplan/revisions")
                .await
                .map_err(ShellError::Transport),
        }
    }

    /// Downloads the immutable parametric asset for one archived scan revision.
    pub async fn revision_asset(&self, revision_id: &str) -> ShellResult<Vec<u8>> {
        if !valid_roomplan_layout_id(revision_id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a RoomPlan revision id.".into()),
            ));
        }
        let path = format!("/interior/api/roomplan/revisions/{revision_id}/asset");
        match self {
            Self::Shell(shell) => shell.interior_roomplan_revision_asset(revision_id).await,
            Self::Device(client) => client.get_bytes(&path).await.map_err(ShellError::Transport),
        }
    }

    pub async fn spaces(&self) -> ShellResult<RoomPlanSpaceSet> {
        match self {
            Self::Shell(shell) => shell.interior_roomplan_spaces().await,
            Self::Device(client) => client
                .get("/interior/api/roomplan/spaces")
                .await
                .map_err(ShellError::Transport),
        }
    }

    pub async fn inventory_rows(&self) -> ShellResult<Vec<crate::inventory::InventoryRow>> {
        match self {
            Self::Shell(shell) => shell.inventory_rows().await,
            Self::Device(client) => client
                .get("/inventory/api/inventory")
                .await
                .map_err(ShellError::Transport),
        }
    }

    pub async fn placements(&self, flat: &str) -> ShellResult<Vec<RoomPlanPlacement>> {
        match self {
            Self::Shell(shell) => shell.interior_placements(flat).await,
            Self::Device(client) => client
                .get(&format!("/interior/api/placements/{flat}"))
                .await
                .map_err(ShellError::Transport),
        }
    }

    pub async fn apply_placement(
        &self,
        space_id: &str,
        item_id: &str,
        revision_id: &str,
        observation_id: &str,
        expected_revision: i64,
        x: i32,
        y: i32,
        rot: i32,
    ) -> ShellResult<serde_json::Value> {
        match self {
            Self::Shell(shell) => {
                shell
                    .interior_roomplan_apply_placement(
                        space_id,
                        item_id,
                        revision_id,
                        observation_id,
                        expected_revision,
                        x,
                        y,
                        rot,
                    )
                    .await
            }
            Self::Device(client) => {
                let body = serde_json::json!({
                    "revision_id": revision_id,
                    "observation_id": observation_id,
                    "expected_revision": expected_revision,
                    "x": x,
                    "y": y,
                    "rot": rot
                });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .put(
                        &format!("/interior/api/roomplan/spaces/{space_id}/placements/{item_id}"),
                        &body,
                    )
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn propose_placement(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<RoomPlanPlacementProposalResponse> {
        match self {
            Self::Shell(shell) => {
                shell
                    .interior_roomplan_propose_placement(
                        space_id,
                        revision_id,
                        observation_id,
                        item_id,
                    )
                    .await
            }
            Self::Device(client) => {
                let body = serde_json::json!({
                    "space_id": space_id,
                    "revision_id": revision_id,
                    "observation_id": observation_id,
                    "item_id": item_id
                });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/roomplan/propose-placement", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn create_space(&self, revision_id: &str) -> ShellResult<RoomPlanSpaceReceipt> {
        match self {
            Self::Shell(shell) => shell.interior_roomplan_create_space(revision_id).await,
            Self::Device(client) => {
                let body = serde_json::json!({ "revision_id": revision_id });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/roomplan/spaces", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn add_revision_to_space(
        &self,
        space_id: &str,
        revision_id: &str,
    ) -> ShellResult<serde_json::Value> {
        match self {
            Self::Shell(shell) => {
                shell
                    .interior_roomplan_add_revision(space_id, revision_id)
                    .await
            }
            Self::Device(client) => {
                let body = serde_json::json!({ "revision_id": revision_id });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post(
                        &format!("/interior/api/roomplan/spaces/{space_id}/revisions"),
                        &body,
                    )
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    pub async fn link_object(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<serde_json::Value> {
        match self {
            Self::Shell(shell) => {
                shell
                    .interior_roomplan_link_object(space_id, revision_id, observation_id, item_id)
                    .await
            }
            Self::Device(client) => {
                let body = serde_json::json!({
                    "revision_id": revision_id,
                    "observation_id": observation_id,
                    "item_id": item_id
                });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .put(
                        &format!("/interior/api/roomplan/spaces/{space_id}/links"),
                        &body,
                    )
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    /// Records an accept or reject decision for one semantic scan.
    pub async fn review(
        &self,
        revision_id: &str,
        decision: RoomPlanDecision,
    ) -> ShellResult<RoomPlanReview> {
        match self {
            Self::Shell(shell) => shell.interior_roomplan_review(revision_id, decision).await,
            Self::Device(client) => {
                let body = serde_json::json!({ "decision": decision.as_str() });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post(
                        &format!("/interior/api/roomplan/revisions/{revision_id}/review"),
                        &body,
                    )
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }

    /// Compares two archived scans. Matches are suggestions and never placements.
    pub async fn reconcile(
        &self,
        before_revision_id: &str,
        after_revision_id: &str,
    ) -> ShellResult<RoomPlanReconciliationResponse> {
        match self {
            Self::Shell(shell) => {
                shell
                    .interior_roomplan_reconcile(before_revision_id, after_revision_id)
                    .await
            }
            Self::Device(client) => {
                let body = serde_json::json!({
                    "before_revision_id": before_revision_id,
                    "after_revision_id": after_revision_id
                });
                let body = serde_json::to_vec(&body).map_err(|_| ShellError::Unreadable)?;
                client
                    .post("/interior/api/roomplan/reconcile", &body)
                    .await
                    .map_err(ShellError::Transport)
            }
        }
    }
}

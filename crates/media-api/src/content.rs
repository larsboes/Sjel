//! Local content evidence and explicit metadata review contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One uncalibrated classifier candidate, not an approved tag.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentLabel {
    pub identifier: String,
    pub score: f64,
}

/// A bounded preview of the same oriented frame supplied to the classifier.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ContentFrame {
    pub requested_seconds: Option<f64>,
    pub seconds: Option<f64>,
    pub labels: Vec<ContentLabel>,
    pub preview_png: String,
}

/// Durable review state; new evidence always starts pending.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentState {
    Pending,
    Accepted,
    Rejected,
}

/// Lightweight list row; private preview bytes are loaded only in the inspector.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct ContentProposal {
    pub id: i64,
    pub sha256: String,
    pub fingerprint: String,
    pub kind: String,
    pub engine: String,
    pub revision: u32,
    pub state: ContentState,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    pub created_at: String,
}

/// Cursor-bounded review queue.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct ContentPage {
    pub items: Vec<ContentProposal>,
    pub next_after: Option<i64>,
}

/// Immutable evidence plus the proposal's current decision and original filename provenance.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct ContentDetail {
    pub proposal: ContentProposal,
    pub os: String,
    pub frames: Vec<ContentFrame>,
    pub filenames: Vec<String>,
}

/// Explicit review action; accepting requires at least one chosen tag or collection.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentAction {
    Accept,
    Reject,
}

/// A decision binds to the exact evidence shown, not just an asset or classifier label.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentDecision {
    pub fingerprint: String,
    pub action: ContentAction,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
}

/// Stage results contain counts, never inferred labels or source paths.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ContentStageResult {
    pub inserted: usize,
    pub unchanged: usize,
    pub failed_items: usize,
}

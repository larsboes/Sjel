//! Typed API contract, wire models, and route constants for the Sjel import capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-import`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_IMPORT_PLAN: &str = "/api/import/plan";
pub const ROUTE_IMPORT_STATUS: &str = "/api/import/status";
pub const ROUTE_IMPORT_RUNS_CURRENT: &str = "/api/import/runs/current";
pub const ROUTE_IMPORT_RUNS: &str = "/api/import/runs";

// ============================================================================
// Core Models
// ============================================================================

/// `GET /api/import/plan`
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportPlan {
    pub folder: String,
    pub files: usize,
    pub routes: Vec<ImportRoute>,
    pub skipped: Vec<ImportSkip>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportRoute {
    /// `documents`, `media`, `finance`, `notes` or `skip`.
    pub route: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportSkip {
    pub reason: String,
    pub files: usize,
}

/// One import run as the page polls it (`GET /api/import/runs/current`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportJob {
    pub folder: String,
    pub total: usize,
    pub done: usize,
    pub unchanged: usize,
    /// Files per outcome: `imported`, `present`, `pending`, `skipped`, `failed`.
    #[serde(default)]
    pub outcomes: BTreeMap<String, usize>,
    pub finished: bool,
    #[serde(default)]
    pub error: Option<String>,
}

pub type Job = ImportJob;

/// `GET /api/import/status`: the ledger for one folder.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportStatus {
    pub folder: String,
    pub rows: Vec<ImportStatusRow>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ImportStatusRow {
    pub route: String,
    pub outcome: String,
    /// The reason for a skipped, pending or failed row.
    #[serde(default)]
    pub detail: Option<String>,
    pub files: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportRunRequest {
    pub folder: String,
}

pub type RunRequest = ImportRunRequest;

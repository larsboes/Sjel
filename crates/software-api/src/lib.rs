//! Typed API contract, wire models, and route constants for the Sjel software capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-software`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_APPS: &str = "/api/apps";
pub const ROUTE_APP_DECISION: &str = "/api/apps/{bundle_id}/decision";
pub const ROUTE_APP_UNINSTALL: &str = "/api/apps/{bundle_id}/uninstall";
pub const ROUTE_APPS_SCAN: &str = "/api/apps/scan";

// ============================================================================
// Models
// ============================================================================

/// What the operator decided about one app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// In use, keep it.
    Use,
    /// Not needed; uninstall it.
    Remove,
    /// Keep for now, but look for a better app.
    Replace,
    /// Keep for now, but build the part that matters into Sjel.
    Rebuild,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Use => "use",
            Self::Remove => "remove",
            Self::Replace => "replace",
            Self::Rebuild => "rebuild",
        }
    }
}

/// One installed app, leftover data directory, or CLI tool.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SoftwareApp {
    /// `app`, `leftover` or `tool`.
    #[serde(default)]
    pub kind: String,
    /// Who installed it: `app-store`, `cask:<token>`, `other`, or a tool's manager.
    #[serde(default)]
    pub source: String,
    pub bundle_id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub app_bytes: u64,
    #[serde(default)]
    pub data_bytes: u64,
    /// `YYYY-MM-DD HH:MM:SS +0000`, or none when Spotlight holds no date.
    #[serde(default)]
    pub last_used: Option<String>,
    #[serde(default)]
    pub present: bool,
    /// `use`, `remove`, `replace` or `rebuild`.
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Why the row is due another look, or none.
    #[serde(default)]
    pub review: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SoftwareApps {
    pub apps: Vec<SoftwareApp>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DecisionRequest {
    pub decision: Decision,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct UninstallRequest {}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ScanRequest {}

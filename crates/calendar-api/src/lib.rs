//! Typed API contract, wire models, and route constants for the Sjel calendar capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-calendar`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_ENTRIES: &str = "/api/entries";
pub const ROUTE_ENTRY_CHANGES: &str = "/api/entries/changes";
pub const ROUTE_ENTRY_BY_ID: &str = "/api/entries/{id}";
pub const ROUTE_ENTRY_EXTERNAL: &str = "/api/entries/external";
pub const ROUTE_PROPOSALS: &str = "/api/proposals";
pub const ROUTE_RHYTHMS: &str = "/api/rhythms";
pub const ROUTE_RHYTHM_BY_ID: &str = "/api/rhythms/{id}";
pub const ROUTE_RHYTHM_MATERIALIZE: &str = "/api/rhythms/{id}/materialize";
pub const ROUTE_VERDICTS: &str = "/api/verdicts";
pub const ROUTE_WINDOWS: &str = "/api/windows";
pub const ROUTE_TRIP_DRAFTS: &str = "/api/trip-drafts";
pub const ROUTE_TRIP_DRAFTS_MATERIALIZE: &str = "/api/trip-drafts/materialize";
pub const ROUTE_TRIP_PLANS_SYNC: &str = "/api/trip-plans/{plan_id}/sync";
pub const ROUTE_GOOGLE_DRAFTS: &str = "/api/google/drafts";
pub const ROUTE_GOOGLE_IMPORT: &str = "/api/google/import";
pub const ROUTE_GOOGLE_IMPORT_PREVIEW: &str = "/api/google/import-preview";
pub const ROUTE_GOOGLE_IMPORT_SELECTED: &str = "/api/google/import-selected";
pub const ROUTE_GOOGLE_EXPORT: &str = "/api/google/export";
pub const ROUTE_GOOGLE_EXPORTS: &str = "/api/google/exports";
pub const ROUTE_GOOGLE_ENTRY_EXPORT: &str = "/api/entries/{id}/google-export";
pub const ROUTE_MARKDOWN_SOURCES: &str = "/api/markdown/sources";
pub const ROUTE_MARKDOWN_PREVIEW: &str = "/api/markdown/preview";
pub const ROUTE_MARKDOWN_IMPORT: &str = "/api/markdown/import";
pub const ROUTE_LINKS: &str = "/api/links";

// ============================================================================
// Constants and Vocabularies
// ============================================================================

pub const KNOWN_KINDS: &[&str] = &[
    "busy",
    "work_onsite",
    "work_remote",
    "away",
    "event",
    "nightlife",
    "deadline",
    "travel_ok",
];

pub const KINDS: [(&str, &str, u32); 8] = [
    ("busy", "Busy", 0xe11d48),
    ("work_onsite", "Work (on site)", 0x2563eb),
    ("work_remote", "Work (remote)", 0x0891b2),
    ("away", "Away", 0xd97706),
    ("event", "Event", 0x7c3aed),
    ("nightlife", "Nightlife / Open Air", 0xdb2777),
    ("deadline", "Deadline", 0xb45309),
    ("travel_ok", "Travel possible", 0x16a34a),
];

pub const FALLBACK_COLOR: u32 = 0x64748b;

pub const COMMITMENTS: &[&str] = &["possible", "planned", "committed"];

pub fn default_source() -> String {
    "manual".into()
}

// ============================================================================
// Core Calendar Types
// ============================================================================

/// How binding an entry is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Default,
    Hash,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Commitment {
    #[default]
    Possible,
    Planned,
    Committed,
}

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Commitment::Possible => "possible",
            Commitment::Planned => "planned",
            Commitment::Committed => "committed",
        }
    }

    pub fn from_db(text: &str) -> Self {
        match text {
            "committed" => Commitment::Committed,
            "planned" => Commitment::Planned,
            _ => Commitment::Possible,
        }
    }
}

/// A calendar entry on the wire or in client state.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarEntry {
    pub title: String,
    /// `YYYY-MM-DD` when `all_day`, else local `YYYY-MM-DDTHH:MM:SS`.
    pub starts_at: String,
    /// Exclusive for an all-day entry.
    pub ends_at: String,
    pub all_day: bool,
    /// `possible`, `planned` or `committed`.
    pub commitment: String,
    #[serde(default)]
    pub id: String,
    /// Open text; `KINDS` names the known ones.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Who wrote it: `manual`, `google`, a capability.
    #[serde(default)]
    pub source: String,
}

/// An entry read by id, which includes its capability-owned payload for forward references.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct CalendarEntryDetail {
    #[serde(flatten)]
    pub entry: CalendarEntry,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// Input body to create a new calendar entry (`POST /api/entries`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct NewEntry {
    pub kind: String,
    #[serde(default)]
    pub commitment: Commitment,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub rhythm_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

// ============================================================================
// Trip Clustering & Materialization
// ============================================================================

/// A journey candidate recomputed from dated Calendar entries.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct TripDraft {
    pub place: String,
    pub starts_on: String,
    /// Exclusive end, like Calendar's own date windows.
    pub ends_before: String,
    pub entry_ids: Vec<String>,
    pub titles: Vec<String>,
    pub commitment: Commitment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unplanned_entry_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct UnclusteredTripEntry {
    pub entry_id: String,
    pub title: String,
    pub reason: String,
}

pub type Unclustered = UnclusteredTripEntry;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct TripDrafts {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub max_gap_days: i64,
    #[serde(default)]
    pub home: Option<String>,
    pub drafts: Vec<TripDraft>,
    pub unclustered: Vec<UnclusteredTripEntry>,
}

impl TripDrafts {
    pub fn new(drafts: Vec<TripDraft>, unclustered: Vec<UnclusteredTripEntry>) -> Self {
        Self {
            drafts,
            unclustered,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct TripMaterializeRequest {
    pub entry_ids: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct TripMaterialization {
    pub plan_id: String,
    pub created: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

// ============================================================================
// Google Export Models
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportOptIn {
    pub entry_id: String,
    pub google_calendar_id: String,
    pub google_event_id: Option<String>,
    pub pushed_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportOutcome {
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub google_event_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportSkipped {
    #[serde(default)]
    pub google_event_id: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportReport {
    #[serde(default)]
    pub calendar_id: String,
    #[serde(default)]
    pub home_timezone: String,
    #[serde(default)]
    pub opted_in: usize,
    #[serde(default)]
    pub inserted: usize,
    #[serde(default)]
    pub patched: usize,
    #[serde(default)]
    pub pushed: Vec<ExportOutcome>,
    #[serde(default)]
    pub skipped: Vec<ExportSkipped>,
    #[serde(default)]
    pub dry_run: bool,
}

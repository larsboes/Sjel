//! Pure wire models, quality review flags, content projections, and route constants for Sjel feed.
//!
//! Shared across `crates/comms`, `services/feed-sweep`, and `apps/gpui/core`. Zero platform dependencies (pure `serde`).

use serde::{Deserialize, Serialize};

// --- Route Constants ---

pub const ROUTE_COMMS_FEED: &str = "/comms/feed";
pub const ROUTE_COMMS_FEED_QUALITY: &str = "/comms/feed/quality";
pub const ROUTE_COMMS_FEED_QUALITY_REFRESH: &str = "/comms/feed/quality/refresh";
pub const ROUTE_COMMS_CONTENT_CLOUD_PROVIDERS: &str = "/comms/content/cloud-providers";

// --- Relevance & Evaluation ---

/// A feed entry's best profile match (`relevance` in `GET /comms/feed`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedRelevance {
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
}

/// Why the evaluator ranked an entry where it did.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedEvaluation {
    pub overall_score: f64,
    pub explanation: String,
}

// --- Feed Entries & Details ---

/// One row of `GET /comms/feed`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedEntry {
    pub id: String,
    pub stream: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    /// The digest's opening, sent only when the entry has no summary of its own.
    #[serde(default)]
    pub digest_preview: Option<String>,
    pub day: String,
    pub status: String,
    pub relevance: Option<FeedRelevance>,
    pub evaluation: Option<FeedEvaluation>,
}

impl FeedEntry {
    /// The order the dashboard ranks the inbox by: the evaluator's score, else the relevance score, else nothing.
    pub fn score(&self) -> f64 {
        self.evaluation
            .as_ref()
            .map(|e| e.overall_score)
            .or(self.relevance.as_ref().map(|r| r.score))
            .unwrap_or(0.0)
    }
}

/// `GET /comms/feed/:id`, comms' `FeedFullItem`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedDetail {
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub transcript: Option<String>,
    pub day: String,
    pub status: String,
    pub content_status: String,
    #[serde(default)]
    pub relevance: Vec<FeedRelevance>,
    pub evaluation: Option<FeedEvaluation>,
}

// --- Content Projections & Data Classes ---

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FeedDataClass {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub version: String,
}

pub fn data_class_rank(value: &str) -> u8 {
    match value {
        "c0" => 0,
        "c1" => 10,
        "c2" => 20,
        "c3" => 30,
        _ => 30,
    }
}

pub fn data_class_lowering_needs_rationale(current: &str, target: &str) -> bool {
    !current.is_empty() && data_class_rank(target) < data_class_rank(current)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CloudProviderOption {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider_label: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub data_tier: String,
    #[serde(default)]
    pub billing_mode: String,
    #[serde(default)]
    pub failover_priority: u16,
    #[serde(default)]
    pub max_requests_per_day: u32,
    #[serde(default)]
    pub requests_used_today: Option<u32>,
    #[serde(default)]
    pub requests_remaining_today: Option<u32>,
    #[serde(default)]
    pub max_input_tokens: u32,
    #[serde(default)]
    pub credit_expires_on: Option<String>,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedCloudProcessingState {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub preview_hash: Option<String>,
    #[serde(default)]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub dispatch_status: String,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub provider_role: Option<String>,
    #[serde(default)]
    pub queued_at: Option<String>,
    #[serde(default)]
    pub provider_calls: u8,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CloudRedactionFinding {
    #[serde(default)]
    pub entity_type: String,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CloudDerivativePreview {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub source_revision: String,
    #[serde(default)]
    pub preview_hash: String,
    #[serde(default)]
    pub original_data_class: String,
    #[serde(default)]
    pub derivative_data_class: String,
    #[serde(default)]
    pub transformation: String,
    #[serde(default)]
    pub document: String,
    #[serde(default)]
    pub redaction_count: usize,
    #[serde(default)]
    pub redactions: Vec<CloudRedactionFinding>,
    #[serde(default)]
    pub redaction_receipt: Option<String>,
    #[serde(default)]
    pub entity_detection: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub provider_calls: u8,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FeedDigest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FeedCalendarContent {
    #[serde(default)]
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub commitment: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub entry_source: String,
    #[serde(default)]
    pub rhythm_id: Option<String>,
}

/// The shared reader projection for a feed or mail item (`GET /comms/content/:source/:id`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FeedContentDetail {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_label: String,
    #[serde(default)]
    pub day: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub content_status: String,
    #[serde(default)]
    pub digest: Option<FeedDigest>,
    #[serde(default)]
    pub calendar: Option<FeedCalendarContent>,
    #[serde(default)]
    pub data_class: FeedDataClass,
    #[serde(default)]
    pub cloud_processing: FeedCloudProcessingState,
}

// --- Quality Flags & Refresh ---

/// One signal of `GET /comms/feed/quality`, comms' `QualityReviewRow`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FeedQualityFlag {
    pub feed_id: String,
    pub title: Option<String>,
    pub url: String,
    pub status: String,
    pub content_status: String,
    pub signal: String,
    pub reason: String,
    pub evidence: String,
    pub derived_at: String,
}

/// What `POST /comms/feed/quality/refresh` checked.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FeedQualityRefresh {
    pub reviewed: usize,
    pub flagged_items: usize,
    pub flag_count: usize,
    pub provider_calls: usize,
}

/// Where a keep, dismiss or open happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedSurface {
    Inbox,
    Reader,
    Library,
}

impl FeedSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Reader => "reader",
            Self::Library => "library",
        }
    }
}

/// Untagged response wrapper for feed queries.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum FeedList {
    Rows(Vec<FeedEntry>),
    Failed { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_score() {
        let mut entry = FeedEntry::default();
        assert_eq!(entry.score(), 0.0);

        entry.relevance = Some(FeedRelevance {
            profile_label: "test".into(),
            score: 0.75,
            rationale: "matches".into(),
        });
        assert_eq!(entry.score(), 0.75);

        entry.evaluation = Some(FeedEvaluation {
            overall_score: 0.9,
            explanation: "high quality".into(),
        });
        assert_eq!(entry.score(), 0.9);
    }

    #[test]
    fn test_data_class_ranks() {
        assert_eq!(data_class_rank("c0"), 0);
        assert_eq!(data_class_rank("c1"), 10);
        assert_eq!(data_class_rank("c2"), 20);
        assert_eq!(data_class_rank("c3"), 30);
        assert!(data_class_lowering_needs_rationale("c2", "c1"));
        assert!(!data_class_lowering_needs_rationale("c1", "c2"));
    }

    #[test]
    fn test_feed_surface_strings() {
        assert_eq!(FeedSurface::Inbox.as_str(), "inbox");
        assert_eq!(FeedSurface::Reader.as_str(), "reader");
        assert_eq!(FeedSurface::Library.as_str(), "library");
    }
}

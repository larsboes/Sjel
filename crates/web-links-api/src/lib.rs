//! Typed API contract, wire models, and route constants for the web-links capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-web-links`)
//! share these definitions to eliminate wire model drift. Zero platform dependencies.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_LINKS: &str = "/api/links";
pub const ROUTE_HISTORY_SCAN: &str = "/api/history/scan";
pub const ROUTE_HISTORY_CANDIDATES: &str = "/api/history/candidates";

pub const PROXY_ROUTE_WEB_LINKS: &str = "/web-links/api/links";
pub const PROXY_ROUTE_HISTORY_SCAN: &str = "/web-links/api/history/scan";
pub const PROXY_ROUTE_HISTORY_CANDIDATES: &str = "/web-links/api/history/candidates";

// ============================================================================
// Core Wire Models
// ============================================================================

/// A saved web destination, with its derived site group and any user override.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebLink {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    #[serde(default)]
    pub tenant: Option<String>,
    pub group: String,
    #[serde(default)]
    pub group_override: Option<String>,
    pub pinned: bool,
    #[serde(default)]
    pub opens: u64,
    #[serde(default)]
    pub last_opened_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

pub type Link = WebLink;

/// List wrapper for destinations.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebLinks {
    pub links: Vec<WebLink>,
}

pub type LinksReply = WebLinks;

/// One locally reviewed import candidate from browser history.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebLinkCandidate {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    #[serde(default)]
    pub tenant: Option<String>,
    pub browser: String,
    pub visits: u64,
    #[serde(default)]
    pub last_visit: Option<String>,
}

pub type Candidate = WebLinkCandidate;

/// Staged count and candidate list from a history scan.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebLinkHistoryReply {
    pub staged: usize,
    pub candidates: Vec<WebLinkCandidate>,
}

pub type HistoryReply = WebLinkHistoryReply;

// ============================================================================
// Mutation and Review Requests
// ============================================================================

/// Payload for adding a web link destination.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct WriteLink {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

/// User changes that do not replace the destination URL.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct EditLink {
    pub title: Option<String>,
    pub group: Option<String>,
    pub pinned: Option<bool>,
}

/// A decision on a staged browser-history suggestion.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryDecision {
    Keep,
    Dismiss,
}

impl HistoryDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Dismiss => "dismiss",
        }
    }
}

/// Parameters for scanning local browser history for link candidates.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct HistoryScan {
    #[serde(default = "default_days")]
    pub days: u16,
    #[serde(default = "default_visits")]
    pub min_visits: u16,
}

pub fn default_days() -> u16 {
    90
}

pub fn default_visits() -> u16 {
    3
}

impl Default for HistoryScan {
    fn default() -> Self {
        Self {
            days: default_days(),
            min_visits: default_visits(),
        }
    }
}

/// Request to keep or dismiss one staged candidate.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct HistoryDecisionRequest {
    pub decision: HistoryDecision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_link_roundtrip() {
        let link = WebLink {
            id: "abc".into(),
            url: "https://example.com".into(),
            title: "Example".into(),
            host: "example.com".into(),
            tenant: None,
            group: "Example".into(),
            group_override: None,
            pinned: true,
            opens: 5,
            last_opened_at: Some("2026-11-04T00:00:00Z".into()),
            created_at: "2026-11-04T00:00:00Z".into(),
        };
        let serialized = serde_json::to_string(&link).unwrap();
        let deserialized: WebLink = serde_json::from_str(&serialized).unwrap();
        assert_eq!(link, deserialized);
    }

    #[test]
    fn test_history_decision_serialization() {
        assert_eq!(
            serde_json::to_string(&HistoryDecision::Keep).unwrap(),
            "\"keep\""
        );
        assert_eq!(
            serde_json::to_string(&HistoryDecision::Dismiss).unwrap(),
            "\"dismiss\""
        );
        assert_eq!(HistoryDecision::Keep.as_str(), "keep");
    }

    #[test]
    fn test_history_scan_defaults() {
        let scan = HistoryScan::default();
        assert_eq!(scan.days, 90);
        assert_eq!(scan.min_visits, 3);
    }
}

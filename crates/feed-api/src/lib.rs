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

// --- Captures ---

pub const TAGS: [&str; 4] = ["buy", "look", "go", "learn"];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Capture {
    pub id: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub tag: Option<String>,
    pub status: String,
    pub routed_to: Option<String>,
    pub client: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureRoute {
    Wishlist,
    Feed,
    Places,
}

impl CaptureRoute {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wishlist => "To wishlist",
            Self::Feed => "To feed",
            Self::Places => "To places",
        }
    }
}

pub const FEED_HOSTS: [&str; 7] = [
    "github.com",
    "reddit.com",
    "arxiv.org",
    "huggingface.co",
    "youtube.com",
    "youtu.be",
    "instagram.com",
];

pub fn host(url: Option<&str>) -> Option<String> {
    let url = url?.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().map(|h| format!("{h}]"))?
    } else {
        host_port.split(':').next()?.to_ascii_lowercase()
    };
    if host.is_empty() || !host.contains('.') && host != "localhost" {
        return None;
    }
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

pub fn suggested_tag(tag: Option<&str>, url: Option<&str>) -> Option<&'static str> {
    if let Some(tag) = tag {
        return TAGS.iter().find(|known| **known == tag).copied();
    }
    let host = host(url)?;
    FEED_HOSTS
        .iter()
        .any(|f| host == *f || host.ends_with(&format!(".{f}")))
        .then_some("look")
}

pub fn route_for(tag: Option<&str>, url: Option<&str>) -> Option<CaptureRoute> {
    match suggested_tag(tag, url)? {
        "buy" => Some(CaptureRoute::Wishlist),
        "look" | "learn" if url.is_some() => Some(CaptureRoute::Feed),
        "go" => Some(CaptureRoute::Places),
        _ => None,
    }
}

impl Capture {
    pub fn suggested_tag(&self) -> Option<&str> {
        suggested_tag(self.tag.as_deref(), self.url.as_deref())
    }

    pub fn route(&self) -> Option<CaptureRoute> {
        route_for(self.tag.as_deref(), self.url.as_deref())
    }

    pub fn label(&self) -> String {
        self.text
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| host(self.url.as_deref()))
            .or_else(|| self.url.clone())
            .unwrap_or_default()
    }
}

pub fn cents_from(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    let mut t = lower.replace('€', "").replace("eur", "").trim().to_owned();
    if t.is_empty() {
        return None;
    }
    if t.contains('.') && t.contains(',') {
        t = t.replace('.', "");
    }
    let t = t.replacen(',', ".", 1);
    let (whole, frac) = t.split_once('.').unwrap_or((&t, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !(frac.is_empty() || (digits(frac) && frac.len() <= 2)) {
        return None;
    }
    let frac = format!("{frac:0<2}");
    Some(whole.parse::<i64>().ok()? * 100 + frac.parse::<i64>().ok()?)
}

pub fn extract_price(text: Option<&str>) -> (String, Option<i64>) {
    let trimmed = text.unwrap_or_default().trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() >= 2 {
        for take in [2, 1] {
            if let Some(cents) = cents_from(&words[words.len() - take..].join(" ")) {
                return (words[..words.len() - take].join(" "), Some(cents));
            }
        }
    }
    (trimmed.to_owned(), None)
}

pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let c = match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "wunsch".into()
    } else {
        out.into()
    }
}

// --- Mail Triage ---

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MailProposal {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub internal_date: Option<String>,
    pub stream: String,
    pub status: String,
    pub gmail_sync_status: Option<String>,
    pub gmail_sync_action: Option<String>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub relevance: Vec<FeedRelevance>,
    #[serde(default)]
    pub authenticity: Option<MailAuthenticity>,
    #[serde(default)]
    pub model: Option<ModelVerdict>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ModelVerdict {
    pub mode: String,
    pub state: String,
    pub model_stream: Option<String>,
    pub confidence_bp: Option<i64>,
    pub rationale: Option<String>,
    pub held_reason: Option<String>,
    pub classification_version: String,
}

impl ModelVerdict {
    pub fn proposes_change(&self, current: &str) -> Option<&str> {
        (self.state == "generated")
            .then_some(self.model_stream.as_deref())
            .flatten()
            .filter(|stream| *stream != current)
    }

    pub fn state_text(&self) -> &str {
        match self.state.as_str() {
            "local_refused" => {
                "Refused: a Secret mail never enters a prompt, local models included."
            }
            "skipped_over_window" => "Skipped: too long for the local model's window.",
            "unconfigured" => "No local model is configured for this rung on this machine.",
            "invalid_stream" => {
                "The model named a category that does not exist. Nothing was stored."
            }
            "unparseable" => {
                "The model's answer could not be read as JSON. It will be asked again."
            }
            "remote_refused" => "Refused: the resolved model was not on this machine.",
            other => other,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MailAuthenticity {
    pub verdict: String,
    #[serde(default)]
    pub assessments: Vec<AuthenticityAssessment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AuthenticityAssessment {
    pub source: String,
    pub verdict: String,
    #[serde(default)]
    pub findings: Vec<AuthenticityFinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AuthenticityFinding {
    pub code: String,
    pub verdict: String,
    pub detail: String,
}

impl MailProposal {
    pub fn authenticity_badge(&self) -> Option<String> {
        let auth = self.authenticity.as_ref()?;
        let label = match auth.verdict.as_str() {
            "failed" => "Failed check",
            "suspicious" => "Suspicious",
            "unverified" => "Unverified",
            _ => return None,
        };
        let reason = auth
            .assessments
            .iter()
            .flat_map(|a| &a.findings)
            .find(|f| f.verdict == auth.verdict)
            .map(|f| f.detail.as_str());
        Some(match reason {
            Some(reason) => format!("{label}: {reason}"),
            None => label.to_owned(),
        })
    }

    pub fn actionable(&self) -> bool {
        StatusFilter::Pending.matches(&self.status)
            && !matches!(
                self.gmail_sync_status.as_deref(),
                Some("queued" | "retrying")
            )
    }

    pub fn gmail_url(&self) -> String {
        format!("https://mail.google.com/mail/u/0/#all/{}", self.id)
    }

    fn top_score(&self) -> f64 {
        self.relevance
            .first()
            .map_or(f64::NEG_INFINITY, |r| r.score)
    }
}

pub const CATEGORIES: [(&str, &str); 7] = [
    ("issue", "Action"),
    ("aktiv", "Active"),
    ("steuern", "Tax"),
    ("belege", "Receipts"),
    ("feed", "Feed"),
    ("sonstiges", "Other"),
    ("werbung", "Advertising"),
];

pub const CLASS_RAISING: [&str; 2] = ["belege", "steuern"];

pub fn category_label(key: &str) -> &str {
    CATEGORIES
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, label)| label)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusFilter {
    Pending,
    Archived,
    Trashed,
    Dismissed,
    All,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 5] = [
        Self::Pending,
        Self::Archived,
        Self::Trashed,
        Self::Dismissed,
        Self::All,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Archived => "Archived",
            Self::Trashed => "Trash",
            Self::Dismissed => "Dismissed",
            Self::All => "All",
        }
    }

    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::Pending => matches!(status, "proposed" | "approved"),
            Self::Archived => status == "archived",
            Self::Trashed => status == "trashed",
            Self::Dismissed => status == "dismissed",
            Self::All => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailAction {
    Dismiss,
    Categorize(String),
    Archive,
    Trash,
    Waiting,
    ClearWaiting,
    SetDataClass(String),
}

impl MailAction {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Dismiss => "dismiss",
            Self::Categorize(_) => "categorize",
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Waiting => "waiting",
            Self::ClearWaiting => "clear-waiting",
            Self::SetDataClass(_) => "set-data-class",
        }
    }

    pub fn confirmation(&self, count: usize) -> Option<String> {
        let threads = if count == 1 {
            "1 thread".to_owned()
        } else {
            format!("{count} threads")
        };
        match self {
            Self::Archive => Some(format!("Archive {threads} in Sjel and Gmail?")),
            Self::Trash => Some(format!("Move {threads} to Gmail Trash?")),
            Self::SetDataClass(class) => Some(format!(
                "Set data class {} on {threads}? A raise redacts the stored subject and preview for good.",
                class.to_uppercase()
            )),
            Self::Categorize(key) if CLASS_RAISING.contains(&key.as_str()) => Some(format!(
                "Set {} on {threads}? The stored subject and preview are redacted for good.",
                category_label(key)
            )),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct BulkResult {
    pub succeeded: Vec<String>,
    #[serde(default)]
    pub failures: Vec<BulkFailure>,
    #[serde(default)]
    pub narrowed: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BulkFailure {
    pub id: String,
    pub error: String,
}

pub fn in_view<'a>(
    rows: impl Iterator<Item = &'a MailProposal>,
    status: StatusFilter,
    category: Option<&str>,
) -> Vec<&'a MailProposal> {
    let rank = |key: &str| {
        CATEGORIES
            .iter()
            .position(|(k, _)| *k == key)
            .unwrap_or(CATEGORIES.len())
    };
    let mut rows: Vec<&MailProposal> = rows
        .filter(|m| status.matches(&m.status))
        .filter(|m| category.is_none_or(|c| m.stream == c))
        .collect();
    rows.sort_by(|a, b| {
        rank(&a.stream)
            .cmp(&rank(&b.stream))
            .then(b.top_score().total_cmp(&a.top_score()))
            .then(b.internal_date.cmp(&a.internal_date))
    });
    rows
}

// --- Clipper ---

pub const BEHAVIORS: [(&str, &str); 6] = [
    ("create", "New clip"),
    ("overwrite", "Overwrite note"),
    ("append-specific", "Append to note"),
    ("prepend-specific", "Prepend to note"),
    ("append-daily", "Append to daily"),
    ("prepend-daily", "Prepend to daily"),
];

pub const FIELDS: [(&str, &str); 4] = [
    ("name", "Name"),
    ("path", "Path"),
    ("noteNameFormat", "Note name"),
    ("noteContentFormat", "Note content"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interpreter {
    pub available: bool,
    pub model: Option<String>,
    pub backend: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub revision: i64,
    pub templates: Vec<serde_json::Value>,
    pub interpreter: Interpreter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipDocument {
    pub key: String,
    pub title: String,
    pub url: String,
    pub versions: u64,
    pub updated_at: String,
    pub markdown: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipTurn {
    pub index: usize,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thoughts: Option<String>,
    pub response: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CitationStatus {
    Resolved,
    UnlinkedRef,
    MalformedUrl,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipCitation {
    pub marker: String,
    pub label: String,
    pub url: Option<String>,
    pub status: CitationStatus,
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

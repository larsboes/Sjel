//! Typed API contract, wire models, and route constants for the Sjel documents capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-documents`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_DOCUMENTS: &str = "/api/documents";
pub const ROUTE_DOCUMENT_BY_ID: &str = "/api/documents/{id}";
pub const ROUTE_DOCUMENT_ANALYZE: &str = "/api/documents/{id}/analyze";
pub const ROUTE_DOCUMENT_CHILDREN: &str = "/api/documents/{id}/children";
pub const ROUTE_DOCUMENT_ORIGINAL: &str = "/api/documents/{id}/original";
pub const ROUTE_DOCUMENT_DUPLICATES: &str = "/api/documents/duplicates";
pub const ROUTE_DOCUMENT_DUPLICATES_REVIEW: &str = "/api/documents/duplicates/review";
pub const ROUTE_DOCUMENT_SPLIT: &str = "/api/documents/{id}/split";
pub const ROUTE_DOCUMENT_PROPOSALS_REGENERATE: &str = "/api/documents/{id}/proposals/regenerate";
pub const ROUTE_DOCUMENT_PROPOSALS_MODEL: &str = "/api/documents/{id}/proposals/model";
pub const ROUTE_DOCUMENT_PROPOSAL_REVIEW: &str = "/api/documents/{id}/proposals/{proposal_id}/review";

// ============================================================================
// Core Document Models
// ============================================================================

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentPageRange {
    pub first: u32,
    pub last: u32,
}

pub type PageRange = DocumentPageRange;
pub type DuplicatePageRange = DocumentPageRange;

/// A page-range partition is empty, out of bounds, overlapping, or incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionError {
    /// The parent has no pages.
    EmptyParent,
    /// No child ranges were provided.
    NoRanges,
    /// A range starts at page zero or ends before it starts.
    InvalidRange,
    /// A range extends beyond the parent page count.
    OutOfBounds,
    /// Ranges overlap or leave one or more parent pages unassigned.
    NotExactPartition,
}

impl std::fmt::Display for PartitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::EmptyParent => "parent PDF must have at least one page",
            Self::NoRanges => "split must contain at least one child range",
            Self::InvalidRange => "page ranges must be one-based and non-empty",
            Self::OutOfBounds => "page range exceeds the parent page count",
            Self::NotExactPartition => "child ranges must cover every parent page exactly once",
        };
        f.write_str(message)
    }
}

impl std::error::Error for PartitionError {}

impl DocumentPageRange {
    /// Create an inclusive, one-based page range.
    ///
    /// # Errors
    /// The first page is zero or the last page precedes the first.
    pub fn new(first: u32, last: u32) -> Result<Self, PartitionError> {
        if first == 0 || last < first {
            return Err(PartitionError::InvalidRange);
        }
        Ok(Self { first, last })
    }
}

/// A complete partition of a parent PDF into ordered child page ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Partition {
    page_count: u32,
    ranges: Vec<DocumentPageRange>,
}

impl Partition {
    /// Validate child ranges as an ordered, gap-free, non-overlapping partition.
    ///
    /// Each range represents one child. The parent remains the source of the page
    /// count and is not modified by validation.
    ///
    /// # Errors
    /// The parent has no pages, no ranges were supplied, a range is invalid or
    /// out of bounds, or the ranges overlap or leave pages unassigned.
    pub fn new(page_count: u32, ranges: Vec<DocumentPageRange>) -> Result<Self, PartitionError> {
        if page_count == 0 {
            return Err(PartitionError::EmptyParent);
        }
        if ranges.is_empty() {
            return Err(PartitionError::NoRanges);
        }
        let mut next_page = 1_u64;
        for range in &ranges {
            if range.first == 0 || range.last < range.first {
                return Err(PartitionError::InvalidRange);
            }
            if range.last > page_count {
                return Err(PartitionError::OutOfBounds);
            }
            if u64::from(range.first) != next_page {
                return Err(PartitionError::NotExactPartition);
            }
            next_page = u64::from(range.last) + 1;
        }
        if next_page != u64::from(page_count) + 1 {
            return Err(PartitionError::NotExactPartition);
        }
        Ok(Self { page_count, ranges })
    }

    pub fn ranges(&self) -> &[DocumentPageRange] {
        &self.ranges
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }
}

/// Metadata row from the operator-only Documents API.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct DocumentSummary {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub byte_len: i64,
    pub imported_at: String,
    pub status: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub extracted_title: Option<String>,
    #[serde(default)]
    pub extractor: Option<String>,
    #[serde(default)]
    pub extracted_chars: usize,
    #[serde(default)]
    pub suggested_category: Option<String>,
    #[serde(default)]
    pub suggestion_reason: Option<String>,
    #[serde(default)]
    pub data_class: String,
    #[serde(default)]
    pub page_count: usize,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub page_range: Option<DocumentPageRange>,
    #[serde(default)]
    pub matched_pages: Vec<u32>,
    #[serde(default)]
    pub workflow_stage: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct DocumentExtractedPage {
    pub page_number: u32,
    pub text: String,
    pub engine: String,
    #[serde(default)]
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentBoundaryEvidence {
    pub before_page: u32,
    pub marker: String,
    pub quote: String,
}

pub type BoundaryEvidence = DocumentBoundaryEvidence;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentSplitSuggestion {
    pub page_count: u32,
    pub ranges: Vec<DocumentPageRange>,
    pub boundaries: Vec<DocumentBoundaryEvidence>,
    #[serde(default)]
    pub abstention: Option<String>,
}

pub type SplitSuggestion = DocumentSplitSuggestion;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentSplitChildInput {
    pub first: u32,
    pub last: u32,
    pub filename: String,
}

pub type ChildInput = DocumentSplitChildInput;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentSplitRequest {
    pub page_count: u32,
    pub children: Vec<DocumentSplitChildInput>,
}

pub type SplitRequest = DocumentSplitRequest;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default, schemars::JsonSchema)]
pub struct DocumentDetail {
    pub document: DocumentSummary,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub proposals: Vec<DocumentProposalRecord>,
    #[serde(default)]
    pub pages: Vec<DocumentExtractedPage>,
    #[serde(default)]
    pub children: Vec<DocumentSummary>,
    #[serde(default)]
    pub split_suggestions: Option<DocumentSplitSuggestion>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentUpload {
    pub document: DocumentSummary,
    pub created: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DocumentPage {
    pub documents: Vec<DocumentSummary>,
    pub total: usize,
}

// ============================================================================
// Classification & Proposals
// ============================================================================

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReview {
    Accepted,
    Rejected,
    Overridden,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalSource {
    Rules {
        identity: String,
        version: String,
    },
    Model {
        role: String,
        model: String,
        #[serde(default)]
        revision: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalEvidenceSource {
    Filename,
    ExtractedText,
}

pub type EvidenceSource = ProposalEvidenceSource;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProposalEvidence {
    pub source: ProposalEvidenceSource,
    pub quote: String,
}

pub type Evidence = ProposalEvidence;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAbstentionReason {
    NoMatch,
    MissingEvidence,
    ModelUnavailable,
    CategoryNotConfigured,
    SafetyRefused,
}

pub type AbstentionReason = ProposalAbstentionReason;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalInvalidReason {
    MalformedOutput,
    EvidenceNotExact,
    EvidenceTooLong,
    InvalidCategory,
}

pub type InvalidProposalReason = ProposalInvalidReason;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalOutcome {
    Proposed {
        category: String,
        evidence: ProposalEvidence,
    },
    Abstained {
        reason: ProposalAbstentionReason,
    },
    Invalid {
        reason: ProposalInvalidReason,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct ClassificationProposal {
    pub document_id: String,
    pub source: ProposalSource,
    pub outcome: ProposalOutcome,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub review: Option<ProposalReview>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProposalReviewRequest {
    pub outcome: ProposalReview,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProposalReviewEvent {
    pub id: i64,
    pub outcome: ProposalReview,
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DocumentProposalRecord {
    pub id: String,
    pub generation_id: String,
    pub generated_at: String,
    pub proposal: ClassificationProposal,
    #[serde(default)]
    pub review_history: Vec<ProposalReviewEvent>,
}

pub type StoredProposal = DocumentProposalRecord;

// ============================================================================
// Duplicates & Review
// ============================================================================

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateCandidateKind {
    ExactCopy,
    Rescan,
    Part,
}

pub type CandidateKind = DuplicateCandidateKind;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DuplicatePartRelation {
    pub part_id: String,
    pub parent_id: String,
}

pub type PartRelation = DuplicatePartRelation;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DuplicateSourceLocation {
    pub document_id: String,
    pub path: String,
}

pub type SourceLocation = DuplicateSourceLocation;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DuplicateCandidate {
    pub candidate_key: String,
    pub kind: DuplicateCandidateKind,
    pub member_ids: Vec<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub part: Option<DuplicatePartRelation>,
    #[serde(default)]
    pub locations: Vec<DuplicateSourceLocation>,
}

pub type ReviewCandidate = DuplicateCandidate;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
pub struct DuplicateCandidatePage {
    pub candidates: Vec<DuplicateCandidate>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DuplicateDecisionAction {
    ConfirmGroup {
        survivor_id: String,
    },
    Dismiss,
    Split {
        parent_id: String,
        page_count: u32,
        ranges: Vec<DocumentPageRange>,
    },
}

pub type DecisionAction = DuplicateDecisionAction;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DuplicateDecision {
    pub candidate_key: String,
    pub kind: DuplicateCandidateKind,
    pub member_ids: Vec<String>,
    pub action: DuplicateDecisionAction,
}

/// A partition or decision does not satisfy the candidate contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionError {
    /// Candidate key is empty or too long.
    InvalidCandidateKey,
    /// Member ids must be unique SHA-256 record identifiers.
    InvalidMembers,
    /// The selected survivor or split parent is not part of the candidate.
    SelectedRecordNotMember,
    /// A part candidate must be dismissed or promoted to a split, not confirmed as a group.
    ConfirmPartRequiresSplit,
    /// Splits can only resolve a part candidate.
    SplitRequiresPart,
    /// The page ranges do not exactly partition the parent.
    InvalidPartition(PartitionError),
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidCandidateKey => "duplicate candidate key is invalid",
            Self::InvalidMembers => "duplicate candidate members are invalid",
            Self::SelectedRecordNotMember => "selected document is not a candidate member",
            Self::ConfirmPartRequiresSplit => {
                "a part candidate must be dismissed or promoted to a split"
            }
            Self::SplitRequiresPart => "only a part candidate can become a split",
            Self::InvalidPartition(error) => return write!(f, "invalid split partition: {error}"),
        };
        f.write_str(message)
    }
}

impl std::error::Error for DecisionError {}

impl DuplicateDecision {
    /// Validate the decision's stable key, members, and outcome-specific fields.
    ///
    /// # Errors
    /// The candidate key or member ids are invalid, a selected record is not a member,
    /// or the split ranges do not form a strict partition.
    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.candidate_key.is_empty() || self.candidate_key.len() > 256 {
            return Err(DecisionError::InvalidCandidateKey);
        }
        if self.member_ids.is_empty()
            || self.member_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || self.member_ids.iter().any(|id| !is_digest(id))
        {
            return Err(DecisionError::InvalidMembers);
        }
        match &self.action {
            DuplicateDecisionAction::ConfirmGroup { survivor_id } => {
                if self.kind == DuplicateCandidateKind::Part {
                    return Err(DecisionError::ConfirmPartRequiresSplit);
                }
                if !self.member_ids.contains(survivor_id) {
                    return Err(DecisionError::SelectedRecordNotMember);
                }
            }
            DuplicateDecisionAction::Dismiss => {}
            DuplicateDecisionAction::Split {
                parent_id,
                page_count,
                ranges,
            } => {
                if self.kind != DuplicateCandidateKind::Part {
                    return Err(DecisionError::SplitRequiresPart);
                }
                if !self.member_ids.contains(parent_id) {
                    return Err(DecisionError::SelectedRecordNotMember);
                }
                Partition::new(*page_count, ranges.clone())
                    .map_err(DecisionError::InvalidPartition)?;
            }
        }
        Ok(())
    }
}

fn is_digest(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ReviewRequest {
    pub category: String,
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DuplicateDecisionResponse {
    pub decision: serde_json::Value,
}

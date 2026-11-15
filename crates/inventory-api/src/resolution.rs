//! Read-only identity resolution and lexical candidate retrieval.

use serde::{Deserialize, Serialize};

/// Exactly one selector: a canonical reference, untrusted title or reviewed scoped alias.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub reference: Option<String>,
    pub title: Option<String>,
    pub alias: Option<crate::aliases::AliasKey>,
}

/// Whether physical identity is established, unresolved with candidates, or unknown.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Exact,
    Ambiguous,
    Unknown,
}

/// Why a record was returned; lexical evidence never establishes physical identity.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateReason {
    Reference,
    TitleEqual,
    TokenOverlap,
    ReviewedItemAlias,
    ReviewedProductAlias,
}

/// A current review snapshot with an explainable retrieval score.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ResolutionCandidate {
    pub alias_id: Option<i64>,
    #[serde(flatten)]
    pub item: crate::MailMatchSnapshot,
    pub typed_ref: String,
    pub reason: CandidateReason,
    /// Token intersection divided by union in basis points; never a probability.
    pub overlap_bp: Option<u32>,
    pub shared_tokens: Vec<String>,
}

/// A bounded candidate list; `total_candidates` includes records beyond the returned limit.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Resolution {
    pub status: ResolutionStatus,
    pub candidates: Vec<ResolutionCandidate>,
    pub total_candidates: usize,
}

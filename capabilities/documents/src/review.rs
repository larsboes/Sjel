//! Persisted human decisions for duplicate-review candidates.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::split::{PageRange, Partition, PartitionError};

/// The evidence class that produced a duplicate-review candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// Original bytes have the same SHA-256 digest.
    ExactCopy,
    /// Different bytes have highly overlapping extracted text.
    Rescan,
    /// One extracted text appears to be contained in another.
    Part,
}

/// One displayable duplicate candidate, with only metadata and source locations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewCandidate {
    /// Stable key derived from candidate kind and its record ids or digest.
    pub candidate_key: String,
    /// Evidence class that produced the candidate.
    pub kind: CandidateKind,
    /// All document records in the candidate, sorted by id.
    pub member_ids: Vec<String>,
    /// Similarity score for advisory candidates; exact copies have no score.
    pub score: Option<f64>,
    /// Shared digest for byte-identical copies.
    pub sha256: Option<String>,
    /// The likely child and parent for an advisory part candidate.
    pub part: Option<PartRelation>,
    /// Recorded source paths for these records.
    pub locations: Vec<SourceLocation>,
}

/// Direction of an advisory part relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartRelation {
    /// Record whose text appears to be contained in the parent.
    pub part_id: String,
    /// Record whose extracted text appears to contain the part.
    pub parent_id: String,
}

/// One recorded filesystem location for a candidate member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    /// Document record that was found at this path.
    pub document_id: String,
    /// Local source path. Returned only through the authenticated operator API.
    pub path: String,
}

/// The operator's durable decision for one candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionAction {
    /// Keep one record as the group's preferred representative; retain all records and bytes.
    ConfirmGroup { survivor_id: String },
    /// Keep the candidate separate from all other records.
    Dismiss,
    /// Turn a part finding into stored child PDFs for page ranges of the named parent.
    Split {
        parent_id: String,
        page_count: u32,
        ranges: Vec<PageRange>,
    },
}

/// One stable review-candidate decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateDecision {
    /// Stable key derived from candidate kind and its record ids or digest.
    pub candidate_key: String,
    /// Evidence class that produced the candidate.
    pub kind: CandidateKind,
    /// All document records in the candidate, sorted by id.
    pub member_ids: Vec<String>,
    /// The human-reviewed outcome.
    pub action: DecisionAction,
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
            DecisionAction::ConfirmGroup { survivor_id } => {
                if self.kind == CandidateKind::Part {
                    return Err(DecisionError::ConfirmPartRequiresSplit);
                }
                if !self.member_ids.contains(survivor_id) {
                    return Err(DecisionError::SelectedRecordNotMember);
                }
            }
            DecisionAction::Dismiss => {}
            DecisionAction::Split {
                parent_id,
                page_count,
                ranges,
            } => {
                if self.kind != CandidateKind::Part {
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

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn decision(kind: CandidateKind, action: DecisionAction) -> DuplicateDecision {
        DuplicateDecision {
            candidate_key: "rescan:one:two".into(),
            kind,
            member_ids: vec![A.into(), B.into()],
            action,
        }
    }

    #[test]
    fn accepts_a_confirmed_group_with_a_member_survivor() {
        assert!(decision(
            CandidateKind::Rescan,
            DecisionAction::ConfirmGroup {
                survivor_id: A.into()
            }
        )
        .validate()
        .is_ok());
    }

    #[test]
    fn rejects_survivors_outside_candidate() {
        let error = decision(
            CandidateKind::Rescan,
            DecisionAction::ConfirmGroup {
                survivor_id: "c".repeat(64),
            },
        )
        .validate()
        .unwrap_err();
        assert_eq!(error, DecisionError::SelectedRecordNotMember);
    }

    #[test]
    fn accepts_a_part_promoted_to_a_complete_split() {
        assert!(decision(
            CandidateKind::Part,
            DecisionAction::Split {
                parent_id: B.into(),
                page_count: 4,
                ranges: vec![PageRange::new(1, 2).unwrap(), PageRange::new(3, 4).unwrap()],
            }
        )
        .validate()
        .is_ok());
    }

    #[test]
    fn rejects_incomplete_split_partitions_and_non_part_splits() {
        let incomplete = decision(
            CandidateKind::Part,
            DecisionAction::Split {
                parent_id: B.into(),
                page_count: 4,
                ranges: vec![PageRange::new(1, 2).unwrap()],
            },
        );
        assert!(matches!(
            incomplete.validate(),
            Err(DecisionError::InvalidPartition(_))
        ));

        let not_a_part = decision(
            CandidateKind::Rescan,
            DecisionAction::Split {
                parent_id: B.into(),
                page_count: 4,
                ranges: vec![PageRange::new(1, 4).unwrap()],
            },
        );
        assert_eq!(not_a_part.validate(), Err(DecisionError::SplitRequiresPart));
    }

    #[test]
    fn rejects_unsorted_or_repeated_member_ids() {
        let mut value = decision(CandidateKind::Rescan, DecisionAction::Dismiss);
        value.member_ids.reverse();
        assert_eq!(value.validate(), Err(DecisionError::InvalidMembers));
        value.member_ids = vec![A.into(), A.into()];
        assert_eq!(value.validate(), Err(DecisionError::InvalidMembers));
    }
}

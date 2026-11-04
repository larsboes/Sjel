//! Persisted human decisions for duplicate-review candidates.

pub use documents_api::{
    CandidateKind, DecisionAction, DecisionError, DuplicateCandidate, DuplicateDecision,
    PageRange, PartRelation, ReviewCandidate, SourceLocation,
};

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

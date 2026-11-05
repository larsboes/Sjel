//! The traveller profile: who is travelling, as data a solver reads.

pub use traveler_api::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unstated_profile_constrains_nothing_and_claims_no_weights() {
        let profile = TravelProfile::unstated();
        assert_eq!(profile.revision, 0);
        assert!(profile.hard.earliest_departure.is_none());
        assert!(profile.hard.latest_arrival.is_none());
        assert!(profile.hard.max_changes.is_none());
        assert!(profile.stated_weights().is_none());
    }

    #[test]
    fn a_stated_weight_makes_the_profile_the_source() {
        let mut profile = TravelProfile::unstated();
        profile
            .basis
            .insert("soft.budget_fit".into(), Provenance::Stated);
        assert!(profile.stated_weights().is_some());
    }

    #[test]
    fn every_declared_field_has_a_basis_entry_and_none_are_invented() {
        let input = ProfileInput::unstated();
        input.validate().unwrap();
        assert_eq!(input.basis.len(), BASIS_KEYS.len());
        for key in BASIS_KEYS {
            assert!(
                input.basis.contains_key(*key),
                "{key} has no provenance, so a reader cannot tell a decision from a default"
            );
        }
    }

    #[test]
    fn a_row_written_before_a_block_existed_is_readable_and_writable_again() {
        let mut profile = TravelProfile::unstated();
        profile.basis.remove("journey.price");
        profile.basis.remove("journey.reliability");
        assert!(profile.stated_journey_weights().is_none());

        let completed = profile.clone().with_complete_basis();
        assert_eq!(
            completed.basis.get("journey.price"),
            Some(&Provenance::Default),
            "an absent entry means nobody stated it"
        );

        let input = ProfileInput {
            hard: completed.hard,
            soft: completed.soft,
            journey: completed.journey,
            interests: completed.interests,
            pace: completed.pace,
            anchors: completed.anchors,
            basis: completed.basis,
        };
        input.validate().unwrap();
    }

    #[test]
    fn weights_must_sum_to_one() {
        let mut input = ProfileInput::unstated();
        input.soft.budget_fit = 0.50;
        let err = input.validate().unwrap_err();
        assert!(matches!(err, ProfileError::WeightSum { block: "soft", .. }));

        input = ProfileInput::unstated();
        input.journey.price = 0.50;
        let err = input.validate().unwrap_err();
        assert!(matches!(
            err,
            ProfileError::WeightSum {
                block: "journey",
                ..
            }
        ));
    }

    #[test]
    fn negative_and_nan_weights_are_refused() {
        let mut input = ProfileInput::unstated();
        input.soft.budget_fit = -0.10;
        input.soft.feasibility = 0.65;
        let err = input.validate().unwrap_err();
        assert!(matches!(
            err,
            ProfileError::NegativeWeight {
                block: "soft",
                key: "budget_fit",
                ..
            }
        ));

        input = ProfileInput::unstated();
        input.journey.price = f64::NAN;
        let err = input.validate().unwrap_err();
        assert!(matches!(
            err,
            ProfileError::NegativeWeight {
                block: "journey",
                key: "price",
                ..
            }
        ));
    }

    #[test]
    fn clocks_must_be_hh_mm() {
        let mut input = ProfileInput::unstated();
        input.hard.earliest_departure = Some("7:00".into());
        assert!(input.validate().is_err());
        input.hard.earliest_departure = Some("25:00".into());
        assert!(input.validate().is_err());
        input.hard.earliest_departure = Some("07:60".into());
        assert!(input.validate().is_err());
        input.hard.earliest_departure = Some("07:00".into());
        assert!(input.validate().is_ok());
    }

    #[test]
    fn basis_must_match_declared_keys_exactly() {
        let mut input = ProfileInput::unstated();
        input.basis.remove("pace");
        let err = input.validate().unwrap_err();
        assert!(matches!(err, ProfileError::MissingBasisKey(_)));

        input = ProfileInput::unstated();
        input
            .basis
            .insert("unknown.key".into(), Provenance::Default);
        let err = input.validate().unwrap_err();
        assert!(matches!(err, ProfileError::UnknownBasisKey(_)));
    }
}

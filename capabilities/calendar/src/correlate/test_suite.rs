use super::*;

#[cfg(test)]
use events::{comparable_title, normalize_instant};
#[cfg(test)]
use trips::{city_of, is_home, without_postal_code};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// A real entry in the calendar. Defaults to `Committed` because that is
    /// what every test written before the axis existed meant by "there is an
    /// away block on the 14th" — use `entry_at` to vary the commitment.
    fn entry(kind: &str, starts_at: &str, ends_at: &str) -> Entry {
        entry_at(kind, Commitment::Committed, starts_at, ends_at)
    }

    fn entry_at(kind: &str, commitment: Commitment, starts_at: &str, ends_at: &str) -> Entry {
        let all_day = !starts_at.contains('T');
        Entry {
            id: format!("cal:entry:{kind}-{starts_at}"),
            kind: kind.into(),
            commitment,
            title: format!("{kind} block"),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day,
            location: None,
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    fn candidate(starts_at: &str, ends_at: Option<&str>) -> Candidate {
        Candidate {
            id: "opp:munich-meetup".into(),
            starts_at: starts_at.into(),
            ends_at: ends_at.map(str::to_string),
        }
    }

    fn verdict(candidate: &Candidate, entries: &[Entry]) -> Feasibility {
        verdict_for(candidate, entries).unwrap().verdict
    }

    #[test]
    fn empty_calendar_is_free() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(verdict(&day, &[]), Feasibility::Free);
    }

    #[test]
    fn travel_ok_and_remote_work_stay_free_but_are_still_evidence() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let entries = [
            entry("travel_ok", "2026-08-10", "2026-08-20"),
            entry("work_remote", "2026-08-14T09:00", "2026-08-14T17:00"),
        ];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(
            result.evidence.len(),
            2,
            "a free verdict still explains the day"
        );
    }

    #[test]
    fn onsite_work_and_busy_need_a_travel_day() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(
            verdict(&day, &[entry("work_onsite", "2026-08-14", "2026-08-15")]),
            Feasibility::NeedsTravelDay
        );
        assert_eq!(
            verdict(
                &day,
                &[entry("busy", "2026-08-14T09:00", "2026-08-14T10:00")]
            ),
            Feasibility::NeedsTravelDay
        );
    }

    #[test]
    fn away_and_a_confirmed_event_conflict() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(
            verdict(&day, &[entry("away", "2026-08-12", "2026-08-16")]),
            Feasibility::Conflicts
        );
        assert_eq!(
            verdict(
                &day,
                &[entry("event", "2026-08-14T18:00", "2026-08-14T22:00")]
            ),
            Feasibility::Conflicts
        );
    }

    #[test]
    fn the_worst_overlapping_entry_wins_and_leads_the_evidence() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let entries = [
            entry("work_remote", "2026-08-14T08:00", "2026-08-14T09:00"),
            entry("away", "2026-08-14", "2026-08-15"),
            entry("busy", "2026-08-14T10:00", "2026-08-14T11:00"),
        ];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Conflicts);
        assert_eq!(result.evidence[0].kind, "away");
        assert_eq!(result.evidence.len(), 3);
    }

    #[test]
    fn an_unconfirmed_google_draft_explains_a_day_without_blocking_it() {
        // Phase E's whole safety property, at the layer that would enforce a
        // block: an all-day import from Google must not make the day
        // infeasible before the operator has looked at it.
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let mut draft = entry("draft", "2026-08-14", "2026-08-15");
        draft.source = "google".into();
        draft.external_id = Some("3q7l9v1c8m2p4t6y0x5z".into());

        let result = verdict_for(&day, &[draft.clone()]).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(result.evidence.len(), 1, "it still explains the day");

        // ...and the moment the operator confirms it into an `event`, it does.
        let mut confirmed = draft;
        confirmed.kind = "event".into();
        assert_eq!(verdict(&day, &[confirmed]), Feasibility::Conflicts);
    }

    #[test]
    fn an_unknown_kind_is_neutral_never_a_conflict() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        // Phase F's day-planning blocks land as kinds this layer never saw.
        let entries = [entry("deep_work", "2026-08-14", "2026-08-15")];
        let result = verdict_for(&day, &entries).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert_eq!(result.evidence[0].impact, Feasibility::Free);
        assert_eq!(
            impact("something_invented_in_2027", Commitment::Committed),
            Feasibility::Free
        );
    }

    /// The full 3 x 7 matrix, written out rather than derived, so that a
    /// change to either ceiling has to be argued cell by cell.
    #[test]
    fn impact_is_the_cheaper_of_the_two_ceilings() {
        use Commitment::{Committed, Planned, Possible};
        use Feasibility::{Conflicts, Free, NeedsTravelDay as Ntd};

        let cases: &[(&str, Feasibility, Feasibility, Feasibility)] = &[
            // kind            possible  planned  committed
            ("busy", Free, Ntd, Ntd),
            ("work_onsite", Free, Ntd, Ntd),
            ("work_remote", Free, Free, Free),
            ("away", Free, Ntd, Conflicts),
            ("event", Free, Ntd, Conflicts),
            ("deadline", Free, Free, Free),
            ("travel_ok", Free, Free, Free),
            ("something_invented_in_2027", Free, Free, Free),
        ];

        for (kind, possible, planned, committed) in cases {
            assert_eq!(impact(kind, Possible), *possible, "{kind} @ possible");
            assert_eq!(impact(kind, Planned), *planned, "{kind} @ planned");
            assert_eq!(impact(kind, Committed), *committed, "{kind} @ committed");
        }
    }

    /// The cell a commitment-first rule gets wrong: planning to be available
    /// is not a cost, so it must not raise the verdict.
    #[test]
    fn planning_to_be_available_costs_nothing() {
        assert_eq!(impact("travel_ok", Commitment::Planned), Feasibility::Free);
        assert_eq!(
            impact("work_remote", Commitment::Planned),
            Feasibility::Free
        );
    }

    /// The Atlanta regression. An event scouting merely found, on a day you
    /// are otherwise free, must leave the day free — it used to reach
    /// `Conflicts` through `kind = "event"` and delete the day from August.
    #[test]
    fn a_merely_discovered_event_never_blocks_a_day() {
        let day = candidate("2026-08-04", Some("2026-08-05"));
        let found = [entry_at(
            "event",
            Commitment::Possible,
            "2026-08-04T23:00:00",
            "2026-08-05T03:00:00",
        )];
        assert_eq!(verdict(&day, &found), Feasibility::Free);

        // ...and the same event, once he has actually registered, does block.
        let booked = [entry_at(
            "event",
            Commitment::Committed,
            "2026-08-04T23:00:00",
            "2026-08-05T03:00:00",
        )];
        assert_eq!(verdict(&day, &booked), Feasibility::Conflicts);
    }

    #[test]
    fn ends_are_exclusive_at_the_day_boundary() {
        let day = candidate("2026-08-15", Some("2026-08-16"));
        // Ends 2026-08-15, so it covers the 14th and stops.
        let entries = [entry("away", "2026-08-13", "2026-08-15")];
        assert_eq!(verdict(&day, &entries), Feasibility::Free);

        let starts_that_day = [entry("away", "2026-08-15", "2026-08-16")];
        assert_eq!(verdict(&day, &starts_that_day), Feasibility::Conflicts);
    }

    #[test]
    fn ends_are_exclusive_at_the_midnight_boundary_too() {
        // The case tuple comparison gets wrong: an all-day entry's start reads
        // as "no time", which sorts *before* an explicit T00:00 end.
        let all_day = candidate("2026-08-14", Some("2026-08-15"));
        let ends_at_midnight = [entry("away", "2026-08-13T22:00", "2026-08-14T00:00")];
        assert_eq!(verdict(&all_day, &ends_at_midnight), Feasibility::Free);

        let starts_at_midnight = [entry("away", "2026-08-14T00:00", "2026-08-14T01:00")];
        assert_eq!(
            verdict(&all_day, &starts_at_midnight),
            Feasibility::Conflicts
        );
    }

    #[test]
    fn all_day_entries_overlap_timed_candidates_and_the_reverse() {
        let evening = candidate("2026-08-14T18:00", Some("2026-08-14T22:00"));
        let all_day_block = [entry("work_onsite", "2026-08-14", "2026-08-15")];
        assert_eq!(
            verdict(&evening, &all_day_block),
            Feasibility::NeedsTravelDay
        );

        // A timed block that ends before the candidate starts does not.
        let morning_block = [entry("work_onsite", "2026-08-14T09:00", "2026-08-14T17:00")];
        assert_eq!(verdict(&evening, &morning_block), Feasibility::Free);

        // ...and the all-day candidate sees that same morning block.
        let whole_day = candidate("2026-08-14", Some("2026-08-15"));
        assert_eq!(
            verdict(&whole_day, &morning_block),
            Feasibility::NeedsTravelDay
        );
    }

    #[test]
    fn the_candidates_own_promoted_entry_never_conflicts_with_it() {
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let mut promoted = entry("event", "2026-08-14", "2026-08-15");
        promoted.source = "scouting".into();
        promoted.external_id = Some("opp:munich-meetup".into());

        let result = verdict_for(&day, &[promoted.clone()]).unwrap();
        assert_eq!(result.verdict, Feasibility::Free);
        assert!(result.already_in_calendar);
        assert!(result.evidence.is_empty());

        // A *different* confirmed event on the same day is still a conflict.
        let mut other = promoted;
        other.external_id = Some("opp:some-other-thing".into());
        let result = verdict_for(&day, &[other]).unwrap();
        assert_eq!(result.verdict, Feasibility::Conflicts);
        assert!(!result.already_in_calendar);
    }

    #[test]
    fn a_promoted_entry_is_recognised_by_its_opportunity_id_too() {
        // The seam between scouting's promotion and this layer, found live on
        // 2026-07-30. Promotion stores the *provider's* key in `external_id`
        // (bare `evt-…`) so an ICS import of the same event dedupes onto the
        // same row — but Feed's Discover view asks with the *opportunity* id
        // (`evt:luma:evt-…`). Matching only `external_id` made a saved
        // opportunity conflict with itself.
        let mut promoted = entry("event", "2026-08-14", "2026-08-15");
        promoted.source = "luma".into();
        promoted.external_id = Some("evt-E8mj424DVKBXFb4".into());
        promoted.payload = json!({ "opportunity_id": "evt:luma:evt-E8mj424DVKBXFb4" });

        for asked_as in ["evt-E8mj424DVKBXFb4", "evt:luma:evt-E8mj424DVKBXFb4"] {
            let mut c = candidate("2026-08-14", Some("2026-08-15"));
            c.id = asked_as.into();
            let result = verdict_for(&c, &[promoted.clone()]).unwrap();
            assert!(
                result.already_in_calendar,
                "asked as {asked_as}: the entry that IS this candidate was not recognised"
            );
            assert_eq!(result.verdict, Feasibility::Free);
        }

        // A different Luma event on the same day is still a real clash.
        let mut other = candidate("2026-08-14", Some("2026-08-15"));
        other.id = "evt:luma:evt-SOMETHING-ELSE".into();
        let result = verdict_for(&other, &[promoted]).unwrap();
        assert!(!result.already_in_calendar);
        assert_eq!(result.verdict, Feasibility::Conflicts);
    }

    #[test]
    fn provider_timestamps_are_read_as_local_wall_time() {
        // Luma, euro-hackathons and transit_fare all emit a different shape;
        // every one of them means 18:00 on the operator's own clock here.
        let expected = date::instant_minutes("2026-08-14T18:00").unwrap();
        for raw in [
            "2026-08-14T18:00:00.000Z",
            "2026-08-14T18:00:00+02:00",
            "2026-08-14T18:00:00-04:00",
            "2026-08-14T18:00:00",
            "2026-08-14T18:00",
        ] {
            let normalized = normalize_instant(raw).expect(raw);
            assert_eq!(
                date::instant_minutes(&normalized),
                Some(expected),
                "{raw} normalized to {normalized}, which is not 18:00 local"
            );
        }
        assert_eq!(
            normalize_instant("2026-08-14").as_deref(),
            Some("2026-08-14")
        );
        assert!(normalize_instant("2026-08-14 18:00").is_none());
        assert!(normalize_instant("next tuesday").is_none());
    }

    #[test]
    fn a_candidate_without_an_end_is_the_whole_start_day() {
        let evening = candidate("2026-08-14T18:00:00Z", None);
        let result = verdict_for(&evening, &[]).unwrap();
        assert_eq!(result.starts_at, "2026-08-14");
        assert_eq!(result.ends_at, "2026-08-15");

        // ...and so is one whose provider repeated the same instant.
        let degenerate = candidate("2026-08-14T18:00:00", Some("2026-08-14T18:00:00"));
        let result = verdict_for(&degenerate, &[]).unwrap();
        assert_eq!(result.starts_at, "2026-08-14");
        assert_eq!(result.ends_at, "2026-08-15");
    }

    #[test]
    fn a_broken_instant_is_loud() {
        assert!(verdict_for(&candidate("whenever", None), &[]).is_err());
        let corrupt = entry("busy", "2026-08-14", "not-a-date");
        let day = candidate("2026-08-14", Some("2026-08-15"));
        let error = verdict_for(&day, &[corrupt]).unwrap_err();
        assert!(error.contains("unreadable ends_at"), "{error}");
    }

    #[test]
    fn query_window_spans_every_candidate() {
        let candidates = [
            Candidate {
                id: "a".into(),
                starts_at: "2026-08-14".into(),
                ends_at: Some("2026-08-16".into()),
            },
            Candidate {
                id: "b".into(),
                starts_at: "2026-09-02T18:00:00Z".into(),
                ends_at: None,
            },
        ];
        let (from, to) = query_window(&candidates).unwrap().unwrap();
        assert_eq!(from, "2026-08-14");
        assert_eq!(to, "2026-09-04");
        assert_eq!(query_window(&[]).unwrap(), None);
    }

    #[test]
    fn a_batch_preserves_candidate_order_and_returns_all_three_verdicts() {
        let candidates = [
            Candidate {
                id: "free".into(),
                starts_at: "2026-08-10".into(),
                ends_at: None,
            },
            Candidate {
                id: "needs-travel-day".into(),
                starts_at: "2026-08-11".into(),
                ends_at: None,
            },
            Candidate {
                id: "conflicts".into(),
                starts_at: "2026-08-12".into(),
                ends_at: None,
            },
        ];
        let entries = [
            entry("work_remote", "2026-08-10", "2026-08-11"),
            entry("work_onsite", "2026-08-11", "2026-08-12"),
            entry("away", "2026-08-12", "2026-08-13"),
        ];

        let verdicts = verdicts_for(&candidates, &entries).unwrap();
        assert_eq!(
            verdicts
                .iter()
                .map(|verdict| (verdict.id.as_str(), verdict.verdict))
                .collect::<Vec<_>>(),
            [
                ("free", Feasibility::Free),
                ("needs-travel-day", Feasibility::NeedsTravelDay),
                ("conflicts", Feasibility::Conflicts),
            ]
        );
    }

    fn days(from: &str, to: &str) -> (i64, i64) {
        (
            date::parse_date(from).unwrap(),
            date::parse_date(to).unwrap(),
        )
    }

    #[test]
    fn windows_break_on_conflicts_and_carry_the_soft_cost() {
        let (from, to) = days("2026-08-10", "2026-08-17");
        let entries = [
            entry("away", "2026-08-12", "2026-08-14"),
            entry("work_onsite", "2026-08-11", "2026-08-12"),
        ];
        let windows = feasible_windows(from, to, &entries, 1).unwrap();
        assert_eq!(windows.len(), 2);

        assert_eq!(windows[0].starts_on, "2026-08-10");
        assert_eq!(windows[0].ends_before, "2026-08-12");
        assert_eq!(windows[0].days, ["2026-08-10", "2026-08-11"]);
        assert_eq!(windows[0].verdict, Feasibility::NeedsTravelDay);
        assert_eq!(windows[0].days_needing_travel_day, ["2026-08-11"]);

        // The away block ends 2026-08-14, exclusive, so the 14th is feasible.
        assert_eq!(windows[1].starts_on, "2026-08-14");
        assert_eq!(windows[1].ends_before, "2026-08-17");
        assert_eq!(windows[1].verdict, Feasibility::Free);
        assert!(windows[1].days_needing_travel_day.is_empty());
    }

    #[test]
    fn min_days_drops_runs_too_short_to_search() {
        let (from, to) = days("2026-08-10", "2026-08-20");
        let entries = [
            entry("away", "2026-08-11", "2026-08-15"),
            entry("away", "2026-08-16", "2026-08-18"),
        ];
        let all = feasible_windows(from, to, &entries, 1).unwrap();
        assert_eq!(all.len(), 3, "10th, 15th, and 18th-19th");

        let long_enough = feasible_windows(from, to, &entries, 2).unwrap();
        assert_eq!(long_enough.len(), 1);
        assert_eq!(long_enough[0].days, ["2026-08-18", "2026-08-19"]);
    }

    #[test]
    fn a_fully_blocked_span_yields_no_windows() {
        let (from, to) = days("2026-08-10", "2026-08-12");
        let entries = [entry("away", "2026-08-01", "2026-09-01")];
        assert!(feasible_windows(from, to, &entries, 1).unwrap().is_empty());
    }

    /// The bug this exists for, in its real shape: the operator adopted the
    /// Google import of a party, and the venue scraper keeps re-proposing the
    /// same night under its own key, so the month grid draws it twice.
    #[test]
    fn the_same_event_from_two_sources_is_recognised_across_keys() {
        let mut adopted = entry_at(
            "nightlife",
            Commitment::Planned,
            "2026-08-15T16:00:00",
            "2026-08-15T22:00:00",
        );
        adopted.title = "Bootshaus pres. BC173 (let\u{2019}s get loco)".into();
        adopted.source = "google".into();
        adopted.external_id = Some("3q7l9v1c8m2p4t6y0x5z".into());

        let mut proposal = entry_at(
            "nightlife",
            Commitment::Possible,
            "2026-08-15T16:00:00",
            "2026-08-15T23:00:00",
        );
        proposal.title = "Bootshaus pres BC173 (lets get loco)".into();
        proposal.source = "web".into();
        proposal.external_id = Some("bootshaus:15-8-26-bc173-lets-get-loco".into());

        assert!(
            is_same_event(&proposal, &adopted).unwrap(),
            "an apostrophe and a full stop are not two different parties"
        );
        assert!(
            without_already_adopted(vec![proposal], &[adopted])
                .unwrap()
                .is_empty(),
            "a proposal already in the calendar must not be proposed again"
        );
    }

    /// The negation, which is what stops this from swallowing real proposals.
    #[test]
    fn a_different_event_or_a_different_night_survives() {
        let adopted = entry("event", "2026-08-15T16:00", "2026-08-15T22:00");

        let mut same_night_other_thing = entry_at(
            "event",
            Commitment::Possible,
            "2026-08-15T16:00",
            "2026-08-15T22:00",
        );
        same_night_other_thing.title = "Something else entirely".into();
        assert!(!is_same_event(&same_night_other_thing, &adopted).unwrap());

        let mut same_thing_other_night = same_night_other_thing.clone();
        same_thing_other_night.title = adopted.title.clone();
        same_thing_other_night.starts_at = "2026-08-22T16:00".into();
        same_thing_other_night.ends_at = "2026-08-22T22:00".into();
        assert!(
            !is_same_event(&same_thing_other_night, &adopted).unwrap(),
            "a recurring night is a real second event"
        );

        let kept = without_already_adopted(
            vec![same_night_other_thing, same_thing_other_night],
            &[adopted],
        )
        .unwrap();
        assert_eq!(kept.len(), 2, "neither was already in the calendar");
    }

    /// An all-day import of a timed event still covers it — the shape Google
    /// produces for anything it treats as a whole-day commitment.
    #[test]
    fn an_all_day_twin_still_matches_the_timed_original() {
        let mut all_day = entry("event", "2026-08-15", "2026-08-16");
        all_day.title = "Beispiel Summit 2026".into();

        let mut timed = entry_at(
            "event",
            Commitment::Possible,
            "2026-08-15T09:00",
            "2026-08-15T18:00",
        );
        timed.title = "Beispiel Summit 2026".into();

        assert!(is_same_event(&timed, &all_day).unwrap());
    }

    #[test]
    fn titles_compare_past_case_diacritics_and_punctuation() {
        assert_eq!(
            comparable_title("Müllers Straße — Café!"),
            "mullers strasse cafe"
        );
        assert_eq!(
            comparable_title("Bootshaus pres. BC173 (let\u{2019}s get loco)"),
            comparable_title("bootshaus  pres BC173 lets get loco")
        );
        // The live 2026-08-04 miss: an apostrophe must vanish, not split a word.
        assert_eq!(comparable_title("let\u{2019}s"), comparable_title("lets"));
        assert_eq!(comparable_title("BC-173"), comparable_title("BC173"));
        // ...but whitespace still separates, so two words are never one.
        assert_ne!(
            comparable_title("Boots haus"),
            comparable_title("Bootshaus")
        );
        // ...and two events that merely share a word are not the same event.
        assert_ne!(
            comparable_title("Bootshaus BC173"),
            comparable_title("Bootshaus BC174")
        );
        // A title made only of punctuation cannot identify anything, so it
        // must never match another one — see the emptiness guard.
        assert_eq!(comparable_title("!!! ---"), "");
        let mut untitled = entry("event", "2026-08-15T16:00", "2026-08-15T22:00");
        untitled.title = "***".into();
        let mut other = untitled.clone();
        other.title = "###".into();
        assert!(!is_same_event(&untitled, &other).unwrap());
    }

    #[test]
    fn verdicts_serialize_as_the_contracts_wire_names() {
        let json = serde_json::to_string(&[
            Feasibility::Free,
            Feasibility::NeedsTravelDay,
            Feasibility::Conflicts,
        ])
        .unwrap();
        assert_eq!(json, r#"["free","needs-travel-day","conflicts"]"#);
    }
}

// ---------------------------------------------------------------------------
// Phase D: which entries belong to one journey
// ---------------------------------------------------------------------------

#[cfg(test)]
mod cluster_tests {
    use super::*;
    use serde_json::json;

    fn event(id: &str, place: &str, starts_at: &str, ends_at: &str, c: Commitment) -> Entry {
        Entry {
            id: id.into(),
            kind: "event".into(),
            commitment: c,
            title: format!("{place} {id}"),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day: !starts_at.contains('T'),
            location: None,
            notes: None,
            attendees: Vec::new(),
            source: "luma".into(),
            external_id: None,
            rhythm_id: None,
            payload: json!({ "city": place }),
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    /// The issue's own example: two events in Munich five days apart are one
    /// trip, not two.
    #[test]
    fn two_events_in_one_place_within_the_gap_are_one_trip() {
        let entries = [
            event(
                "a",
                "München",
                "2026-08-12T09:00:00",
                "2026-08-12T17:00:00",
                Commitment::Possible,
            ),
            event(
                "b",
                "München",
                "2026-08-17T18:00:00",
                "2026-08-17T21:00:00",
                Commitment::Possible,
            ),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts.len(), 1, "five days apart is still one journey");
        assert_eq!(out.drafts[0].place, "München");
        assert_eq!(out.drafts[0].starts_on, "2026-08-12");
        assert_eq!(out.drafts[0].ends_before, "2026-08-18");
        assert_eq!(out.drafts[0].entry_ids, vec!["a", "b"]);
    }

    /// ...and the negation, which is what makes the gap mean anything.
    #[test]
    fn a_wider_gap_leaves_two_single_events_for_calendar_review() {
        let entries = [
            event(
                "a",
                "München",
                "2026-08-12T09:00:00",
                "2026-08-12T17:00:00",
                Commitment::Possible,
            ),
            event(
                "b",
                "München",
                "2026-08-20T18:00:00",
                "2026-08-20T21:00:00",
                Commitment::Possible,
            ),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty(), "one event is not a trip");
        assert_eq!(
            out.unclustered.len(),
            2,
            "each isolated event stays a calendar proposal"
        );
        assert!(out
            .unclustered
            .iter()
            .all(|entry| entry.reason.contains("calendar event")));
    }

    #[test]
    fn different_places_never_merge_however_close_in_time() {
        let entries = [
            event(
                "a",
                "München",
                "2026-08-12T09:00:00",
                "2026-08-12T17:00:00",
                Commitment::Possible,
            ),
            event(
                "b",
                "Hamburg",
                "2026-08-13T09:00:00",
                "2026-08-13T17:00:00",
                Commitment::Possible,
            ),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 2);
    }

    /// 13 of 62 events in the last live Luma sweep had no usable place.
    #[test]
    fn an_event_without_a_place_is_reported_not_dropped() {
        let mut placeless = event(
            "x",
            "",
            "2026-08-12T09:00:00",
            "2026-08-12T17:00:00",
            Commitment::Possible,
        );
        placeless.payload = json!({});
        let out = cluster_trips(&[placeless], 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 1);
        assert!(out.unclustered[0].reason.contains("no place"));
    }

    /// A booked outward and return train nine days apart: the return arrives
    /// home, but leaves from the trip's city, so it closes that trip.
    #[test]
    fn a_return_ticket_home_closes_the_trip_it_leaves() {
        let leg = |id: &str, city: &str, from: &str, day: &str| {
            let mut entry = event(id, city, day, day, Commitment::Possible);
            entry.payload = json!({ "city": city, "from_city": from });
            entry
        };
        let entries = [
            leg("out", "Berlin", "Example City", "2026-10-30"),
            leg("back", "Example City", "Berlin", "2026-11-08"),
        ];
        let out = cluster_trips(&entries, 5, Some("Example City")).unwrap();
        assert_eq!(out.drafts.len(), 1, "{:?}", out.unclustered);
        assert_eq!(out.drafts[0].place, "Berlin");
        assert_eq!(out.drafts[0].starts_on, "2026-10-30");
        assert_eq!(out.drafts[0].ends_before, "2026-11-09");
    }

    #[test]
    fn a_null_string_location_is_no_place() {
        let mut entry = event("a", "x", "2026-08-12", "2026-08-12", Commitment::Possible);
        entry.payload = json!({});
        entry.location = Some("null".into());
        let out = cluster_trips(&[entry], 5, None).unwrap();
        assert!(out.unclustered[0].reason.contains("no place"));
    }

    #[test]
    fn home_is_not_a_trip_and_says_so() {
        let entries = [event(
            "a",
            "Example City",
            "2026-08-12T19:00:00",
            "2026-08-12T21:00:00",
            Commitment::Possible,
        )];
        let out = cluster_trips(&entries, 5, Some("example city")).unwrap();
        assert!(out.drafts.is_empty(), "matching is case-insensitive");
        assert!(out.unclustered[0].reason.contains("home"));
    }

    /// The live miss: two committed days at the operator's own employer, in the
    /// city they live in, proposed as a journey — because a venue line carries
    /// the city name and the home setting is that same city. The city here is an
    /// example; the deployment's own is in the overlay.
    #[test]
    fn a_venue_in_the_home_city_is_still_home() {
        let entries = [
            event(
                "a",
                "Nordwerk, Example City",
                "2026-09-15",
                "2026-09-16",
                Commitment::Committed,
            ),
            event(
                "b",
                "Nordwerk, Example City",
                "2026-09-16",
                "2026-09-17",
                Commitment::Committed,
            ),
        ];
        let out = cluster_trips(&entries, 5, Some("Example City")).unwrap();
        assert!(out.drafts.is_empty(), "your own city is not a journey");
        assert_eq!(out.unclustered.len(), 2);
        assert!(out.unclustered.iter().all(|e| e.reason.contains("home")));
    }

    /// The failure that would cost something: cancelling a real trip because
    /// the home city appears *inside* a venue name somewhere else.
    #[test]
    fn a_venue_merely_named_after_home_is_still_a_trip() {
        let away = "Moxy Köln/Bonn Flughafen, Kennedystrasse Flughafen 14, 51147 Köln";
        assert!(!is_home(away, "Bonn"), "that airport hotel is in Köln");
        assert!(
            is_home(away, "köln"),
            "and the postal code must not hide it"
        );

        let entries = [
            event(
                "a",
                away,
                "2026-08-15T16:00:00",
                "2026-08-15T22:00:00",
                Commitment::Possible,
            ),
            event(
                "b",
                away,
                "2026-08-16T16:00:00",
                "2026-08-16T22:00:00",
                Commitment::Possible,
            ),
        ];
        assert_eq!(
            cluster_trips(&entries, 5, Some("Bonn"))
                .unwrap()
                .drafts
                .len(),
            1
        );
    }

    #[test]
    fn only_a_leading_postal_code_is_dropped_never_a_house_number() {
        assert_eq!(without_postal_code(" 20097 Hamburg "), "Hamburg");
        assert_eq!(without_postal_code("Musterdeich 15"), "Musterdeich 15");
        assert_eq!(without_postal_code("Bonn"), "Bonn");
        // A street is not a city, whatever the home city is called.
        assert!(!is_home(
            "Beispiel Innovation Hub, Musterdeich 15, 20097 Hamburg",
            "Bonn"
        ));
        assert!(is_home(
            "Beispiel Innovation Hub, Musterdeich 15, 20097 Hamburg",
            "Hamburg"
        ));
        // An empty home means every place clusters, as documented.
        assert!(!is_home("Bonn", "   "));
    }

    /// One booked ticket makes the whole journey real: you are going anyway.
    #[test]
    fn a_draft_carries_the_strongest_commitment_in_it() {
        let entries = [
            event(
                "a",
                "München",
                "2026-08-12T09:00:00",
                "2026-08-12T17:00:00",
                Commitment::Possible,
            ),
            event(
                "b",
                "München",
                "2026-08-13T09:00:00",
                "2026-08-13T17:00:00",
                Commitment::Committed,
            ),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts[0].commitment, Commitment::Committed);
    }

    /// An all-day entry's stored end is exclusive, so it must not stretch the
    /// trip a day past where it actually ends.
    #[test]
    fn an_all_day_events_exclusive_end_does_not_lengthen_the_trip() {
        let entries = [
            event(
                "a",
                "München",
                "2026-08-12",
                "2026-08-13",
                Commitment::Possible,
            ),
            event(
                "b",
                "München",
                "2026-08-12T18:00:00",
                "2026-08-12T21:00:00",
                Commitment::Possible,
            ),
        ];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert_eq!(out.drafts[0].starts_on, "2026-08-12");
        assert_eq!(out.drafts[0].ends_before, "2026-08-13");
    }

    /// You travel to a city, not to a venue. Two Hamburg addresses that share
    /// nothing textually are one journey — the case the raw-string key missed,
    /// leaving both below the two-event floor and neither ever a trip.
    #[test]
    fn two_venues_in_one_city_are_one_trip() {
        let mut hub = event(
            "a",
            "",
            "2026-10-16T09:00:00",
            "2026-10-16T18:00:00",
            Commitment::Committed,
        );
        hub.payload = json!({});
        hub.location = Some("Beispiel Innovation Hub, Musterdeich 15, 20097 Hamburg".into());

        let mut elsewhere = event(
            "b",
            "",
            "2026-10-17T19:00:00",
            "2026-10-17T22:00:00",
            Commitment::Possible,
        );
        elsewhere.payload = json!({});
        elsewhere.location =
            Some("Elbphilharmonie, Platz der Deutschen Einheit 4, 20457 Hamburg".into());

        let out = cluster_trips(&[hub, elsewhere], 5, Some("Bonn")).unwrap();
        assert_eq!(out.drafts.len(), 1, "two venues, one Hamburg, one journey");
        assert_eq!(
            out.drafts[0].place, "Hamburg",
            "the draft is labelled by city"
        );
        assert_eq!(out.drafts[0].entry_ids, vec!["a", "b"]);
        assert!(out.unclustered.is_empty());
    }

    #[test]
    fn a_city_is_derived_from_whatever_shape_the_source_wrote() {
        assert_eq!(city_of("Nordwerk, Bonn"), "Bonn");
        assert_eq!(
            city_of("Beispiel Innovation Hub, Musterdeich 15, 20097 Hamburg"),
            "Hamburg"
        );
        assert_eq!(
            city_of("Moxy Köln/Bonn Flughafen, Kennedystrasse Flughafen 14, 51147 Köln"),
            "Köln"
        );
        assert_eq!(city_of("München"), "München");
        // A postal code identifies the city segment wherever it sits, so a
        // country tacked on the end cannot displace it.
        assert_eq!(
            city_of("Elbphilharmonie, 20457 Hamburg, Germany"),
            "Hamburg"
        );
        // Nothing usable in, the original back out rather than an empty key.
        assert_eq!(city_of("   "), "");
        assert_eq!(city_of(",,,"), ",,,");
    }

    /// Different cities must never merge, whatever their addresses look like.
    #[test]
    fn two_cities_stay_two_places() {
        let mut hamburg = event(
            "a",
            "",
            "2026-10-16T09:00:00",
            "2026-10-16T18:00:00",
            Commitment::Possible,
        );
        hamburg.payload = json!({});
        hamburg.location = Some("Musterdeich 15, 20097 Hamburg".into());

        let mut koeln = event(
            "b",
            "",
            "2026-10-17T09:00:00",
            "2026-10-17T18:00:00",
            Commitment::Possible,
        );
        koeln.payload = json!({});
        koeln.location = Some("Kennedystrasse 14, 51147 Köln".into());

        let out = cluster_trips(&[hamburg, koeln], 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 2);
        assert!(out
            .unclustered
            .iter()
            .any(|u| u.reason.contains("in Hamburg")));
        assert!(out.unclustered.iter().any(|u| u.reason.contains("in Köln")));
    }

    #[test]
    fn one_remote_event_is_a_calendar_proposal_not_a_trip() {
        let entries = [event(
            "a",
            "München",
            "2026-08-12T09:00:00",
            "2026-08-12T17:00:00",
            Commitment::Possible,
        )];
        let out = cluster_trips(&entries, 5, None).unwrap();
        assert!(out.drafts.is_empty());
        assert_eq!(out.unclustered.len(), 1);
        assert!(out.unclustered[0].reason.contains("calendar event"));
    }

    #[test]
    fn only_events_cluster_work_and_away_blocks_are_not_trips() {
        let mut work = event(
            "w",
            "München",
            "2026-08-12T09:00:00",
            "2026-08-12T17:00:00",
            Commitment::Committed,
        );
        work.kind = "work_onsite".into();
        let out = cluster_trips(&[work], 5, None).unwrap();
        assert!(out.drafts.is_empty() && out.unclustered.is_empty());
    }
}

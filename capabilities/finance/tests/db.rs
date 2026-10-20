//! Finance's database-backed suite, kept out of CI's hermetic run by the name every
//! test in it carries: `db_tests::`. That is the one selector the database-backed
//! modules in comms, places, scouting and transit are also named by, so CI names
//! a filter and never a list of capabilities — see `capabilities/scouting/src/store.rs`.
//! It was `postgres_tests` until PRD Q45 retired the server; a temp file per test is
//! the whole fixture now.
//!
//! An integration test rather than a `#[cfg(test)]` module because it links the library
//! from outside, the way a caller does. It was a Bazel-only target under
//! `src/postgres_test.rs` until 2026-08-25 — a path Cargo never compiled at all, so the
//! suite ran nowhere but `bazel test //:postgres_integration_tests` (PRD Q44).

mod db_tests {
    use finance::accounting::{Amount, JournalTransaction, Posting};
    use finance::analytics::{project, TransactionRow};
    use finance::import::{
        parse_csv, AmountSign, CandidateState, CsvDateFormat, CsvLocationColumns, CsvMapping,
        CsvRowPolicy,
    };
    use finance::investment::{
        Holding, HoldingsCoverage, Quantity, ReviewedHoldingsSnapshot, ReviewedHoldingsSource,
    };
    use finance::price::{FetchAttempt, FetchStatus, PriceObservation};
    use finance::store::{StoredDecisionEvent, StoredProposal};
    use finance::FinanceStore;

    /// A file per test, in a directory this process owns. It replaces the per-pid
    /// schema and the guard that dropped it: a temp file is neither shared with
    /// another run nor carried into a backup.
    fn store(suffix: &str) -> FinanceStore {
        let dir = std::env::temp_dir().join(format!("finance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        // The pid is recycled eventually, so the file starts empty.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        FinanceStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    fn synthetic_projection() -> Vec<TransactionRow> {
        let amount = |mantissa| Amount {
            commodity: "EUR".into(),
            mantissa,
            scale: 2,
        };
        let tags = std::collections::BTreeMap::from([
            ("sjel-purpose".into(), "trip".into()),
            ("sjel-trip-id".into(), "trip:synthetic".into()),
            ("sjel-shared-cents".into(), "600".into()),
        ]);
        project(
            &[JournalTransaction {
                index: 1,
                date: "2026-08-01".into(),
                description: "Synthetic market".into(),
                source_id: Some("synthetic-source".into()),
                tags,
                postings: vec![
                    Posting {
                        account: "expenses:food".into(),
                        amounts: vec![amount(400)],
                    },
                    Posting {
                        account: "assets:receivable:shared".into(),
                        amounts: vec![amount(600)],
                    },
                    Posting {
                        account: "assets:bank:checking".into(),
                        amounts: vec![amount(-1_000)],
                    },
                ],
            }],
            "EUR",
        )
    }

    #[test]
    fn an_empty_store_rebuilds_to_the_same_projection_every_time() {
        let rows = synthetic_projection();
        let store = store("rebuild");
        assert!(store.transaction_projection().unwrap().is_empty());
        store.replace_transaction_projection(&rows).unwrap();
        let first = store.transaction_projection().unwrap();
        store.replace_transaction_projection(&rows).unwrap();
        assert_eq!(store.transaction_projection().unwrap(), first);
        assert_eq!(first.len(), rows.len());
    }

    #[test]
    fn candidate_staging_is_idempotent_and_review_is_explicit() {
        let mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        };
        let first_export = parse_csv(
            b"Date;Amount;Description;Reference\n2026-08-01;-10.00;Synthetic market;one\n2026-08-02;-5.00;Synthetic service;two\n",
            &mapping,
        )
        .unwrap();
        let overlapping_export = parse_csv(
            b"Date;Amount;Description;Reference\n2026-08-02;-5.00;Synthetic service;two\n2026-08-03;20.00;Synthetic refund;three\n",
            &mapping,
        )
        .unwrap();
        let store = store("candidates");
        assert_eq!(
            store.stage_candidates(&first_export, "2026-08-08").unwrap(),
            (2, 0)
        );
        let mut remapped_export = first_export.clone();
        remapped_export[0].proposed_account = "expenses:groceries".into();
        remapped_export[0].confidence_basis_points = 9_000;
        assert_eq!(
            store
                .stage_candidates(&remapped_export, "2026-08-09")
                .unwrap(),
            (0, 2)
        );
        let remapped = store.candidate(&first_export[0].id).unwrap().unwrap();
        assert_eq!(remapped.proposed_account, "expenses:groceries");
        assert_eq!(remapped.confidence_basis_points, 9_000);
        assert_eq!(
            store
                .stage_candidates(&overlapping_export, "2026-08-08")
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            store.candidate(&first_export[0].id).unwrap().unwrap().state,
            CandidateState::Pending
        );
        store
            .review_candidate(
                &first_export[0].id,
                CandidateState::Rejected,
                "expenses:food",
                "2026-08-08",
            )
            .unwrap();
        assert_eq!(
            store.candidate(&first_export[0].id).unwrap().unwrap().state,
            CandidateState::Rejected
        );
    }

    #[test]
    fn location_fields_round_trip_and_a_location_less_reimport_never_erases_them() {
        let mut mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "liabilities:card:review".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: Some(CsvLocationColumns {
                street_column: Some("Adresse".into()),
                postal_code_column: Some("PLZ".into()),
                city_column: None,
                country_column: Some("Land".into()),
                mcc_column: None,
            }),
        };
        // The quoted street cell keeps the Amex two-line shape: street, then city.
        let csv = b"Date;Amount;Description;Reference;Adresse;PLZ;Land\n2026-08-01;-10.00;Synthetic market;one;\"Beispielstr. 1\nMusterstadt\";12345;Deutschland\n";
        let with_location = parse_csv(csv, &mapping).unwrap();
        let store = store("candidate_location");
        assert_eq!(
            store
                .stage_candidates(&with_location, "2026-08-25")
                .unwrap(),
            (1, 0)
        );
        let stored = store.candidate(&with_location[0].id).unwrap().unwrap();
        assert_eq!(stored, with_location[0]);
        assert_eq!(
            stored.location_street.as_deref(),
            Some("Beispielstr. 1\nMusterstadt")
        );

        // Re-staging through a mapping without location columns keeps the captured
        // values on the still-pending candidate.
        mapping.location_columns = None;
        let without_location = parse_csv(csv, &mapping).unwrap();
        assert_eq!(without_location[0].id, with_location[0].id);
        assert_eq!(
            store
                .stage_candidates(&without_location, "2026-08-25")
                .unwrap(),
            (0, 1)
        );
        assert_eq!(
            store.candidate(&with_location[0].id).unwrap().unwrap(),
            with_location[0]
        );
    }

    #[test]
    fn a_pending_candidate_gains_location_when_the_mapping_learns_the_columns() {
        let mut mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "liabilities:card:review".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        };
        let csv = b"Date;Amount;Description;Reference;Adresse;PLZ;Land\n2026-08-01;-10.00;Synthetic market;one;Beispielstr. 1;12345;Deutschland\n";
        let without_location = parse_csv(csv, &mapping).unwrap();
        let store = store("candidate_location_backfill");
        assert_eq!(
            store
                .stage_candidates(&without_location, "2026-08-25")
                .unwrap(),
            (1, 0)
        );

        mapping.location_columns = Some(CsvLocationColumns {
            street_column: Some("Adresse".into()),
            postal_code_column: Some("PLZ".into()),
            city_column: None,
            country_column: Some("Land".into()),
            mcc_column: None,
        });
        let with_location = parse_csv(csv, &mapping).unwrap();
        assert_eq!(with_location[0].id, without_location[0].id);
        assert_eq!(
            store
                .stage_candidates(&with_location, "2026-08-25")
                .unwrap(),
            (0, 1)
        );
        let stored = store.candidate(&without_location[0].id).unwrap().unwrap();
        assert_eq!(stored.location_street.as_deref(), Some("Beispielstr. 1"));
        assert_eq!(stored.location_postal_code.as_deref(), Some("12345"));
        assert_eq!(stored.location_city, None);
        assert_eq!(stored.location_country.as_deref(), Some("Deutschland"));
    }

    #[test]
    fn reference_less_overlap_preserves_multiplicity_without_reimporting_it() {
        let mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: None,
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "liabilities:card:review".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        };
        let first_export = parse_csv(
            b"Date;Amount;Description\n2026-08-01;-7.13;Synthetic market\n",
            &mapping,
        )
        .unwrap();
        let larger_export = parse_csv(
            b"Date;Amount;Description\n2026-08-01;-7.13;Synthetic market\n2026-08-01;-7.13;Synthetic market\n",
            &mapping,
        )
        .unwrap();
        assert_eq!(first_export[0].id, larger_export[0].id);
        assert_ne!(larger_export[0].id, larger_export[1].id);

        let store = store("candidate_multiplicity");
        assert_eq!(
            store.stage_candidates(&first_export, "2026-08-09").unwrap(),
            (1, 0)
        );
        assert_eq!(
            store
                .stage_candidates(&larger_export, "2026-08-09")
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            store
                .stage_candidates(&larger_export, "2026-08-09")
                .unwrap(),
            (0, 2)
        );
    }

    #[test]
    fn reconciled_transfer_pair_has_one_canonical_candidate() {
        let mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        };
        let mut bank = parse_csv(
            b"Date;Amount;Description;Reference\n2026-08-02;-12.34;Synthetic transfer;bank\n",
            &mapping,
        )
        .unwrap()
        .remove(0);
        bank.proposed_account = "liabilities:card:review".into();
        let mut card = bank.clone();
        card.id = "card-candidate".into();
        card.fingerprint = "card-fingerprint".into();
        card.booked_at = "2026-08-01".into();
        card.amount_cents = 1234;
        card.source_account = "liabilities:card:review".into();
        card.proposed_account = "assets:bank:checking".into();
        let store = store("transfer_pair");
        store
            .stage_candidates(&[bank.clone(), card.clone()], "2026-08-09")
            .unwrap();

        assert!(store
            .review_transfer_pair(&bank.id, &card.id, &bank.proposed_account, "2026-08-09")
            .unwrap());
        assert_eq!(
            store.candidate(&bank.id).unwrap().unwrap().state,
            CandidateState::Confirmed
        );
        assert_eq!(
            store.candidate(&card.id).unwrap().unwrap().state,
            CandidateState::Duplicate
        );
    }

    #[test]
    fn reviewed_holdings_replace_atomically_and_preserve_an_empty_review() {
        let store = store("holdings");
        assert_eq!(store.holding_projection().unwrap(), None);
        let snapshot = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic-snapshot".into(),
            reviewed_at: "2026-08-09".into(),
            coverage: HoldingsCoverage::Partial,
            holdings: vec![Holding {
                instrument: "ACME".into(),
                quantity: Quantity {
                    mantissa: 1250,
                    scale: 3,
                },
                latest_unit_price: Some(Quantity {
                    mantissa: 101234,
                    scale: 4,
                }),
                currency: "EUR".into(),
            }],
            sources: vec![ReviewedHoldingsSource {
                source_key: "synthetic-broker".into(),
                snapshot_id: "synthetic-source-snapshot".into(),
                reviewed_at: "2026-08-09".into(),
                coverage: HoldingsCoverage::Partial,
            }],
        };
        store.replace_holding_projection(&snapshot).unwrap();
        assert_eq!(store.holding_projection().unwrap(), Some(snapshot));

        let empty = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic-empty".into(),
            reviewed_at: "2026-08-10".into(),
            coverage: HoldingsCoverage::Complete,
            holdings: vec![],
            sources: vec![ReviewedHoldingsSource {
                source_key: "synthetic-broker".into(),
                snapshot_id: "synthetic-empty-source".into(),
                reviewed_at: "2026-08-10".into(),
                coverage: HoldingsCoverage::Complete,
            }],
        };
        store.replace_holding_projection(&empty).unwrap();
        assert_eq!(store.holding_projection().unwrap(), Some(empty));
        store.clear_holding_projection().unwrap();
        assert_eq!(store.holding_projection().unwrap(), None);
    }
    // ---------------------------------------------------------------------
    // Market observations and the decision ledger
    // ---------------------------------------------------------------------

    /// Two synthetic candidates, from the CSV shape the importer exists for.
    fn synthetic_candidates() -> Vec<finance::import::TransactionCandidate> {
        let mapping = CsvMapping {
            delimiter: ';',
            decimal_separator: '.',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: None,
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![CsvDateFormat::IsoYearMonthDay],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        };
        parse_csv(
            b"Date;Amount;Description;Reference\n2026-08-01;-10.00;Synthetic market;one\n2026-08-02;-5.00;Synthetic service;two\n",
            &mapping,
        )
        .unwrap()
    }

    fn observation(instrument: &str, day: &str, fetched_at: &str) -> PriceObservation {
        PriceObservation {
            instrument: instrument.into(),
            observed_on: day.into(),
            price: Quantity {
                mantissa: 12_842,
                scale: 2,
            },
            currency: "EUR".into(),
            source: "broker".into(),
            fetched_at: fetched_at.into(),
        }
    }

    fn proposal(id: &str, subject: &str) -> StoredProposal {
        StoredProposal {
            id: id.into(),
            kind: "rebalance".into(),
            subject: subject.into(),
            rung: "rule".into(),
            data_class: "c1".into(),
            data_class_rationale: "synthetic".into(),
            proposal_json: r#"{"kind":"rebalance"}"#.into(),
            evidence_json: r#"{"numbers":{}}"#.into(),
            model_revision: "finance-decisions-1".into(),
            proposed_at: "2026-09-05T08:00:00Z".into(),
        }
    }

    fn verdict(recorded_at: &str) -> StoredDecisionEvent {
        StoredDecisionEvent {
            event: "verdict".into(),
            verdict: Some("accepted".into()),
            note: "synthetic note".into(),
            outcome_json: None,
            recorded_at: recorded_at.into(),
        }
    }

    /// A re-fetch must be free. The UNIQUE tuple is what makes it so, and nothing
    /// UPDATEs a quote: the second write is refused, not applied.
    #[test]
    fn a_price_row_is_idempotent_on_re_fetch() {
        let store = store("prices-idempotent");
        let first = observation("SYN-A", "2026-09-04", "2026-09-04T10:00:00Z");
        assert!(store.append_market_price(&first).unwrap());
        // A second fetch of the same instrument-day from the same source, later.
        let again = observation("SYN-A", "2026-09-04", "2026-09-05T10:00:00Z");
        assert!(!store.append_market_price(&again).unwrap());
        assert_eq!(store.price_series("SYN-A").unwrap().len(), 1);
        // The stored row is the first one: an observation is never corrected.
        assert_eq!(store.price_series("SYN-A").unwrap()[0], first);
    }

    /// The exact-decimal rule reaches the new table: a price is (mantissa, scale)
    /// on the way in and the same pair on the way out.
    #[test]
    fn a_price_never_becomes_a_float_in_the_database() {
        let store = store("prices-exact");
        store
            .append_market_price(&observation("SYN-A", "2026-09-04", "2026-09-04T10:00:00Z"))
            .unwrap();
        let stored = &store.latest_prices().unwrap()[0];
        assert_eq!(stored.price.mantissa, 12_842);
        assert_eq!(stored.price.scale, 2);
    }

    /// Every registered provider name must survive a round trip through the
    /// column that deliberately carries no CHECK.
    #[test]
    fn every_registered_provider_name_round_trips() {
        let store = store("prices-providers");
        for (index, name) in finance::price::PROVIDERS.iter().enumerate() {
            let mut row = observation("SYN-A", "2026-09-04", "2026-09-04T10:00:00Z");
            row.source = (*name).into();
            row.instrument = format!("SYN-{index}");
            assert!(store.append_market_price(&row).unwrap(), "{name}");
        }
        let stored: Vec<String> = store
            .latest_prices()
            .unwrap()
            .into_iter()
            .map(|observation| observation.source)
            .collect();
        for name in finance::price::PROVIDERS {
            assert!(stored.iter().any(|source| source == name), "{name}");
        }
    }

    /// A refusal is a row. "Why is this instrument stale" must have an answer
    /// that is not somebody's memory.
    #[test]
    fn a_fetch_refusal_is_a_recorded_row() {
        let store = store("prices-refusal");
        store
            .record_fetch(&FetchAttempt {
                provider: "broker".into(),
                target: "SYN-A".into(),
                requested_on: "2026-09-05".into(),
                status: FetchStatus::Refused,
                detail: "instrument is not in the reviewed projection".into(),
                rows_written: 0,
                fetched_at: "2026-09-05T10:00:00Z".into(),
            })
            .unwrap();
        let fetches = store.recent_fetches(50).unwrap();
        assert_eq!(fetches.len(), 1);
        assert_eq!(fetches[0].status, FetchStatus::Refused);
        assert_eq!(fetches[0].rows_written, 0);
        assert!(store.latest_prices().unwrap().is_empty());
    }

    /// The freshness contract's whole body, and the case it exists to refuse.
    ///
    /// `capabilities/finance/service.toml` declares `freshness_stale_hours`, so
    /// doctor calls finance stale when this stops moving. A run in which every
    /// provider refused still WROTE rows to `finance_price_fetches`, and if
    /// those rows answered here, a machine whose price fetch has been failing
    /// nightly for a week would report data arriving every night.
    #[test]
    fn a_failed_fetch_does_not_answer_the_freshness_contract() {
        let store = store("prices-freshness");
        let attempt = |status, fetched_at: &str| FetchAttempt {
            provider: "yahoo".into(),
            target: "SYN-A".into(),
            requested_on: fetched_at[0..10].to_string(),
            status,
            detail: String::new(),
            rows_written: 0,
            fetched_at: fetched_at.to_string(),
        };

        assert_eq!(
            store.newest_price_arrival().unwrap(),
            None,
            "nothing has ever arrived, and that must read as never rather than as fresh"
        );

        // The idempotent night: the provider handed over observations the UNIQUE
        // tuple already held. Zero rows written, and data did arrive.
        store
            .record_fetch(&attempt(FetchStatus::Ok, "2026-09-05T03:00:00Z"))
            .unwrap();
        assert_eq!(
            store.newest_price_arrival().unwrap().as_deref(),
            Some("2026-09-05T03:00:00Z")
        );

        // Three nights of failure on top of it. Each is a row with a newer
        // stamp, and none of them is an arrival.
        for (status, at) in [
            (FetchStatus::Refused, "2026-09-06T03:00:00Z"),
            (FetchStatus::Error, "2026-09-07T03:00:00Z"),
            (FetchStatus::Empty, "2026-09-08T03:00:00Z"),
        ] {
            store.record_fetch(&attempt(status, at)).unwrap();
        }
        assert_eq!(store.recent_fetches(50).unwrap().len(), 4);
        assert_eq!(
            store.newest_price_arrival().unwrap().as_deref(),
            Some("2026-09-05T03:00:00Z"),
            "a refused, errored or empty attempt is not a delivery"
        );

        // And the stamp the handler serves is the stamp the row holds.
        store
            .record_fetch(&attempt(FetchStatus::Ok, "2026-09-09T03:00:00Z"))
            .unwrap();
        assert_eq!(
            store
                .newest_price_arrival()
                .unwrap()
                .as_deref()
                .and_then(finance::clock::epoch_seconds),
            Some(1_788_922_800)
        );
    }

    /// The store.rs candidate fix: a primary-key read must agree with the scan it
    /// replaced, and answer None for an id that does not exist.
    #[test]
    fn a_candidate_is_found_by_primary_key() {
        let store = store("candidate-by-key");
        let (created, _) = store
            .stage_candidates(&synthetic_candidates(), "2026-08-10")
            .unwrap();
        assert!(created > 0);
        let listed = store.list_candidates().unwrap();
        let known = &listed[0];
        assert_eq!(store.candidate(&known.id).unwrap().as_ref(), Some(known));
        assert_eq!(store.candidate("no-such-candidate").unwrap(), None);
    }

    /// The ledger cannot lose the date a call was made: a verdict appends a row
    /// and leaves the proposal byte-identical.
    #[test]
    fn a_verdict_is_an_appended_row_not_an_update() {
        let store = store("ledger-verdict");
        let row = proposal("rebalance:instrument:SYN-A:0011", "instrument:SYN-A");
        let outcome = store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:00:00Z")
            .unwrap();
        assert_eq!(outcome.proposed, 1);
        assert!(store
            .append_decision_event(&row.id, &verdict("2026-09-05T09:00:00Z"))
            .unwrap());
        let stored = store.decision(&row.id).unwrap().expect("the proposal");
        assert_eq!(stored.proposal, row, "the proposal row is unmodified");
        assert_eq!(stored.events.len(), 1);
        assert_eq!(stored.status(), "accepted");
        // The open filter no longer returns it, and `all` still does.
        assert!(store.decisions(Some("open")).unwrap().is_empty());
        assert_eq!(store.decisions(Some("all")).unwrap().len(), 1);
    }

    /// Second granularity is the choice this proves: a same-day corrected verdict
    /// is a second row, and only two inside one second collide -- which the store
    /// reports by name so the handler can answer 409 rather than 500.
    #[test]
    fn two_verdicts_in_one_second_are_refused_by_name() {
        let store = store("ledger-collision");
        let row = proposal("rebalance:instrument:SYN-B:0022", "instrument:SYN-B");
        store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:00:00Z")
            .unwrap();
        assert!(store
            .append_decision_event(&row.id, &verdict("2026-09-05T09:00:00Z"))
            .unwrap());
        assert!(
            !store
                .append_decision_event(&row.id, &verdict("2026-09-05T09:00:00Z"))
                .unwrap(),
            "two verdicts inside one second are refused"
        );
        assert!(
            store
                .append_decision_event(&row.id, &verdict("2026-09-05T09:00:01Z"))
                .unwrap(),
            "a second apart, both land"
        );
        assert_eq!(store.decision(&row.id).unwrap().unwrap().events.len(), 2);
    }

    /// The deterministic id doing its job: a re-run over an unchanged fixture
    /// writes nothing and reports it as unchanged.
    #[test]
    fn an_open_proposal_survives_a_rerun_that_changes_nothing() {
        let store = store("ledger-rerun");
        let row = proposal("rebalance:instrument:SYN-C:0033", "instrument:SYN-C");
        let first = store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:00:00Z")
            .unwrap();
        assert_eq!(
            (first.proposed, first.unchanged, first.superseded),
            (1, 0, 0)
        );
        let second = store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:01:00Z")
            .unwrap();
        assert_eq!(
            (second.proposed, second.unchanged, second.superseded),
            (0, 1, 0)
        );
        assert_eq!(store.decisions(Some("open")).unwrap().len(), 1);
    }

    /// Supersession without a mutable column: the old row is untouched and gains
    /// an event, and the new proposal is a new row.
    #[test]
    fn a_material_change_supersedes_rather_than_edits() {
        let store = store("ledger-supersede");
        let old = proposal("rebalance:instrument:SYN-D:0044", "instrument:SYN-D");
        store
            .reconcile_decisions(std::slice::from_ref(&old), "2026-09-05T08:00:00Z")
            .unwrap();
        let new = proposal("rebalance:instrument:SYN-D:0055", "instrument:SYN-D");
        let outcome = store
            .reconcile_decisions(std::slice::from_ref(&new), "2026-09-05T09:00:00Z")
            .unwrap();
        assert_eq!((outcome.proposed, outcome.superseded), (1, 1));
        let previous = store.decision(&old.id).unwrap().unwrap();
        assert_eq!(previous.proposal, old, "the superseded row is unmodified");
        assert_eq!(previous.status(), "superseded");
        assert_eq!(store.decisions(Some("open")).unwrap().len(), 1);
    }

    /// A drift that leaves its band and comes back. The proposal id is a hash
    /// over BUCKETED numbers, so the third run mints the id the first run wrote;
    /// with `INSERT OR IGNORE` alone the supersession from run two survived it
    /// and the proposal the engine is producing right now could never reach the
    /// inbox again, while the run reported `unchanged` and success.
    ///
    /// The repair is an appended `reinstated` event, not a deleted `superseded`
    /// one, so the fourth run below can supersede the same proposal again and the
    /// whole journey stays readable.
    #[test]
    fn a_proposal_a_later_run_produces_again_is_open_again() {
        let store = store("ledger-reopen");
        let first = proposal("rebalance:asset_class:bond:0111", "asset_class:bond");
        let moved = proposal("rebalance:asset_class:bond:0222", "asset_class:bond");
        store
            .reconcile_decisions(std::slice::from_ref(&first), "2026-09-05T08:00:00Z")
            .unwrap();
        let second = store
            .reconcile_decisions(std::slice::from_ref(&moved), "2026-09-05T09:00:00Z")
            .unwrap();
        assert_eq!((second.proposed, second.superseded), (1, 1));
        assert_eq!(
            store.decision(&first.id).unwrap().unwrap().status(),
            "superseded"
        );
        let third = store
            .reconcile_decisions(std::slice::from_ref(&first), "2026-09-05T10:00:00Z")
            .unwrap();
        assert_eq!(
            (
                third.proposed,
                third.unchanged,
                third.reopened,
                third.superseded
            ),
            (0, 0, 1, 1),
            "the re-minted proposal is reopened and the replacement is superseded"
        );
        let restored = store.decision(&first.id).unwrap().unwrap();
        assert_eq!(restored.status(), "open");
        assert_eq!(restored.proposal, first, "the row itself is unmodified");
        assert_eq!(
            restored
                .events
                .iter()
                .map(|event| event.event.as_str())
                .collect::<Vec<_>>(),
            ["superseded", "reinstated"],
            "both assertions stay on the record; the later one is the state"
        );
        let open = store.decisions(Some("open")).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].proposal.id, first.id);
        // And the reopened proposal can be superseded again, which a presence
        // test on the supersession event would have made impossible.
        let fourth = store
            .reconcile_decisions(std::slice::from_ref(&moved), "2026-09-05T11:00:00Z")
            .unwrap();
        assert_eq!((fourth.reopened, fourth.superseded), (1, 1));
        assert_eq!(
            store.decision(&first.id).unwrap().unwrap().status(),
            "superseded"
        );
    }

    /// A human's answer is not withdrawn by a rule. A proposal that was rejected
    /// and is produced again stays rejected: the verdict was given on these exact
    /// numbers, and re-asking would be the ledger forgetting.
    #[test]
    fn a_rerun_does_not_reopen_a_proposal_a_human_answered() {
        let store = store("ledger-reopen-verdict");
        let row = proposal("rebalance:instrument:SYN-G:0099", "instrument:SYN-G");
        store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:00:00Z")
            .unwrap();
        store
            .append_decision_event(&row.id, &verdict("2026-09-05T09:00:00Z"))
            .unwrap();
        let again = store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T10:00:00Z")
            .unwrap();
        assert_eq!((again.unchanged, again.reopened), (1, 0));
        assert_eq!(
            store.decision(&row.id).unwrap().unwrap().status(),
            "accepted"
        );
    }

    /// Two runs on two connections against one file. The guard is `BEGIN
    /// IMMEDIATE` in the store, which an in-process mutex could not give: the
    /// CLI is a second process.
    #[test]
    fn a_concurrent_recompute_supersedes_once() {
        let dir = std::env::temp_dir().join(format!("finance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger-concurrent.db");
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        let one = FinanceStore::open(&path).unwrap();
        let two = FinanceStore::open(&path).unwrap();
        let old = proposal("rebalance:instrument:SYN-E:0066", "instrument:SYN-E");
        one.reconcile_decisions(std::slice::from_ref(&old), "2026-09-05T08:00:00Z")
            .unwrap();
        let replacement = proposal("rebalance:instrument:SYN-E:0077", "instrument:SYN-E");
        let first = one
            .reconcile_decisions(std::slice::from_ref(&replacement), "2026-09-05T09:00:00Z")
            .unwrap();
        let second = two
            .reconcile_decisions(std::slice::from_ref(&replacement), "2026-09-05T09:00:01Z")
            .unwrap();
        assert_eq!(first.superseded + second.superseded, 1);
        let previous = one.decision(&old.id).unwrap().unwrap();
        assert_eq!(
            previous
                .events
                .iter()
                .filter(|event| event.event == "superseded")
                .count(),
            1
        );
    }

    /// Principle 8 on the write path: the copy exists at the moment the call is
    /// made, and re-rendering it is byte-identical.
    #[test]
    fn a_verdict_writes_the_month_file_in_the_same_call() {
        let store = store("ledger-export");
        let overlay = std::env::temp_dir().join(format!("finance-overlay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&overlay);
        let row = proposal("rebalance:instrument:SYN-F:0088", "instrument:SYN-F");
        store
            .reconcile_decisions(std::slice::from_ref(&row), "2026-09-05T08:00:00Z")
            .unwrap();
        store
            .append_decision_event(&row.id, &verdict("2026-09-05T09:00:00Z"))
            .unwrap();
        let written = finance::decision::export_month(&store, &overlay, "2026-09").unwrap();
        let body = std::fs::read_to_string(&written).unwrap();
        assert!(body.contains(&row.id));
        assert!(body.contains("accepted"));
        assert!(body.contains("synthetic note"));
        let again = finance::decision::export_month(&store, &overlay, "2026-09").unwrap();
        assert_eq!(std::fs::read_to_string(&again).unwrap(), body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&written).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the copy is owner-only");
        }
    }
    /// Nothing in this ledger is deleted, including a supersession.
    ///
    /// The earlier form of the repair removed the `superseded` row, which made
    /// the fact that a proposal had once left the inbox unrecoverable -- in the
    /// one table whose entire shape exists so that a decision's history cannot be
    /// overwritten. Both events survive and the later one is the state.
    #[test]
    fn a_reinstated_proposal_keeps_the_supersession_that_preceded_it() {
        let store = store("ledger-reinstate-history");
        let first = proposal("rebalance:asset_class:bond:0333", "asset_class:bond");
        let moved = proposal("rebalance:asset_class:bond:0444", "asset_class:bond");
        store
            .reconcile_decisions(std::slice::from_ref(&first), "2026-09-05T08:00:00Z")
            .unwrap();
        store
            .reconcile_decisions(std::slice::from_ref(&moved), "2026-09-05T09:00:00Z")
            .unwrap();
        store
            .reconcile_decisions(std::slice::from_ref(&first), "2026-09-05T10:00:00Z")
            .unwrap();

        let restored = store.decision(&first.id).unwrap().unwrap();
        assert_eq!(restored.status(), "open");
        let superseded: Vec<&StoredDecisionEvent> = restored
            .events
            .iter()
            .filter(|event| event.event == "superseded")
            .collect();
        assert_eq!(
            superseded.len(),
            1,
            "the supersession is still on the record"
        );
        assert_eq!(superseded[0].recorded_at, "2026-09-05T09:00:00Z");
        let reinstated: Vec<&StoredDecisionEvent> = restored
            .events
            .iter()
            .filter(|event| event.event == "reinstated")
            .collect();
        assert_eq!(reinstated.len(), 1);
        assert_eq!(reinstated[0].recorded_at, "2026-09-05T10:00:00Z");
        assert!(
            !reinstated[0].note.is_empty(),
            "an appended assertion says why it was made"
        );

        // A fourth run that produces nothing supersedes it again, and the pair
        // grows rather than flipping a single row back and forth.
        store
            .reconcile_decisions(&[], "2026-09-05T11:00:00Z")
            .unwrap();
        let gone = store.decision(&first.id).unwrap().unwrap();
        assert_eq!(gone.status(), "superseded");
        assert_eq!(
            gone.events
                .iter()
                .map(|event| event.event.as_str())
                .collect::<Vec<_>>(),
            ["superseded", "reinstated", "superseded"]
        );
    }

    /// The file the owner's machine already carries has the three-value CHECK,
    /// written by an earlier run of this same code, and `CREATE TABLE IF NOT
    /// EXISTS` never revisits an installed table. Opening a store must widen it
    /// in place, keep every row, and do nothing at all on the second pass.
    #[test]
    fn opening_a_store_widens_a_three_value_event_check_and_keeps_its_rows() {
        let dir = std::env::temp_dir().join(format!("finance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join("ledger-check-rebuild.db");
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }

        // The shape as it was shipped before `reinstated` existed, plus one row
        // under it and the index the batch declares.
        {
            let conn = rusqlite::Connection::open(&path).expect("a scratch database");
            conn.execute_batch(
                "CREATE TABLE finance_decisions (
                     id TEXT PRIMARY KEY,
                     kind TEXT NOT NULL,
                     subject TEXT NOT NULL,
                     rung TEXT NOT NULL,
                     data_class TEXT NOT NULL DEFAULT 'c1',
                     data_class_rationale TEXT NOT NULL DEFAULT '',
                     proposal_json TEXT NOT NULL,
                     evidence_json TEXT NOT NULL,
                     model_revision TEXT NOT NULL,
                     proposed_at TEXT NOT NULL
                 );
                 CREATE TABLE finance_decision_events (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     decision_id TEXT NOT NULL
                         REFERENCES finance_decisions(id) ON DELETE CASCADE,
                     event TEXT NOT NULL
                         CHECK (event IN ('verdict','outcome','superseded')),
                     verdict TEXT CHECK (verdict IN ('accepted','rejected')),
                     note TEXT NOT NULL DEFAULT '',
                     outcome_json TEXT,
                     recorded_at TEXT NOT NULL,
                     CHECK ((event = 'verdict') = (verdict IS NOT NULL)),
                     UNIQUE (decision_id, event, recorded_at)
                 );
                 CREATE INDEX idx_finance_decision_events_decision
                     ON finance_decision_events(decision_id, recorded_at);
                 INSERT INTO finance_decisions
                     (id, kind, subject, rung, proposal_json, evidence_json,
                      model_revision, proposed_at)
                 VALUES ('rebalance:asset_class:bond:0555', 'rebalance',
                         'asset_class:bond', 'rule', '{}', '{}', 'finance-decisions-1',
                         '2026-09-05T08:00:00Z');
                 INSERT INTO finance_decision_events
                     (decision_id, event, verdict, note, recorded_at)
                 VALUES ('rebalance:asset_class:bond:0555', 'superseded', NULL,
                         'written under the old shape', '2026-09-05T09:00:00Z');",
            )
            .expect("the pre-existing three-value shape");
            assert!(
                conn.execute(
                    "INSERT INTO finance_decision_events
                        (decision_id, event, recorded_at)
                     VALUES ('rebalance:asset_class:bond:0555', 'reinstated', '2026-09-05T10:00:00Z')",
                    [],
                )
                .is_err(),
                "the old CHECK refuses the fourth value, which is what forces the rebuild"
            );
        }

        let store = FinanceStore::open(&path).expect("the rebuild runs on open");
        assert!(store.ping().is_ok(), "the store works after the rebuild");

        let stored = store
            .decision("rebalance:asset_class:bond:0555")
            .unwrap()
            .expect("the proposal survived the rebuild");
        assert_eq!(stored.events.len(), 1, "the event row survived the rebuild");
        assert_eq!(stored.events[0].note, "written under the old shape");
        assert_eq!(stored.status(), "superseded");

        {
            let conn = rusqlite::Connection::open(&path).expect("the rebuilt database");
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master
                     WHERE type = 'table' AND name = 'finance_decision_events'",
                    [],
                    |row| row.get(0),
                )
                .expect("the installed shape");
            assert!(ddl.contains("'reinstated'"), "{ddl}");
            let index: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'index' AND tbl_name = 'finance_decision_events'
                       AND name = 'idx_finance_decision_events_decision'",
                    [],
                    |row| row.get(0),
                )
                .expect("the index count");
            assert_eq!(index, 1, "the index survived the rebuild");
            assert!(
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE name = 'finance_decision_events_reinstated'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
                    == 0,
                "the scratch table is gone"
            );
        }

        // A second open finds the widened CHECK and does nothing, which is what
        // makes this safe to leave in the migration for ever.
        let again = FinanceStore::open(&path).expect("a second pass");
        let stored = again
            .decision("rebalance:asset_class:bond:0555")
            .unwrap()
            .expect("still there");
        assert_eq!(stored.events.len(), 1, "the second pass rebuilt nothing");
    }
}

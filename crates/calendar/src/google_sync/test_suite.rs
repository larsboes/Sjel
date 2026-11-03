use super::*;

#[cfg(test)]
use auth::{read_env_key, SyncResult};
#[cfg(test)]
use import::review_window;
#[cfg(test)]
use transport::{encode_segment, fail_for};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::{EventTime, EventsPage};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A recorded `events.list` page — one timed event, one all-day event,
    /// one expanded recurring instance.
    const PAGE: &str = r#"{
      "items": [
        {
          "id": "3q7l9v1c8m2p4t6y0x5z",
          "status": "confirmed",
          "summary": "Team sync",
          "location": "Example City",
          "start": { "dateTime": "2026-08-14T10:00:00+02:00", "timeZone": "Europe/Berlin" },
          "end":   { "dateTime": "2026-08-14T11:00:00+02:00", "timeZone": "Europe/Berlin" }
        },
        {
          "id": "7h2k4n6q8s0u2w4y6a8c",
          "status": "confirmed",
          "summary": "Urlaub",
          "start": { "date": "2026-08-17" },
          "end":   { "date": "2026-08-22" }
        },
        {
          "id": "base9x7v5t3r1p_20260819T063000Z",
          "status": "confirmed",
          "summary": "Standup",
          "start": { "dateTime": "2026-08-19T08:30:00+02:00" },
          "end":   { "dateTime": "2026-08-19T08:45:00+02:00" }
        }
      ]
    }"#;

    struct FixtureApi {
        events: Vec<GoogleEvent>,
        pushes: RefCell<Vec<(String, Option<String>, Value)>>,
    }

    impl FixtureApi {
        fn new(events: Vec<GoogleEvent>) -> Self {
            Self {
                events,
                pushes: RefCell::new(Vec::new()),
            }
        }
    }

    impl CalendarApi for FixtureApi {
        fn list_events(
            &self,
            _: &str,
            _: &str,
            _: &str,
            max: usize,
        ) -> SyncResult<Vec<GoogleEvent>> {
            Ok(self.events.iter().take(max).cloned().collect())
        }
        fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String> {
            self.pushes
                .borrow_mut()
                .push((calendar_id.into(), None, body.clone()));
            Ok(format!("g-inserted-{}", self.pushes.borrow().len()))
        }
        fn patch_event(
            &self,
            calendar_id: &str,
            event_id: &str,
            body: &Value,
        ) -> SyncResult<String> {
            self.pushes.borrow_mut().push((
                calendar_id.into(),
                Some(event_id.into()),
                body.clone(),
            ));
            Ok(event_id.to_string())
        }
    }

    /// In-memory stand-in with the one invariant the real table enforces:
    /// `(source, external_id)` is unique, so an upsert replaces rather than
    /// appends. Without that the idempotency test would prove nothing.
    #[derive(Default)]
    struct FakeStore {
        by_external: RefCell<BTreeMap<String, Entry>>,
        next_id: RefCell<usize>,
        exports: RefCell<Vec<(ExportOptIn, Entry)>>,
        pushes_recorded: RefCell<Vec<(String, String)>>,
    }

    impl FakeStore {
        fn seed(&self, entry: Entry) {
            self.by_external
                .borrow_mut()
                .insert(entry.external_id.clone().unwrap(), entry);
        }
        fn count(&self) -> usize {
            self.by_external.borrow().len()
        }
        fn get(&self, external_id: &str) -> Entry {
            self.by_external.borrow().get(external_id).cloned().unwrap()
        }
    }

    impl ImportStore for FakeStore {
        fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>> {
            Ok(self.by_external.borrow().get(external_id).cloned())
        }
        fn upsert(&self, input: &NewEntry) -> SyncResult<Entry> {
            let external_id = input.external_id.clone().unwrap();
            let mut map = self.by_external.borrow_mut();
            let id = match map.get(&external_id) {
                Some(existing) => existing.id.clone(),
                None => {
                    *self.next_id.borrow_mut() += 1;
                    format!("cal:entry:{}", self.next_id.borrow())
                }
            };
            let entry = Entry {
                id,
                kind: input.kind.clone(),
                commitment: input.commitment,
                title: input.title.clone(),
                starts_at: input.starts_at.clone(),
                ends_at: input.ends_at.clone(),
                all_day: input.all_day,
                location: input.location.clone(),
                notes: input.notes.clone(),
                attendees: input.attendees.clone(),
                source: input.source.clone(),
                external_id: Some(external_id.clone()),
                rhythm_id: None,
                payload: input.payload.clone(),
                created_at: "0".into(),
                updated_at: "1".into(),
            };
            map.insert(external_id, entry.clone());
            Ok(entry)
        }
        fn delete(&self, entry_id: &str) -> SyncResult<bool> {
            let mut map = self.by_external.borrow_mut();
            let key = map
                .iter()
                .find(|(_, entry)| entry.id == entry_id)
                .map(|(key, _)| key.clone());
            Ok(key.map(|key| map.remove(&key)).is_some())
        }
    }

    impl ExportStore for FakeStore {
        fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>> {
            Ok(self.exports.borrow().clone())
        }
        fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()> {
            self.pushes_recorded
                .borrow_mut()
                .push((entry_id.into(), google_event_id.into()));
            Ok(())
        }
    }

    fn settings() -> Settings {
        Settings {
            tz: HomeTimezone::parse("Europe/Berlin").unwrap(),
            calendar_id: "primary".into(),
            google: GoogleConfig::default(),
        }
    }

    fn fixture_events() -> Vec<GoogleEvent> {
        serde_json::from_str::<EventsPage>(PAGE).unwrap().items
    }

    #[test]
    fn importing_twice_produces_one_entry_per_event() {
        let store = FakeStore::default();
        let api = FixtureApi::new(fixture_events());

        let first = import(&store, &api, &settings(), false).unwrap();
        assert_eq!(first.fetched, 3);
        assert_eq!(first.created, 3);
        assert_eq!(store.count(), 3);

        let second = import(&store, &api, &settings(), false).unwrap();
        assert_eq!(second.created, 0, "nothing new on a repeat run");
        assert_eq!(second.refreshed, 0, "and nothing rewritten either");
        assert_eq!(second.unchanged, 3);
        assert_eq!(store.count(), 3, "still one entry per Google event");
    }

    #[test]
    fn every_imported_event_arrives_as_a_neutral_draft() {
        let store = FakeStore::default();
        import(
            &store,
            &FixtureApi::new(fixture_events()),
            &settings(),
            false,
        )
        .unwrap();
        for entry in store.by_external.borrow().values() {
            assert_eq!(
                entry.commitment,
                google::IMPORT_COMMITMENT,
                "{} is not a draft",
                entry.title
            );
            assert_eq!(entry.source, "google");
            assert_eq!(
                crate::correlate::impact(&entry.kind, entry.commitment),
                crate::correlate::Feasibility::Free
            );
        }
    }

    #[test]
    fn a_moved_google_event_refreshes_a_draft_but_not_a_confirmed_entry() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        import(&store, &FixtureApi::new(events.clone()), &settings(), false).unwrap();

        // The operator adopts one of the three.
        let mut confirmed = store.get("3q7l9v1c8m2p4t6y0x5z");
        confirmed.kind = "work_onsite".into();
        confirmed.commitment = crate::model::Commitment::Committed;
        confirmed.title = "Sprint-Review (verschoben)".into();
        store.seed(confirmed);

        // Google moves both that one and a still-draft one by an hour.
        events[0].start.date_time = Some("2026-08-14T14:00:00+02:00".into());
        events[0].end.date_time = Some("2026-08-14T15:00:00+02:00".into());
        events[2].start.date_time = Some("2026-08-19T09:30:00+02:00".into());
        events[2].end.date_time = Some("2026-08-19T09:45:00+02:00".into());

        let report = import(&store, &FixtureApi::new(events), &settings(), false).unwrap();
        assert_eq!(report.kept_sjel_version, 1);
        assert_eq!(report.refreshed, 1);
        assert_eq!(report.unchanged, 1);

        let kept = store.get("3q7l9v1c8m2p4t6y0x5z");
        assert_eq!(
            kept.starts_at, "2026-08-14T10:00:00",
            "Sjel wins the collision"
        );
        assert_eq!(kept.title, "Sprint-Review (verschoben)");
        assert_eq!(kept.kind, "work_onsite");

        let draft = store.get("base9x7v5t3r1p_20260819T063000Z");
        assert_eq!(
            draft.starts_at, "2026-08-19T09:30:00",
            "an unadopted draft follows Google"
        );

        // The divergence is reported, not swallowed.
        let kept = report
            .outcomes
            .iter()
            .find(|o| o.action == Action::KeepSjelVersion)
            .unwrap();
        assert_eq!(
            kept.google_says.as_ref().unwrap()["start"]["dateTime"],
            "2026-08-14T14:00:00+02:00"
        );
    }

    #[test]
    fn a_cancellation_removes_an_untouched_draft_and_spares_an_adopted_one() {
        let store = FakeStore::default();
        let events = fixture_events();
        import(&store, &FixtureApi::new(events.clone()), &settings(), false).unwrap();

        let mut adopted = store.get("7h2k4n6q8s0u2w4y6a8c");
        adopted.kind = "away".into();
        adopted.commitment = crate::model::Commitment::Committed;
        store.seed(adopted);

        let cancelled: Vec<GoogleEvent> = events
            .iter()
            .map(|event| GoogleEvent {
                id: event.id.clone(),
                status: Some("cancelled".into()),
                ..Default::default()
            })
            .collect();

        let report = import(&store, &FixtureApi::new(cancelled), &settings(), false).unwrap();
        assert_eq!(report.dropped_drafts, 2);
        assert_eq!(report.kept_sjel_version, 1);
        assert_eq!(store.count(), 1);
        assert_eq!(store.get("7h2k4n6q8s0u2w4y6a8c").kind, "away");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let store = FakeStore::default();
        let report = import(
            &store,
            &FixtureApi::new(fixture_events()),
            &settings(),
            true,
        )
        .unwrap();
        assert_eq!(report.created, 3);
        assert!(report.dry_run);
        assert_eq!(
            store.count(),
            0,
            "a dry run reports what it would do, and does not"
        );
    }

    #[test]
    fn review_is_read_only_and_groups_only_exact_title_and_time_twins() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        let mut copy = events[0].clone();
        copy.id = "same-meeting-different-google-id".into();
        copy.summary = Some("TEAM  sync".into());
        events.push(copy);

        let preview = preview(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
        )
        .unwrap();
        assert_eq!(store.count(), 0, "review must never write a draft");
        let twins: Vec<&ImportCandidate> = preview
            .candidates
            .iter()
            .filter(|candidate| candidate.duplicate_group.is_some())
            .collect();
        assert_eq!(twins.len(), 2);
        assert!(twins
            .iter()
            .all(|candidate| candidate.status == ReviewStatus::LikelyDuplicate));
        assert_eq!(twins[0].duplicate_group, twins[1].duplicate_group);
    }

    #[test]
    fn selected_import_writes_only_the_explicit_current_choice() {
        let store = FakeStore::default();
        let events = fixture_events();
        let preview = preview(
            &store,
            &FixtureApi::new(events.clone()),
            &settings(),
            "2026-08-01",
            "2026-08-31",
        )
        .unwrap();
        let selected = preview
            .candidates
            .iter()
            .find(|candidate| candidate.title == "Urlaub")
            .unwrap();
        let report = import_selected(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[SelectedGoogleEvent {
                google_event_id: selected.google_event_id.clone(),
                google_updated: selected.google_updated.clone(),
            }],
        )
        .unwrap();

        assert_eq!(report.created, 1);
        assert_eq!(store.count(), 1);
        assert_eq!(store.get(&selected.google_event_id).title, "Urlaub");
    }

    #[test]
    fn selected_import_refuses_two_members_of_one_duplicate_group() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        let mut copy = events[0].clone();
        copy.id = "second-team-sync".into();
        events.push(copy);

        let error = import_selected(
            &store,
            &FixtureApi::new(events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[
                SelectedGoogleEvent {
                    google_event_id: "3q7l9v1c8m2p4t6y0x5z".into(),
                    google_updated: None,
                },
                SelectedGoogleEvent {
                    google_event_id: "second-team-sync".into(),
                    google_updated: None,
                },
            ],
        )
        .unwrap_err();

        assert!(error.contains("at most one"), "{error}");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn selected_import_rejects_a_google_revision_that_changed_after_review() {
        let store = FakeStore::default();
        let mut reviewed_events = fixture_events();
        reviewed_events[0].updated = Some("2026-08-01T10:00:00Z".into());
        let mut current_events = reviewed_events.clone();
        current_events[0].updated = Some("2026-08-01T11:00:00Z".into());

        let error = import_selected(
            &store,
            &FixtureApi::new(current_events),
            &settings(),
            "2026-08-01",
            "2026-08-31",
            &[SelectedGoogleEvent {
                google_event_id: reviewed_events[0].id.clone(),
                google_updated: reviewed_events[0].updated.clone(),
            }],
        )
        .unwrap_err();

        assert!(error.contains("changed since the preview"), "{error}");
        assert_eq!(store.count(), 0, "a stale preview must not write anything");
    }

    #[test]
    fn review_rejects_an_unbounded_window() {
        let error = review_window("2026-08-01", "2026-11-01").unwrap_err();
        assert!(error.contains("90 days"), "{error}");
    }

    #[test]
    fn an_unmappable_event_is_skipped_with_its_reason_and_the_rest_still_import() {
        let store = FakeStore::default();
        let mut events = fixture_events();
        events.push(GoogleEvent {
            id: "broken".into(),
            status: Some("confirmed".into()),
            summary: Some("Kein Ende".into()),
            start: EventTime {
                date_time: Some("2026-08-21T10:00:00+02:00".into()),
                ..Default::default()
            },
            ..Default::default()
        });

        let report = import(&store, &FixtureApi::new(events), &settings(), false).unwrap();
        assert_eq!(report.created, 3);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].google_event_id, "broken");
        assert!(
            report.skipped[0].reason.contains("no end"),
            "{:?}",
            report.skipped[0]
        );
        assert_eq!(store.count(), 3);
    }

    #[test]
    fn nothing_exports_without_an_opt_in() {
        let store = FakeStore::default();
        let api = FixtureApi::new(vec![]);
        let report = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(report.opted_in, 0);
        assert_eq!(report.inserted, 0);
        assert!(
            api.pushes.borrow().is_empty(),
            "an empty ledger pushes nothing"
        );
    }

    #[test]
    fn an_opted_in_entry_inserts_once_then_patches() {
        let store = FakeStore::default();
        let entry = Entry {
            id: "cal:entry:42".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Vortrag".into(),
            starts_at: "2026-08-14T18:00:00".into(),
            ends_at: "2026-08-14T20:00:00".into(),
            all_day: false,
            location: Some("München".into()),
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let optin = ExportOptIn {
            entry_id: entry.id.clone(),
            google_calendar_id: "primary".into(),
            google_event_id: None,
            pushed_at: None,
            created_at: "0".into(),
        };
        *store.exports.borrow_mut() = vec![(optin.clone(), entry.clone())];

        let api = FixtureApi::new(vec![]);
        let first = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(first.inserted, 1);
        assert_eq!(first.patched, 0);
        assert_eq!(
            store.pushes_recorded.borrow()[0],
            ("cal:entry:42".into(), "g-inserted-1".into())
        );
        assert_eq!(
            api.pushes.borrow()[0].2["start"]["dateTime"],
            "2026-08-14T18:00:00+02:00"
        );

        // The ledger now knows the remote id, so the next run updates it.
        *store.exports.borrow_mut() = vec![(
            ExportOptIn {
                google_event_id: Some("g-inserted-1".into()),
                ..optin
            },
            entry,
        )];
        let second = export(&store, &api, &settings(), false).unwrap();
        assert_eq!(second.patched, 1);
        assert_eq!(second.inserted, 0);
        assert_eq!(api.pushes.borrow()[1].1.as_deref(), Some("g-inserted-1"));
    }

    #[test]
    fn an_export_run_pushes_to_the_calendar_the_entry_was_opted_in_against() {
        // Not the currently configured one: re-pointing the config must not
        // silently relocate an event that already lives somewhere else.
        let store = FakeStore::default();
        let entry = Entry {
            id: "cal:entry:7".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Woanders".into(),
            starts_at: "2026-08-17".into(),
            ends_at: "2026-08-18".into(),
            all_day: true,
            location: None,
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        *store.exports.borrow_mut() = vec![(
            ExportOptIn {
                entry_id: entry.id.clone(),
                google_calendar_id: "team@group.calendar.google.com".into(),
                google_event_id: None,
                pushed_at: None,
                created_at: "0".into(),
            },
            entry,
        )];

        let api = FixtureApi::new(vec![]);
        export(&store, &api, &settings(), false).unwrap();
        assert_eq!(api.pushes.borrow()[0].0, "team@group.calendar.google.com");
    }

    #[test]
    fn a_missing_credential_file_names_the_path_and_the_keys() {
        let missing = std::env::temp_dir().join("axon-calendar-does-not-exist.env");
        let error = read_env_key(&missing, "GOOGLE_CLIENT_ID").unwrap_err();
        assert!(
            error.contains("axon-calendar-does-not-exist.env"),
            "{error}"
        );
        assert!(error.contains("GOOGLE_REFRESH_TOKEN"), "{error}");
        assert!(error.contains("never part of this repo"), "{error}");
    }

    #[test]
    fn an_empty_value_is_missing_not_present() {
        let dir = std::env::temp_dir().join(format!("axon-calendar-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calendar.env");
        std::fs::write(
            &path,
            "GOOGLE_CLIENT_ID=1234.apps.example\nGOOGLE_CLIENT_SECRET=\n",
        )
        .unwrap();

        assert_eq!(
            read_env_key(&path, "GOOGLE_CLIENT_ID").unwrap(),
            "1234.apps.example"
        );
        let empty = read_env_key(&path, "GOOGLE_CLIENT_SECRET").unwrap_err();
        assert!(empty.contains("missing or empty"), "{empty}");
        let absent = read_env_key(&path, "GOOGLE_REFRESH_TOKEN").unwrap_err();
        assert!(absent.contains("GOOGLE_REFRESH_TOKEN"), "{absent}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_refuse_to_run_without_a_home_timezone() {
        let config = Config {
            database_path: std::path::PathBuf::from("/tmp/unused.db"),
            port: 8087,
            home_timezone: None,
            home_city: None,
            trips_base_url: String::new(),
            markdown_sources: Vec::new(),
            google: GoogleConfig {
                calendar_id: Some("primary".into()),
                ..Default::default()
            },
        };
        let error = Settings::resolve(&config).unwrap_err();
        assert!(error.contains("home_timezone is not configured"), "{error}");
        // Assert against the path this environment actually resolves, not a
        // literal. `config_path()` answers the overlay's `config/calendar.json`
        // on a configured machine and falls back to the repo's
        // `calendar.config.json` where there is no overlay — and
        // "calendar.config.json" does not contain "calendar.json", so a
        // hardcoded literal passes on any developer machine and fails in CI.
        // It did, on this test's first remote run. The contract worth pinning is
        // the one `config_path` documents: the error names the exact file the
        // operator has to create, whichever that is here.
        let expected = crate::config::config_path();
        assert!(
            error.contains(&format!("{expected:?}")),
            "error should name {expected:?}, got: {error}"
        );
    }

    #[test]
    fn settings_refuse_to_guess_which_calendar() {
        let config = Config {
            database_path: std::path::PathBuf::from("/tmp/unused.db"),
            port: 8087,
            home_timezone: Some("Europe/Berlin".into()),
            home_city: None,
            trips_base_url: String::new(),
            markdown_sources: Vec::new(),
            google: GoogleConfig::default(),
        };
        let error = Settings::resolve(&config).unwrap_err();
        assert!(error.contains("calendar_id is not configured"), "{error}");
        assert!(error.contains("no default"), "{error}");
    }

    #[test]
    fn settings_resolve_when_both_are_present() {
        let config = Config {
            database_path: std::path::PathBuf::from("/tmp/unused.db"),
            port: 8087,
            home_timezone: Some("Europe/Berlin".into()),
            home_city: None,
            trips_base_url: String::new(),
            markdown_sources: Vec::new(),
            google: GoogleConfig {
                calendar_id: Some("  primary  ".into()),
                ..Default::default()
            },
        };
        let settings = Settings::resolve(&config).unwrap();
        assert_eq!(settings.calendar_id, "primary");
        assert_eq!(settings.tz.name(), "Europe/Berlin");
    }

    #[test]
    fn the_import_window_is_utc_marked_and_never_empty() {
        let today = date::parse_date("2026-08-14").unwrap();
        let (from, to) = import_window(today, 7, 120);
        assert_eq!(from, "2026-08-07T00:00:00Z");
        assert_eq!(to, "2026-12-12T00:00:00Z");

        // A nonsensical config still produces a forward-looking window rather
        // than an inverted one Google would reject.
        let (from, to) = import_window(today, -5, 0);
        assert_eq!(from, "2026-08-14T00:00:00Z");
        assert_eq!(to, "2026-08-15T00:00:00Z");
    }

    #[test]
    fn a_secondary_calendar_id_is_url_encoded() {
        assert_eq!(encode_segment("primary"), "primary");
        assert_eq!(
            encode_segment("abc123@group.calendar.google.com"),
            "abc123%40group.calendar.google.com"
        );
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn http_failures_name_the_likely_cause() {
        assert!(fail_for(reqwest::StatusCode::FORBIDDEN, "events.insert").contains("scope"));
        assert!(fail_for(reqwest::StatusCode::NOT_FOUND, "events.list").contains("calendar_id"));
        assert!(fail_for(reqwest::StatusCode::UNAUTHORIZED, "events.list").contains("revoked"));
        let plain = fail_for(reqwest::StatusCode::BAD_GATEWAY, "events.list");
        assert!(plain.contains("502"), "{plain}");
    }

    #[test]
    fn exportable_filters_out_what_may_never_be_pushed() {
        let base = Entry {
            id: "cal:entry:1".into(),
            kind: "event".into(),
            commitment: crate::model::Commitment::Committed,
            title: "Vortrag".into(),
            starts_at: "2026-08-14T18:00:00".into(),
            ends_at: "2026-08-14T20:00:00".into(),
            all_day: false,
            location: None,
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let imported = Entry {
            id: "cal:entry:2".into(),
            source: "google".into(),
            kind: "draft".into(),
            ..base.clone()
        };
        let generated = Entry {
            id: "cal:entry:3".into(),
            rhythm_id: Some("cal:rhythm:1".into()),
            ..base.clone()
        };
        let entries = [base, imported, generated];
        let allowed = exportable(&entries);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].id, "cal:entry:1");
    }
}

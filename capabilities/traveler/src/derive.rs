//! The derived baseline: what the stored trips actually show.
//!
//! Thirteen real plans, their dates, their companions and their interests were
//! being captured and read by nothing. This is the read. It is a projection over
//! rows another capability owns, computed on demand and stored nowhere — the
//! same shape `capabilities/places/src/layers.rs` uses for the travel layer, and
//! for the same reason: the facts belong to `trips`, and a second copy would be
//! a second thing to keep true.
//!
//! ## What this deliberately does not compute
//!
//! **Attendance.** A plan is not a trip. The one signal that exists is a
//! retrospective recorded `not_taken`, and those are excluded by plan id. Every
//! other plan is counted whether or not it happened, because nothing records
//! that it did — so the response says so rather than implying a precision it
//! does not have.
//!
//! **Anything inferred from free text.** `plan.interests` is prose. Turning a
//! sentence about what someone likes into an anchor kind is a guess that would
//! read as a measurement, which is the failure `plan_search.rs` refuses by
//! returning `None` rather than a zero. The counts here are all arithmetic over
//! typed columns. That is also why the tests below use invented interests: a real
//! one is a personal fact and this repository is public.
//!
//! **Names.** Company is counted as a shape — solo, pair, group — never listed.
//! `travelers` holds real names and is already served by `GET /api/plans`, which
//! `Packs/travel/ISA.md` records as an open leak; this projection must not be a
//! second place they leave by.

use serde::{Deserialize, Serialize};

use crate::store::{validate_prefix, TravelerStore};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The prefix `trips` writes its tables under.
pub const TRIPS_PREFIX: &str = "trips";

/// The sentence every response carries, because the counts below cannot say it
/// themselves. Served rather than documented so a consumer that never reads this
/// file still learns what the number means.
pub const ATTENDANCE_NOTE: &str =
    "counts plans, not confirmed trips: a plan is excluded only when its retrospective records \
     not_taken, and nothing records that any other plan was attended";

/// Why the lead-time spread covers fewer plans than the rest of the response.
pub const LEAD_TIME_NOTE: &str =
    "lead_time_days covers only plans whose row was created before the trip started: the vault \
     import stamps created_at with the import date, so an imported trip's row age is how long \
     ago the import ran, not how far ahead it was booked";

/// Why the destination counts under-count repeats.
pub const DESTINATION_NOTE: &str =
    "destinations are as stored, so one city typed two ways counts as two and a multi-stop plan \
     is a single name; repeat_destinations under-counts for the same reason";

/// A min/median/max over one measurement. Median rather than mean because trip
/// length and booking lead time are both skewed by a single outlier — a
/// twenty-day stay and a trip booked a year ahead would each move a mean and
/// neither would move a median.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spread {
    pub min: i64,
    pub median: i64,
    pub max: i64,
}

impl Spread {
    fn of(mut values: Vec<i64>) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        values.sort_unstable();
        let middle = values.len() / 2;
        let median = if values.len() % 2 == 1 {
            values[middle]
        } else {
            (values[middle - 1] + values[middle]) / 2
        };
        Some(Self {
            min: values[0],
            median,
            max: values[values.len() - 1],
        })
    }
}

/// One destination and how many plans named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationCount {
    pub name: String,
    pub trips: usize,
}

/// One bucket of a histogram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    pub key: String,
    pub trips: usize,
}

/// How many people were on the plan, as a shape rather than a list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Company {
    /// No travellers recorded. Not the same as travelling alone — the field is
    /// empty for a plan nobody filled in.
    pub unrecorded: usize,
    /// Exactly one traveller besides the operator's own entry, or one name.
    pub pair: usize,
    pub group: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedTravel {
    /// Plans counted, after exclusions.
    pub considered: usize,
    /// Plan ids left out because a retrospective records `not_taken`.
    pub excluded_not_taken: Vec<String>,
    pub length_days: Option<Spread>,
    /// Days between the plan row being created and the trip starting.
    ///
    /// Only over plans whose row predates the trip, and that restriction is the
    /// measurement. The vault import stamps `created_at` with the import date,
    /// so for imported history the row's age is how long ago the import ran — a
    /// negative number that is not a lead time at all. `lead_time_plans` and
    /// `lead_time_skipped` say how much of the set this rests on, because a
    /// median over two plans and a median over twelve must not read the same.
    pub lead_time_days: Option<Spread>,
    /// How many plans the lead-time spread was computed from.
    pub lead_time_plans: usize,
    /// Plans whose row was created after the trip had already started.
    pub lead_time_skipped: usize,
    /// Every destination named, most-visited first.
    pub destinations: Vec<DestinationCount>,
    /// The subset named by more than one plan.
    ///
    /// Under-counts, and the notes say why: the same city typed two ways is two
    /// destinations, which is the identity problem `places` already detects and
    /// reports as `merge_candidates`.
    pub repeat_destinations: Vec<DestinationCount>,
    pub company: Company,
    /// Plans per calendar month, `01`..`12`, most-used first.
    pub months: Vec<Bucket>,
    pub modes: Vec<Bucket>,
    /// The plan ids every number above was computed from. The whole point of
    /// publishing it: a reader can check the set rather than trust the summary.
    pub basis: Vec<String>,
    /// What these counts cannot say, in the response rather than in a comment,
    /// so a consumer that never reads this file still learns the limits.
    pub notes: Vec<String>,
}

/// One plan row, as much of it as this projection reads.
struct PlanRow {
    id: String,
    date_start: String,
    date_end: String,
    created_at: String,
    travelers: String,
    transport_modes: String,
    destinations: String,
}

/// Count trips, lengths, lead times, destinations, company, months and modes.
pub fn derive(store: &TravelerStore, trips_prefix: &str) -> Fallible<DerivedTravel> {
    validate_prefix(trips_prefix)?;
    let conn = store.conn()?;

    // A machine where `trips` has never run has no tables at all. That is
    // "nothing recorded yet", not a failure — the rule `punctuality` states for
    // an unscored leg, applied to a whole projection. Without this the route
    // answers 500 on a fresh install and the dashboard reads it as an outage.
    if !table_exists(&conn, &format!("{trips_prefix}_plans"))? {
        return Ok(empty_baseline(Vec::new()));
    }

    let not_taken: Vec<String> = if table_exists(&conn, &format!("{trips_prefix}_retrospectives"))?
    {
        use sjel_store::QueryAll;
        conn.query_all(
            &format!(
                "SELECT plan_id FROM {trips_prefix}_retrospectives WHERE again = 'not_taken' \
                 ORDER BY plan_id"
            ),
            [],
            |row| row.get(0),
        )?
    } else {
        Vec::new()
    };

    let rows: Vec<PlanRow> = {
        use sjel_store::QueryAll;
        conn.query_all(
            &format!(
                "SELECT id, date_start, date_end, created_at, travelers, transport_modes, \
                        destinations
                 FROM {trips_prefix}_plans
                 WHERE status != 'archived'
                 ORDER BY date_start, id"
            ),
            [],
            |row| {
                Ok(PlanRow {
                    id: row.get(0)?,
                    date_start: row.get(1)?,
                    date_end: row.get(2)?,
                    created_at: row.get(3)?,
                    travelers: row.get(4)?,
                    transport_modes: row.get(5)?,
                    destinations: row.get(6)?,
                })
            },
        )?
    };

    let mut lengths: Vec<i64> = Vec::new();
    let mut leads: Vec<i64> = Vec::new();
    let mut lead_time_skipped = 0_usize;
    let mut destinations: Vec<DestinationCount> = Vec::new();
    let mut months: Vec<Bucket> = Vec::new();
    let mut modes: Vec<Bucket> = Vec::new();
    let mut company = Company::default();
    let mut basis: Vec<String> = Vec::new();

    for row in &rows {
        if not_taken.contains(&row.id) {
            continue;
        }
        basis.push(row.id.clone());

        if let (Some(start), Some(end)) = (
            civil_date::unix_day_of_iso(&row.date_start),
            civil_date::unix_day_of_iso(&row.date_end),
        ) {
            // Inclusive, so a one-day trip is 1 and not 0 — the same convention
            // `Packs/travel` uses for a date range a person reads.
            lengths.push(end - start + 1);
            // A row created after the trip started is not a booking lead time,
            // it is the import date. Counting it would publish a negative number
            // as a measurement, which is what the first live run did — a negative
            // median over a history that was imported, not booked. The figures are
            // in `<overlay>/data/traveler/evidence-2026-09-23.md` rather than here,
            // because this repository is public.
            match epoch_day(&row.created_at) {
                Some(created) if start >= created => leads.push(start - created),
                Some(_) => lead_time_skipped += 1,
                None => lead_time_skipped += 1,
            }
            let month = row.date_start.get(5..7).unwrap_or_default().to_string();
            bump(&mut months, &month);
        }

        for name in destination_names(&row.destinations) {
            match destinations.iter_mut().find(|entry| entry.name == name) {
                Some(entry) => entry.trips += 1,
                None => destinations.push(DestinationCount { name, trips: 1 }),
            }
        }

        match traveller_count(&row.travelers) {
            0 => company.unrecorded += 1,
            1 => company.pair += 1,
            _ => company.group += 1,
        }

        for mode in string_array(&row.transport_modes) {
            bump(&mut modes, &mode);
        }
    }

    destinations.sort_by(|a, b| b.trips.cmp(&a.trips).then_with(|| a.name.cmp(&b.name)));
    let repeat_destinations = destinations
        .iter()
        .filter(|entry| entry.trips > 1)
        .cloned()
        .collect();
    sort_buckets(&mut months);
    sort_buckets(&mut modes);

    Ok(DerivedTravel {
        considered: basis.len(),
        excluded_not_taken: not_taken,
        length_days: Spread::of(lengths),
        lead_time_plans: leads.len(),
        lead_time_skipped,
        lead_time_days: Spread::of(leads),
        destinations,
        repeat_destinations,
        company,
        months,
        modes,
        basis,
        notes: notes(),
    })
}

/// Whether a capability's table exists yet. A prefix that is a capability's own
/// name and a name that reached SQL only through `format!` with a validated
/// prefix, so the bound parameter is the whole guard this needs.
fn table_exists(conn: &rusqlite::Connection, name: &str) -> Fallible<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        rusqlite::params![name],
        |row| row.get(0),
    )?)
}

/// Every limit this projection has, in the response rather than in a comment.
fn notes() -> Vec<String> {
    vec![
        ATTENDANCE_NOTE.to_string(),
        LEAD_TIME_NOTE.to_string(),
        DESTINATION_NOTE.to_string(),
    ]
}

/// The baseline for a store with nothing in it: absences, not zeros.
fn empty_baseline(excluded_not_taken: Vec<String>) -> DerivedTravel {
    DerivedTravel {
        considered: 0,
        excluded_not_taken,
        length_days: None,
        lead_time_days: None,
        lead_time_plans: 0,
        lead_time_skipped: 0,
        destinations: Vec::new(),
        repeat_destinations: Vec::new(),
        company: Company::default(),
        months: Vec::new(),
        modes: Vec::new(),
        basis: Vec::new(),
        notes: notes(),
    }
}

/// A stored timestamp as a day number.
///
/// `trips` writes timestamps as Unix epoch SECONDS in a TEXT column
/// (`store::now_text`), while its dates are ISO. Both forms reach this function
/// through different columns, so the conversion is named rather than inlined —
/// reading `created_at` as an ISO date would silently yield `None` and drop
/// every lead time rather than failing.
fn epoch_day(stamp: &str) -> Option<i64> {
    let seconds: i64 = stamp.trim().parse().ok()?;
    Some(seconds.div_euclid(86_400))
}

fn bump(buckets: &mut Vec<Bucket>, key: &str) {
    if key.is_empty() {
        return;
    }
    match buckets.iter_mut().find(|bucket| bucket.key == key) {
        Some(bucket) => bucket.trips += 1,
        None => buckets.push(Bucket {
            key: key.to_string(),
            trips: 1,
        }),
    }
}

/// Most-used first, then by key so two runs agree.
fn sort_buckets(buckets: &mut [Bucket]) {
    buckets.sort_by(|a, b| b.trips.cmp(&a.trips).then_with(|| a.key.cmp(&b.key)));
}

fn string_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// The `name` of every destination `PlaceRef`, trimmed and de-duplicated within
/// one plan so a plan naming one city twice counts it once.
fn destination_names(raw: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.get("name").and_then(|name| name.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn traveller_count(raw: &str) -> usize {
    string_array(raw).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spread_is_none_rather_than_zero_when_there_is_nothing_to_measure() {
        assert_eq!(Spread::of(Vec::new()), None);
        assert_eq!(
            Spread::of(vec![3]),
            Some(Spread {
                min: 3,
                median: 3,
                max: 3
            })
        );
    }

    #[test]
    fn a_median_ignores_the_outlier_a_mean_would_follow() {
        // 2, 3, 4, 5 and a twenty-day stay: the median stays at 4 while a mean
        // would be 6.8. This is why the baseline publishes a median.
        let spread = Spread::of(vec![2, 3, 4, 5, 20]).unwrap();
        assert_eq!(spread.median, 4);
        assert_eq!(spread.max, 20);
        // Even counts average the two middle values rather than picking one.
        assert_eq!(Spread::of(vec![2, 4]).unwrap().median, 3);
    }

    #[test]
    fn an_epoch_stamp_is_read_as_seconds_and_a_date_is_not() {
        // The bug this exists to prevent: `created_at` is epoch seconds in a
        // TEXT column, so reading it as ISO yields None and every lead time
        // silently disappears instead of failing.
        assert_eq!(epoch_day("0"), Some(0));
        assert_eq!(epoch_day("86400"), Some(1));
        assert_eq!(epoch_day("2026-01-01"), None);
        assert_eq!(epoch_day(""), None);
    }

    #[test]
    fn company_counts_a_shape_and_never_a_name() {
        assert_eq!(traveller_count("[]"), 0);
        assert_eq!(traveller_count(r#"["Someone"]"#), 1);
        assert_eq!(
            traveller_count(r#"["One","Two","Three"]"#),
            3,
            "the count is all this reads; the names never leave the function"
        );
        assert_eq!(traveller_count("not json"), 0);
    }

    #[test]
    fn destination_names_deduplicate_within_one_plan() {
        let raw = r#"[
            {"id":"place:berlin","name":"Berlin"},
            {"id":"place:berlin-2","name":"Berlin"},
            {"id":"place:bonn","name":" Bonn "},
            {"id":"place:x","name":""},
            {"id":"place:y"}
        ]"#;
        assert_eq!(
            destination_names(raw),
            vec!["Berlin".to_string(), "Bonn".to_string()]
        );
    }

    #[test]
    fn buckets_are_ordered_by_count_then_key() {
        let mut buckets = vec![
            Bucket {
                key: "10".into(),
                trips: 1,
            },
            Bucket {
                key: "03".into(),
                trips: 2,
            },
            Bucket {
                key: "12".into(),
                trips: 1,
            },
        ];
        sort_buckets(&mut buckets);
        assert_eq!(
            buckets.iter().map(|b| b.key.as_str()).collect::<Vec<_>>(),
            vec!["03", "10", "12"],
            "a tie breaks by key so two runs agree"
        );
    }

    // ── the projection itself, over real tables ──────────────────────────

    /// `trips` writes epoch seconds into a TEXT timestamp column and ISO into
    /// its date columns. Building the fixture through `civil_date` rather than
    /// pasting a literal keeps that arithmetic out of the test's own head.
    fn epoch_of(iso: &str) -> String {
        (civil_date::unix_day_of_iso(iso).unwrap() * 86_400).to_string()
    }

    fn scratch_with_trips(name: &str) -> (TravelerStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("axon-traveler-derive-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TravelerStore::open(&dir.join("axon.db")).unwrap();
        let conn = store.conn().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE trips_plans (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, origin TEXT NOT NULL,
                destinations TEXT NOT NULL, date_start TEXT NOT NULL, date_end TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'saved',
                travelers TEXT NOT NULL DEFAULT '[]',
                transport_modes TEXT NOT NULL DEFAULT '[]'
            );
            CREATE TABLE trips_retrospectives (
                plan_id TEXT PRIMARY KEY, cost_cents INTEGER, again TEXT NOT NULL,
                change_note TEXT NOT NULL DEFAULT '', filled_at TEXT NOT NULL
            );
            ",
        )
        .unwrap();
        drop(conn);
        (store, dir)
    }

    /// One plan row for the fixture. A struct rather than eight positional
    /// arguments, because which of `?4` and `?5` is the start date is exactly
    /// the kind of thing a caller gets wrong with the compiler saying nothing.
    struct PlanFixture<'a> {
        id: &'a str,
        start: &'a str,
        end: &'a str,
        created: &'a str,
        travellers: &'a str,
        modes: &'a str,
        destinations: &'a str,
    }

    fn insert_plan(store: &TravelerStore, plan: PlanFixture<'_>) {
        let conn = store.conn().unwrap();
        conn.execute(
            "INSERT INTO trips_plans
                (id, title, origin, destinations, date_start, date_end, created_at,
                 updated_at, status, travelers, transport_modes)
             VALUES (?1, ?2, 'Bonn', ?3, ?4, ?5, ?6, ?6, 'saved', ?7, ?8)",
            rusqlite::params![
                plan.id,
                format!("plan {}", plan.id),
                plan.destinations,
                plan.start,
                plan.end,
                epoch_of(plan.created),
                plan.travellers,
                plan.modes
            ],
        )
        .unwrap();
    }

    #[test]
    fn the_baseline_counts_plans_and_keeps_a_not_taken_one_out() {
        let (store, dir) = scratch_with_trips("baseline");
        let berlin = r#"[{"id":"place:berlin","name":"Berlin"}]"#;

        insert_plan(
            &store,
            PlanFixture {
                id: "p1",
                start: "2026-03-10",
                end: "2026-03-13",
                created: "2026-03-01",
                travellers: r#"["A"]"#,
                modes: r#"["train"]"#,
                destinations: berlin,
            },
        );
        insert_plan(
            &store,
            PlanFixture {
                id: "p2",
                start: "2026-03-20",
                end: "2026-03-21",
                created: "2026-03-01",
                travellers: "[]",
                modes: r#"["train","flight"]"#,
                destinations: berlin,
            },
        );
        // The trip that did not happen. Same destination, so if the exclusion
        // were dropped it would show up as a third Berlin visit rather than as
        // an obviously wrong count.
        insert_plan(
            &store,
            PlanFixture {
                id: "p3",
                start: "2026-08-16",
                end: "2026-08-19",
                created: "2026-03-01",
                travellers: "[]",
                modes: "[]",
                destinations: berlin,
            },
        );
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO trips_retrospectives
                    (plan_id, cost_cents, again, change_note, filled_at)
                 VALUES ('p3', NULL, 'not_taken', 'was not there', '0')",
                [],
            )
            .unwrap();

        let derived = derive(&store, TRIPS_PREFIX).unwrap();

        assert_eq!(derived.considered, 2);
        assert_eq!(derived.basis, vec!["p1".to_string(), "p2".to_string()]);
        assert_eq!(derived.excluded_not_taken, vec!["p3".to_string()]);

        // 4 days and 2 days; the median of an even count is the average of the
        // two middle values.
        assert_eq!(
            derived.length_days,
            Some(Spread {
                min: 2,
                median: 3,
                max: 4
            })
        );
        // Created 2026-03-01, starting 03-10 and 03-20.
        assert_eq!(
            derived.lead_time_days,
            Some(Spread {
                min: 9,
                median: 14,
                max: 19
            })
        );

        assert_eq!(
            derived.destinations,
            vec![DestinationCount {
                name: "Berlin".into(),
                trips: 2
            }]
        );
        assert_eq!(derived.repeat_destinations, derived.destinations);
        assert_eq!(
            derived.company,
            Company {
                unrecorded: 1,
                pair: 1,
                group: 0
            }
        );
        assert_eq!(
            derived.months,
            vec![Bucket {
                key: "03".into(),
                trips: 2
            }]
        );
        assert_eq!(
            derived.modes,
            vec![
                Bucket {
                    key: "train".into(),
                    trips: 2
                },
                Bucket {
                    key: "flight".into(),
                    trips: 1
                }
            ]
        );
        // The response says what it counted, because the counts cannot.
        assert!(derived
            .notes
            .iter()
            .any(|note| note.contains("not confirmed trips")));
        assert!(derived
            .notes
            .iter()
            .any(|note| note.contains("import stamps created_at")));
        assert!(derived
            .notes
            .iter()
            .any(|note| note.contains("typed two ways")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_imported_trip_is_left_out_of_the_lead_time_rather_than_counted_negative() {
        // The first live run over a vault-imported history published a negative
        // median — the row's age, not a booking lead time, rendered as a
        // measurement. A plan whose row was created after the trip started must
        // be skipped and counted, never averaged in. The figures are in
        // `<overlay>/data/traveler/evidence-2026-09-23.md`.
        let (store, dir) = scratch_with_trips("imported");
        let berlin = r#"[{"id":"place:berlin","name":"Berlin"}]"#;
        // Trip in March, imported in September: created_at after date_start.
        insert_plan(
            &store,
            PlanFixture {
                id: "old",
                start: "2026-03-10",
                end: "2026-03-13",
                created: "2026-09-08",
                travellers: "[]",
                modes: "[]",
                destinations: berlin,
            },
        );
        // A real booking: created in September for an October trip.
        insert_plan(
            &store,
            PlanFixture {
                id: "ahead",
                start: "2026-10-07",
                end: "2026-10-13",
                created: "2026-09-08",
                travellers: "[]",
                modes: "[]",
                destinations: berlin,
            },
        );

        let derived = derive(&store, TRIPS_PREFIX).unwrap();
        assert_eq!(
            derived.considered, 2,
            "both still count toward everything else"
        );
        assert_eq!(derived.lead_time_plans, 1);
        assert_eq!(derived.lead_time_skipped, 1);
        assert_eq!(
            derived.lead_time_days,
            Some(Spread {
                min: 29,
                median: 29,
                max: 29
            }),
            "the one real booking, and no negative number from the import"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_history_with_no_bookings_has_no_lead_time_rather_than_a_negative_one() {
        let (store, dir) = scratch_with_trips("no-bookings");
        let berlin = r#"[{"id":"place:berlin","name":"Berlin"}]"#;
        insert_plan(
            &store,
            PlanFixture {
                id: "old",
                start: "2026-03-10",
                end: "2026-03-13",
                created: "2026-09-08",
                travellers: "[]",
                modes: "[]",
                destinations: berlin,
            },
        );
        let derived = derive(&store, TRIPS_PREFIX).unwrap();
        assert_eq!(derived.lead_time_days, None);
        assert_eq!(derived.lead_time_plans, 0);
        assert_eq!(derived.lead_time_skipped, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_store_yields_absences_rather_than_zeros() {
        let (store, dir) = scratch_with_trips("empty");
        let derived = derive(&store, TRIPS_PREFIX).unwrap();
        assert_eq!(derived.considered, 0);
        assert_eq!(derived.length_days, None, "no evidence is not a zero");
        assert_eq!(derived.lead_time_days, None);
        assert_eq!(derived.lead_time_plans, 0);
        assert_eq!(derived.lead_time_skipped, 0);
        assert!(derived.destinations.is_empty());
        assert!(derived.basis.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_machine_where_trips_has_never_run_reads_as_empty_rather_than_failing() {
        // No trips tables at all: the shape of a fresh install. The route must
        // answer, not 500 — a dashboard reads a 500 as an outage.
        let dir = std::env::temp_dir().join("axon-traveler-derive-notrips");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TravelerStore::open(&dir.join("axon.db")).unwrap();

        let derived = derive(&store, TRIPS_PREFIX).unwrap();
        assert_eq!(derived.considered, 0);
        assert!(derived.excluded_not_taken.is_empty());
        assert_eq!(derived.length_days, None);
        assert_eq!(derived.lead_time_days, None);
        assert_eq!(derived.notes.len(), 3, "the limits travel with the answer");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plans_without_a_retrospectives_table_still_derive() {
        // The narrower gap: trips has run, retrospectives has not. Nothing is
        // excluded and nothing fails.
        let (store, dir) = scratch_with_trips("no-retro");
        store
            .conn()
            .unwrap()
            .execute_batch("DROP TABLE trips_retrospectives;")
            .unwrap();
        let derived = derive(&store, TRIPS_PREFIX).unwrap();
        assert_eq!(derived.considered, 0);
        assert!(derived.excluded_not_taken.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_prefix_that_could_carry_sql_is_refused_before_any_read() {
        let (store, dir) = scratch_with_trips("prefix");
        assert!(derive(&store, "trips; DROP TABLE trips_plans").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}

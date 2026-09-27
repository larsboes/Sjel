//! The profile table, in the shared Axon database under its own prefix.
//!
//! One row per profile id. The whole profile is stored as columns of JSON rather
//! than as a column per field, deliberately: the profile is read and written
//! whole, nothing joins on `hard.earliest_departure`, and a column per field
//! would make every added constraint a migration. `basis` is stored the same way
//! for the same reason — it is the field set's shadow and moves with it.
//!
//! The store never resolves a profile id from anywhere but its caller. There is
//! no "current user", which is the point: the day a second person needs a
//! profile is the day `id` starts meaning something, and nothing here would have
//! to change.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use sjel_store::QueryAll;

use crate::model::{ProfileInput, TravelProfile, DEFAULT_PROFILE_ID};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The database prefix every table here carries.
pub const PREFIX: &str = "traveler";

pub struct TravelerStore {
    pool: sjel_store::Pool,
    prefix: String,
}

/// What a write did.
///
/// `Stale` is an outcome rather than an error because it is not a failure: the
/// caller read a revision, someone else wrote, and the caller needs to look
/// again. Returning it through the error channel would make a routine
/// concurrency event indistinguishable from a broken store.
///
/// The profile is boxed because the two variants differ by two orders of
/// magnitude — 337 bytes against 4 — and every caller that only wants the
/// revision would otherwise carry the whole profile on the stack. `PutOutcome`
/// is returned by value from `put`, so this is the difference between a cheap
/// enum and one that memcpys a profile to say "someone else got there first".
#[derive(Debug)]
pub enum PutOutcome {
    Stored {
        profile: Box<TravelProfile>,
        created: bool,
    },
    Stale {
        current_revision: u32,
    },
}

impl TravelerStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, PREFIX)
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Fallible<Self> {
        validate_prefix(prefix)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    pub(crate) fn conn(&self) -> Fallible<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The cheapest statement that proves this store can reach its database,
    /// which is what `/ready` promises rather than mere liveness.
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_profiles (
                id          TEXT PRIMARY KEY,
                hard        TEXT NOT NULL,
                soft        TEXT NOT NULL,
                journey     TEXT NOT NULL,
                interests   TEXT NOT NULL,
                pace        TEXT NOT NULL,
                anchors     TEXT NOT NULL,
                basis       TEXT NOT NULL,
                revision    INTEGER NOT NULL,
                updated_at  TEXT NOT NULL
            );
            "
        ))?;
        Self::add_journey_column(conn, prefix)?;
        Ok(())
    }

    /// Add the journey weight block to a table that predates it.
    ///
    /// SQLite can add a column but cannot alter one, so this is the easy half of
    /// the migration `capabilities/trips` needed for `not_taken`: no rebuild, no
    /// copy, no temporary table. The default is `{}`, which `JourneyWeights`'s
    /// `#[serde(default)]` reads as the built-in values — so an existing row keeps
    /// working and its basis still says `default`, which is the truth: nobody has
    /// stated a journey weight on this machine yet.
    ///
    /// Idempotent, and a no-op on a fresh file because the `CREATE` above has
    /// already written the column. The test is `pragma_table_info` rather than a
    /// version number, for the same reason `trips` tests its stored DDL: the
    /// column IS the fact being asked about.
    fn add_journey_column(conn: &Connection, prefix: &str) -> Fallible<()> {
        let table = format!("{prefix}_profiles");
        let present: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = 'journey')",
            params![&table],
            |row| row.get(0),
        )?;
        if present {
            return Ok(());
        }
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN journey TEXT NOT NULL DEFAULT '{{}}';"
        ))?;
        Ok(())
    }

    /// The stored profile, or `None` when nothing has ever been written.
    ///
    /// `None` rather than `TravelProfile::unstated()` so the caller decides what
    /// to hand a consumer; the HTTP layer turns this into the unstated profile
    /// plus `stored: false`, and a test asserts that is the only place the
    /// conversion happens.
    pub fn get(&self, id: &str) -> Fallible<Option<TravelProfile>> {
        let conn = self.conn()?;
        let prefix = &self.prefix;
        let row = conn
            .query_row(
                &format!(
                    "SELECT id, hard, soft, journey, interests, pace, anchors, basis, revision,
                            updated_at
                     FROM {prefix}_profiles WHERE id = ?1"
                ),
                params![id],
                row_to_profile,
            )
            .optional()?;
        // A row written before a block existed has no provenance for that
        // block, so the read completes it. The alternative — refusing the write
        // instead — would mean a migration that adds a block silently makes
        // every existing profile un-editable.
        Ok(row.map(TravelProfile::with_complete_basis))
    }

    /// Every stored profile id, ordered so two runs agree.
    pub fn ids(&self) -> Fallible<Vec<String>> {
        let conn = self.conn()?;
        let prefix = &self.prefix;
        let ids = conn.query_all(
            &format!("SELECT id FROM {prefix}_profiles ORDER BY id"),
            [],
            |row| row.get(0),
        )?;
        Ok(ids)
    }

    /// Write one profile, conditionally.
    ///
    /// The revision check happens inside the write transaction, not before it.
    /// Read-then-write outside a transaction is the lost-update race the
    /// dashboard already has to avoid by hand (`capabilities/trips` states the
    /// same rule for `expected_updated_at`), and an agent holds a read across
    /// turns while it calls other capabilities, which is exactly the window this
    /// closes.
    pub fn put(
        &self,
        id: &str,
        input: &ProfileInput,
        expected_revision: Option<u32>,
    ) -> Fallible<PutOutcome> {
        let mut conn = self.conn()?;
        let prefix = self.prefix.clone();
        let tx = sjel_store::write_transaction(&mut conn)?;

        let current: Option<u32> = tx
            .query_row(
                &format!("SELECT revision FROM {prefix}_profiles WHERE id = ?1"),
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let current_revision = current.unwrap_or(0);

        if let Some(expected) = expected_revision {
            if expected != current_revision {
                // Dropping the transaction rolls it back, which is the right
                // outcome: nothing was written, so there is nothing to undo.
                drop(tx);
                return Ok(PutOutcome::Stale { current_revision });
            }
        }

        let created = current.is_none();
        let revision = current_revision + 1;
        tx.execute(
            &format!(
                "INSERT INTO {prefix}_profiles
                     (id, hard, soft, journey, interests, pace, anchors, basis, revision, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('{stamp}','now'))
                 ON CONFLICT(id) DO UPDATE SET
                     hard = excluded.hard,
                     soft = excluded.soft,
                     journey = excluded.journey,
                     interests = excluded.interests,
                     pace = excluded.pace,
                     anchors = excluded.anchors,
                     basis = excluded.basis,
                     revision = excluded.revision,
                     updated_at = excluded.updated_at",
                stamp = sjel_store::STAMP_FORMAT
            ),
            params![
                id,
                serde_json::to_string(&input.hard)?,
                serde_json::to_string(&input.soft)?,
                serde_json::to_string(&input.journey)?,
                serde_json::to_string(&input.interests)?,
                serde_json::to_string(&input.pace)?,
                serde_json::to_string(&input.anchors)?,
                serde_json::to_string(&input.basis)?,
                revision,
            ],
        )?;
        tx.commit()?;

        let profile = self
            .get(id)?
            .ok_or("the profile just written could not be read back")?;
        Ok(PutOutcome::Stored {
            profile: Box::new(profile),
            created,
        })
    }
}

/// A table prefix is interpolated into SQL rather than bound, because SQLite has
/// no placeholder for an identifier. So it is checked here, once, on the way in.
///
/// Same rule and same reason as `capabilities/places/src/store.rs`: the prefix
/// arrives from configuration, and a value that reaches `format!` unchecked is
/// how a config typo becomes SQL.
pub(crate) fn validate_prefix(prefix: &str) -> Fallible<()> {
    let ok = !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid table prefix: {prefix:?}").into())
    }
}

fn row_to_profile(row: &Row<'_>) -> rusqlite::Result<TravelProfile> {
    Ok(TravelProfile {
        id: row.get(0)?,
        hard: sjel_store::json_column(row, 1)?,
        soft: sjel_store::json_column(row, 2)?,
        journey: sjel_store::json_column(row, 3)?,
        interests: sjel_store::json_column(row, 4)?,
        pace: sjel_store::json_column(row, 5)?,
        anchors: sjel_store::json_column(row, 6)?,
        basis: sjel_store::json_column(row, 7)?,
        revision: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// The JSON of one profile, for a caller that wants to compare two writes
/// byte for byte rather than field by field.
pub fn canonical_json(profile: &TravelProfile) -> Fallible<Value> {
    Ok(serde_json::to_value(profile)?)
}

/// A profile id is a label, not a path: it names a row, never a file.
pub fn default_id() -> &'static str {
    DEFAULT_PROFILE_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;

    fn scratch(name: &str) -> (TravelerStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("axon-traveler-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("axon.db");
        (TravelerStore::open(&path).unwrap(), dir)
    }

    #[test]
    fn an_unwritten_profile_reads_as_none_rather_than_as_defaults() {
        let (store, dir) = scratch("unwritten");
        assert!(store.get("default").unwrap().is_none());
        assert!(store.ids().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_is_readable_back_and_starts_at_revision_one() {
        let (store, dir) = scratch("write");
        let mut input = ProfileInput::unstated();
        input.hard.earliest_departure = Some("07:00".into());
        input
            .basis
            .insert("hard.earliest_departure".into(), Provenance::Stated);

        let outcome = store.put("default", &input, None).unwrap();
        let PutOutcome::Stored { profile, created } = outcome else {
            panic!("a first write must not be stale");
        };
        assert!(created);
        assert_eq!(profile.revision, 1);
        assert_eq!(profile.hard.earliest_departure.as_deref(), Some("07:00"));
        assert_eq!(
            profile.basis.get("hard.earliest_departure"),
            Some(&Provenance::Stated)
        );
        assert!(!profile.updated_at.is_empty());

        let read = store.get("default").unwrap().unwrap();
        assert_eq!(read, *profile);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_write_replaces_the_first_and_bumps_the_revision() {
        let (store, dir) = scratch("replace");
        let input = ProfileInput::unstated();
        store.put("default", &input, None).unwrap();
        let outcome = store.put("default", &input, Some(1)).unwrap();
        let PutOutcome::Stored { profile, created } = outcome else {
            panic!("revision 1 was current, so this write is not stale");
        };
        assert!(!created);
        assert_eq!(profile.revision, 2);
        assert_eq!(store.ids().unwrap(), vec!["default".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_write_against_a_stale_revision_is_refused_and_changes_nothing() {
        let (store, dir) = scratch("stale");
        let input = ProfileInput::unstated();
        store.put("default", &input, None).unwrap();

        let outcome = store.put("default", &input, Some(99)).unwrap();
        match outcome {
            PutOutcome::Stale { current_revision } => assert_eq!(current_revision, 1),
            PutOutcome::Stored { .. } => panic!("a stale write must not be stored"),
        }
        // The refused write left the row exactly as it was, revision included.
        assert_eq!(store.get("default").unwrap().unwrap().revision, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unconditional_write_ignores_the_stored_revision() {
        let (store, dir) = scratch("unconditional");
        let input = ProfileInput::unstated();
        store.put("default", &input, None).unwrap();
        store.put("default", &input, None).unwrap();
        let outcome = store.put("default", &input, None).unwrap();
        let PutOutcome::Stored { profile, .. } = outcome else {
            panic!("an unconditional write is never stale");
        };
        assert_eq!(profile.revision, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn two_profiles_do_not_see_each_other() {
        let (store, dir) = scratch("two");
        let mut one = ProfileInput::unstated();
        // An invented station. The first version of this test used a real one,
        // which is the easiest way for a personal fact to sit in a public repo
        // unnoticed — a fixture reads as arbitrary even when it is not.
        one.hard.home_stations = vec!["Example City Hbf".into()];
        one.basis
            .insert("hard.home_stations".into(), Provenance::Stated);
        store.put("default", &one, None).unwrap();
        store
            .put("second", &ProfileInput::unstated(), None)
            .unwrap();

        let default = store.get("default").unwrap().unwrap();
        assert_eq!(
            default.hard.home_stations,
            vec!["Example City Hbf".to_string()]
        );
        assert!(store
            .get("second")
            .unwrap()
            .unwrap()
            .hard
            .home_stations
            .is_empty());
        assert_eq!(
            store.ids().unwrap(),
            vec!["default".to_string(), "second".to_string()]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_table_without_the_journey_column_gains_it_without_losing_a_row() {
        // The deployed shape: a profile table written before the journey block
        // existed. SQLite adds a column but cannot alter one, so this is an
        // ADD COLUMN rather than the table rebuild trips needed — and the default
        // `{{}}` has to deserialize into usable weights or every existing row
        // would fail to read after the upgrade.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE traveler_profiles (
                id TEXT PRIMARY KEY, hard TEXT NOT NULL, soft TEXT NOT NULL,
                interests TEXT NOT NULL, pace TEXT NOT NULL, anchors TEXT NOT NULL,
                basis TEXT NOT NULL, revision INTEGER NOT NULL, updated_at TEXT NOT NULL
            );
            INSERT INTO traveler_profiles
                VALUES ('default', '{}', '{}', '[]', '\"balanced\"', '[]', '{}', 1, 'then');",
        )
        .unwrap();

        TravelerStore::add_journey_column(&conn, "traveler").unwrap();

        let (journey, revision): (String, u32) = conn
            .query_row(
                "SELECT journey, revision FROM traveler_profiles WHERE id = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(revision, 1, "the row survived the migration");
        assert_eq!(journey, "{}");
        let parsed: crate::model::JourneyWeights = serde_json::from_str(&journey).unwrap();
        assert!(
            parsed.is_normalised(),
            "an existing row must read back as usable weights, not as a broken one"
        );

        // Idempotent: a second call sees the column and does nothing.
        TravelerStore::add_journey_column(&conn, "traveler").unwrap();
    }

    #[test]
    fn a_prefix_that_could_carry_sql_is_refused() {
        for bad in ["", "Places", "places; DROP TABLE x", "places-1", "pla ces"] {
            assert!(
                validate_prefix(bad).is_err(),
                "{bad:?} should not be a usable prefix"
            );
        }
        for good in ["traveler", "traveler_2", "t1"] {
            assert!(validate_prefix(good).is_ok(), "{good:?} should be usable");
        }
    }
}

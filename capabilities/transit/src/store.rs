//! Persistence for `transit_trips`/`transit_trip_legs` -- the "own
//! trips/trip_legs tables" half of Phase 2 (defined in
//! `capabilities/store/README.md`) --
//! plus `transit_trip_sessions` (Phase 3: fuzzy/triggered trip-search
//! sessions). Same shared file as every other store-owning capability, under
//! this one's table prefix: PRD Q45 (2026-08-27) retired Postgres, and the
//! schema-per-capability convention became a prefix-per-capability one so the
//! cross-capability joins it existed for survive. A recorded journey
//! lands in two places on an "auto" (adapter-driven) run: here, as the
//! detailed structured record; and in `scouting.opportunities` (via
//! `scouting::adapters::transit_fare`), as the scored/dismissable view. A
//! "manual" (`transit search`/`split` CLI) run only writes here -- there's
//! no adapter/pipeline involved in a one-shot CLI query. A "session" run
//! (`transit plan`) writes here AND into a `trip_sessions` row it owns --
//! different code path from the background scan (correlation driving query
//! #2 vs #1), same underlying store.
//!
//! Phase 3 settled two deliberately-deferred column questions:
//! `candidate_destinations` and
//! `date_window` are now real columns, but on the *session* row
//! (`trip_sessions.candidates` / `date_start`/`date_end`), not on `trips` --
//! a trip is one concrete journey, the candidate set and the date window
//! describe the user-intent session that found it, not the journey itself.
//! `trips` gains a nullable `session_id` FK so a session-scoped journey is
//! traceable back to the session that surfaced it, but a manual/auto trip
//! keeps `session_id = NULL` (the prior shape unchanged). `trips.status`
//! still exists (from Phase 2's schema sketch); nothing reads/sets it
//! back yet -- same honest "scaffolding only" gap
//! `scouting::store`'s `source_state.cursor` already carries.

use std::path::Path;

use crate::travel::Journey;
use rusqlite::{params, Connection, OptionalExtension, Row};
use sjel_store::QueryAll;

pub type TripWithLegs = (TripRow, Vec<TripLegRow>);

pub struct TransitStore {
    /// Shared with every other store in this process on the same file, so
    /// opening one is a checkout rather than an open.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `transit` here means `transit_trips`, `transit_trip_legs` and
    /// `transit_trip_sessions`.
    prefix: String,
}

impl TransitStore {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "transit")
    }

    /// `prefix` is always either the literal `"transit"` (production, via
    /// `open()`) or a test-generated name (see `db_tests`) -- never user input.
    /// See `scouting::store`'s identical note for why `format!`-built DDL is
    /// safe specifically in that narrow case.
    fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/sjel-store/README.md has why.
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::init_schema(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// A `Result` where this used to be `self.conn.lock().unwrap()`: that unwrap
    /// could only fail on a poisoned mutex, whereas a checkout can genuinely fail
    /// when the file is unreachable or every connection is busy.
    fn conn(&self) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The tables as they are, not the history that produced them.
    ///
    /// Phase 3 widened the `trigger_reason` CHECK to allow `'session'` with a
    /// DROP + re-ADD of the named constraint, and retrofitted `priced_at`,
    /// `session_id` and eight coordinate columns with `ADD COLUMN IF NOT
    /// EXISTS`. SQLite has neither form, and the file starts empty, so those
    /// columns are declared here and the widened CHECK is the one the
    /// `CREATE TABLE` carries -- see libs/sjel-store/README.md, "Writing a
    /// capability's DDL", for why folding is the translation.
    fn init_schema(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            -- Phase 3: the fuzzy-trip-search session itself. Declared first so the
            -- session_id reference below resolves against a table that exists.
            CREATE TABLE IF NOT EXISTS {prefix}_trip_sessions (
                id TEXT PRIMARY KEY,
                origin_eva TEXT NOT NULL,
                intent TEXT NOT NULL,
                candidates TEXT NOT NULL,
                date_start TEXT NOT NULL,
                date_end TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {prefix}_trips (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                origin_eva TEXT NOT NULL,
                destination_eva TEXT NOT NULL,
                trigger_reason TEXT NOT NULL
                    CHECK (trigger_reason IN ('manual','auto','session')),
                total_duration_minutes INTEGER,
                total_price REAL,
                created_at TEXT NOT NULL,

                -- When the stored fare was last seen.
                --
                -- The upsert refreshes total_price and deliberately leaves
                -- created_at alone, and there was no other timestamp, so `plan
                -- --show` printed a ten-week-old fare and a fresh one
                -- identically. A stored price with no observation time cannot be
                -- told apart from a current one.
                priced_at TEXT,

                -- Phase 3: session-scoped journey ownership. Nullable so a
                -- manual/auto trip keeps the Phase 2 shape.
                session_id TEXT REFERENCES {prefix}_trip_sessions(id) ON DELETE SET NULL,

                -- Endpoint coordinates. hafas.rs has parsed latitude/longitude
                -- into `Station` since the dbnav port (see `dbnav_station`), and
                -- this table dropped them on the way in (survey 2026-08-25).
                -- Nullable: the dbweb parse path carries none, and backfill by
                -- EVA is the places capability's job, not a migration here.
                origin_latitude REAL,
                origin_longitude REAL,
                destination_latitude REAL,
                destination_longitude REAL
            );

            CREATE TABLE IF NOT EXISTS {prefix}_trip_legs (
                trip_id TEXT NOT NULL REFERENCES {prefix}_trips(id) ON DELETE CASCADE,
                leg_index INTEGER NOT NULL,
                origin_eva TEXT NOT NULL,
                origin_name TEXT NOT NULL,
                destination_eva TEXT NOT NULL,
                destination_name TEXT NOT NULL,
                departure_time TEXT NOT NULL,
                arrival_time TEXT NOT NULL,
                train_name TEXT NOT NULL,
                train_number TEXT NOT NULL,
                train_category TEXT NOT NULL,
                platform TEXT,
                -- SQLite has no boolean type; rusqlite writes 0/1 here and reads
                -- a `bool` back out of it.
                is_regional INTEGER NOT NULL,
                origin_latitude REAL,
                origin_longitude REAL,
                destination_latitude REAL,
                destination_longitude REAL,
                PRIMARY KEY (trip_id, leg_index)
            );

            CREATE INDEX IF NOT EXISTS {prefix}_idx_trips_status ON {prefix}_trips(status);
            CREATE INDEX IF NOT EXISTS {prefix}_idx_trips_session ON {prefix}_trips(session_id);
            "
        ))?;
        Ok(())
    }

    pub const VALID_TRIGGER_REASONS: [&'static str; 3] = ["manual", "auto", "session"];

    /// Records a found journey: upserts one `trips` row (keyed on the
    /// journey's own HAFAS-assigned id, same trust-the-source-id pattern
    /// `scouting::store` uses for `Opportunity.id`) and replaces its
    /// `trip_legs` wholesale inside one transaction -- a re-recorded journey
    /// (same id, re-run search) gets a clean, consistent leg set rather than
    /// a delete/insert race outside a transaction. Returns `Ok(true)` if
    /// this was a new trip id, `Ok(false)` if it updated an existing one.
    /// `session_id` is `None` for a manual/auto trip (Phase 2 shape); a
    /// session-scoped recording passes `Some(id)`.
    pub fn record_journey(
        &self,
        journey: &Journey,
        origin_eva: &str,
        destination_eva: &str,
        trigger_reason: &str,
        session_id: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::VALID_TRIGGER_REASONS.contains(&trigger_reason) {
            return Err(format!(
                "invalid trigger_reason '{trigger_reason}' -- must be one of: {}",
                Self::VALID_TRIGGER_REASONS.join(", ")
            )
            .into());
        }
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;

        let existing: Option<String> = tx
            .query_row(
                &format!(
                    "SELECT id FROM {prefix}_trips WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&journey.id],
                |row| row.get(0),
            )
            .optional()?;
        let is_new = existing.is_none();

        // `session_id` is only carried in the INSERT's value list -- the
        // ON CONFLICT branch deliberately does NOT touch it. A journey re-
        // found by a *different* session would otherwise retroactively
        // re-tag a prior session's trip; we record the destination/
        // duration/price refresh but leave ownership stable. Same
        // "don't clobber a prior decision on re-fetch" principle as
        // `scouting::store`'s status-preserving `ON CONFLICT`.
        tx.execute(
            &format!(
                "INSERT INTO {prefix}_trips (id, origin_eva, destination_eva, trigger_reason,
                    total_duration_minutes, total_price, created_at, session_id, priced_at,
                    origin_latitude, origin_longitude, destination_latitude, destination_longitude)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,
                    CASE WHEN ?6 IS NULL THEN NULL ELSE ?7 END,
                    ?9,?10,?11,?12)
                ON CONFLICT (id) DO UPDATE SET
                    origin_eva = excluded.origin_eva,
                    destination_eva = excluded.destination_eva,
                    trigger_reason = excluded.trigger_reason,
                    total_duration_minutes = excluded.total_duration_minutes,
                    total_price = excluded.total_price,
                    -- Coordinates follow the eva/leg refresh unconditionally --
                    -- unlike priced_at below there is no keep-the-old-value
                    -- branch, because trip_legs is replaced wholesale in this
                    -- same transaction and a trip row keeping stale coordinates
                    -- next to freshly replaced legs would let the two disagree.
                    origin_latitude = excluded.origin_latitude,
                    origin_longitude = excluded.origin_longitude,
                    destination_latitude = excluded.destination_latitude,
                    destination_longitude = excluded.destination_longitude,
                    -- Moves only when a price actually arrived. A refresh that
                    -- came back without a fare must not restamp the old one as
                    -- freshly observed.
                    priced_at = CASE
                        WHEN excluded.total_price IS NULL THEN {prefix}_trips.priced_at
                        ELSE excluded.priced_at
                    END",
                prefix = self.prefix
            ),
            params![
                &journey.id,
                &origin_eva,
                &destination_eva,
                &trigger_reason,
                &(journey.total_duration_minutes as i32),
                &journey.total_price,
                &chrono_now(),
                &session_id,
                // Both hafas.rs parse paths set start/end_station from the
                // first/last leg, so these are the journey's own endpoint
                // coordinates -- in scope right here, no caller plumbing.
                &journey.start_station.latitude,
                &journey.start_station.longitude,
                &journey.end_station.latitude,
                &journey.end_station.longitude,
            ],
        )?;

        insert_legs(&tx, &self.prefix, &journey.id, &journey.legs)?;

        tx.commit()?;
        Ok(is_new)
    }

    /// Reads a trip back with its legs, in leg order -- verification/test
    /// helper; no CLI command consumes this yet (see module doc).
    pub fn get_trip(&self, id: &str) -> Result<Option<TripWithLegs>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let trip = conn
            .query_row(
                &format!(
                    "SELECT {TRIP_COLUMNS} FROM {prefix}_trips WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                row_to_trip,
            )
            .optional()?;
        let Some(trip) = trip else { return Ok(None) };
        let legs = self.legs_of(&conn, id)?;
        Ok(Some((trip, legs)))
    }

    /// One trip's legs, in leg order. Split out because `get_trip` and
    /// `list_trips` read them identically and the column order is positional.
    fn legs_of(
        &self,
        conn: &sjel_store::PooledClient,
        trip_id: &str,
    ) -> Result<Vec<TripLegRow>, Box<dyn std::error::Error>> {
        Ok(conn.query_all(
            &format!(
                "SELECT {LEG_COLUMNS}
                 FROM {prefix}_trip_legs WHERE trip_id = ?1 ORDER BY leg_index",
                prefix = self.prefix
            ),
            params![&trip_id],
            row_to_leg,
        )?)
    }

    pub fn count(&self) -> Result<i64, Box<dyn std::error::Error>> {
        self.count_trips(None)
    }

    /// How many trips match the same filter `list_trips` takes, so a bounded
    /// read can say how much it left behind instead of implying it returned
    /// everything.
    pub fn count_trips(&self, session_id: Option<&str>) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {prefix}_trips WHERE (?1 IS NULL OR session_id = ?1)",
                prefix = self.prefix
            ),
            params![&session_id],
            |row| row.get(0),
        )?)
    }

    // ── Phase 3: fuzzy/triggered trip-search sessions ───────────────────
    //
    // A session owns one user-intent ("Valencia or Copenhagen, in September,
    // open"), its resolved candidate destination set, and a date window. The
    // `transit plan` CLI builds one, fans `search_connections` out across
    // (candidate x sampled date) and records every found journey into the
    // existing `trips`/`trip_legs` tables tagged `trigger_reason = "session"`
    // with `session_id` set back to this row. Same store, different query
    // path than the background scan -- see `capabilities/store/README.md`
    // driving query #2 and the module doc above.

    /// Upserts a session row keyed on `id`. `id` is expected to be the
    /// stable id from `stable_session_id` (origin + sorted candidates +
    /// date window + intent hash) so re-running the *same* plan updates
    /// the same session with fresh fare data instead of creating a new
    /// one every time -- the entire point of "triggered" vs "one-shot".
    /// Returns `Ok(true)` if this was a new session.
    pub fn upsert_session(
        &self,
        id: &str,
        origin_eva: &str,
        intent: &str,
        candidates: &[CandidateDest],
        date_start: &str,
        date_end: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let cands_json = serde_json::to_string(candidates)?;
        let conn = self.conn()?;
        let existing: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT id FROM {prefix}_trip_sessions WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                |row| row.get(0),
            )
            .optional()?;
        let is_new = existing.is_none();
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_trip_sessions
                    (id, origin_eva, intent, candidates, date_start, date_end, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT (id) DO UPDATE SET
                    intent = excluded.intent,
                    candidates = excluded.candidates,
                    date_start = excluded.date_start,
                    date_end = excluded.date_end",
                prefix = self.prefix
            ),
            params![
                &id,
                &origin_eva,
                &intent,
                &cands_json,
                &date_start,
                &date_end,
                &chrono_now(),
            ],
        )?;
        Ok(is_new)
    }

    /// Reads a session back. `candidates` is JSON-deserialized from the
    /// stored TEXT column. Returns `Ok(None)` if the id isn't a session.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let session = conn
            .query_row(
                &format!(
                    "SELECT id, origin_eva, intent, candidates, date_start, date_end, status, created_at
                     FROM {prefix}_trip_sessions WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                |row| {
                    Ok(SessionRow {
                        id: row.get(0)?,
                        origin_eva: row.get(1)?,
                        intent: row.get(2)?,
                        // Deliberately not `json_column`: a session written by a
                        // buggy or partial run must not brick `get_session`.
                        candidates: serde_json::from_str(&row.get::<_, String>(3)?)
                            .unwrap_or_default(),
                        date_start: row.get(4)?,
                        date_end: row.get(5)?,
                        status: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    /// Lists every trip owned by a session, ranked cheapest-first (NULL
    /// prices last), then by shortest duration as a tiebreaker -- the
    /// ranking a "I feel like a trip, what's cheap?" query actually wants.
    /// Each entry carries its leg set, same shape as `get_trip` returns.
    pub fn list_session_trips(
        &self,
        session_id: &str,
    ) -> Result<Vec<TripWithLegs>, Box<dyn std::error::Error>> {
        self.list_trips(Some(session_id), None)
    }

    /// The same read with the session filter made optional, so a caller that
    /// wants "every trip" is not forced to already know a session id.
    ///
    /// `session_id = None` returns manual, `transit_fare`-adapter and session
    /// trips together; `limit = None` returns all of them, which keeps the
    /// unbounded read a parameter value rather than a second SQL string to keep
    /// in sync. `COALESCE(?2, -1)` is what carries that across the move off
    /// Postgres: Postgres read `LIMIT NULL` as `LIMIT ALL`, SQLite raises
    /// `datatype mismatch` on it, and a negative limit is SQLite's no-bound.
    ///
    /// This exists because `GET /api/trips` served `{"count": n, "trips": []}`
    /// with HTTP 200 while the rows sat right here: to a human reading the
    /// handler's TODO that was honest scaffolding, but to a programmatic caller
    /// it was indistinguishable from "there are no trips".
    pub fn list_trips(
        &self,
        session_id: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<TripWithLegs>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let trips = conn.query_all(
            &format!(
                "SELECT {TRIP_COLUMNS}
                 FROM {prefix}_trips
                 WHERE (?1 IS NULL OR session_id = ?1)
                 ORDER BY total_price ASC NULLS LAST, total_duration_minutes ASC NULLS LAST
                 LIMIT COALESCE(?2, -1)",
                prefix = self.prefix
            ),
            params![&session_id, &limit],
            row_to_trip,
        )?;
        let mut out = Vec::with_capacity(trips.len());
        for trip in trips {
            let legs = self.legs_of(&conn, &trip.id)?;
            out.push((trip, legs));
        }
        Ok(out)
    }
}

/// Read back positionally by `row_to_trip`, so the order is the contract.
const TRIP_COLUMNS: &str = "id, status, origin_eva, destination_eva, trigger_reason,
     total_duration_minutes, total_price, created_at, session_id, priced_at,
     origin_latitude, origin_longitude, destination_latitude, destination_longitude";

const LEG_COLUMNS: &str = "origin_eva, origin_name, destination_eva, destination_name,
     departure_time, arrival_time, train_name, train_number, train_category,
     platform, is_regional,
     origin_latitude, origin_longitude, destination_latitude, destination_longitude";

fn row_to_trip(row: &Row) -> rusqlite::Result<TripRow> {
    Ok(TripRow {
        id: row.get(0)?,
        status: row.get(1)?,
        origin_eva: row.get(2)?,
        destination_eva: row.get(3)?,
        trigger_reason: row.get(4)?,
        total_duration_minutes: row.get::<_, Option<i64>>(5)?.map(|n| n as u32),
        total_price: row.get(6)?,
        created_at: row.get(7)?,
        session_id: row.get(8)?,
        priced_at: row.get(9)?,
        origin_latitude: row.get(10)?,
        origin_longitude: row.get(11)?,
        destination_latitude: row.get(12)?,
        destination_longitude: row.get(13)?,
    })
}

fn row_to_leg(row: &Row) -> rusqlite::Result<TripLegRow> {
    Ok(TripLegRow {
        origin_eva: row.get(0)?,
        origin_name: row.get(1)?,
        destination_eva: row.get(2)?,
        destination_name: row.get(3)?,
        departure_time: row.get(4)?,
        arrival_time: row.get(5)?,
        train_name: row.get(6)?,
        train_number: row.get(7)?,
        train_category: row.get(8)?,
        platform: row.get(9)?,
        is_regional: row.get(10)?,
        origin_latitude: row.get(11)?,
        origin_longitude: row.get(12)?,
        destination_latitude: row.get(13)?,
        destination_longitude: row.get(14)?,
    })
}

/// Shared `trip_legs` insertion -- used by every `record_journey*` path so
/// the leg-replace (delete-then-reinsert-wholesale) discipline is identical
/// whether the recording came from a manual CLI call, the `transit_fare`
/// adapter, or a session run. Caller holds the live transaction.
fn insert_legs(
    tx: &rusqlite::Transaction,
    prefix: &str,
    journey_id: &str,
    legs: &[crate::travel::Leg],
) -> Result<(), Box<dyn std::error::Error>> {
    tx.execute(
        &format!("DELETE FROM {prefix}_trip_legs WHERE trip_id = ?1"),
        params![&journey_id],
    )?;
    for (i, leg) in legs.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO {prefix}_trip_legs (trip_id, leg_index, origin_eva, origin_name,
                    destination_eva, destination_name, departure_time, arrival_time,
                    train_name, train_number, train_category, platform, is_regional,
                    origin_latitude, origin_longitude, destination_latitude, destination_longitude)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                prefix = prefix
            ),
            params![
                &journey_id,
                &(i as i64),
                &leg.origin.id,
                &leg.origin.name,
                &leg.destination.id,
                &leg.destination.name,
                &leg.departure_time,
                &leg.arrival_time,
                &leg.train_name,
                &leg.train_number,
                &leg.train_category,
                &leg.platform,
                &leg.is_regional,
                &leg.origin.latitude,
                &leg.origin.longitude,
                &leg.destination.latitude,
                &leg.destination.longitude,
            ],
        )?;
    }
    Ok(())
}

/// A trip's stored timestamps are unix seconds as text, not `sjel_store::NOW`.
///
/// Deliberately left alone by the SQLite move: `created_at` and `priced_at` are
/// written from Rust rather than by the statement, and changing what they hold
/// would have made every row written before today unreadable next to one written
/// after it. See PRD Q45 for the canonical format the columns that DO use
/// `now()` carry.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// A resolved candidate destination for a session -- the EVA id plus the
/// human-readable station name `HafasClient::suggest_stations` returned for
/// it. Stored serialized as JSON in `trip_sessions.candidates`; round-trips
/// via `serde_json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CandidateDest {
    pub eva: String,
    pub name: String,
}

/// One fuzzy-trip-search session, read back from the store. `candidates`
/// has been JSON-parsed; an unparseable column yields an empty Vec rather
/// than an error (a session created by a buggy/partial run shouldn't
/// brick `get_session`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: String,
    pub origin_eva: String,
    pub intent: String,
    pub candidates: Vec<CandidateDest>,
    pub date_start: String,
    pub date_end: String,
    pub status: String,
    pub created_at: String,
}

/// Deterministic session id from the inputs that define the session's
/// shape: origin, the *sorted* candidate destination set, the date window,
/// and the (trimmed, lowercased) intent text. Same inputs -> same id, so
/// re-running `transit plan --destinations Valencia,Copenhagen --date-from
/// 2026-09-01 --date-to 2026-09-30` updates the *same* session row with
/// fresh fare data rather than accumulating duplicate sessions. Same
/// "stable id from identity-bearing inputs" pattern `Opportunity::id` and
/// `Opportunity::fingerprint` already use; no random/uuid dependency
/// needed (and we wouldn't want one here -- a stable id is the feature,
/// not a bug).
pub fn stable_session_id(
    origin_eva: &str,
    candidates: &[CandidateDest],
    date_start: &str,
    date_end: &str,
    intent: &str,
) -> String {
    let mut evas: Vec<&str> = candidates.iter().map(|c| c.eva.as_str()).collect();
    evas.sort_unstable();
    evas.dedup();
    let stem = format!(
        "{origin_eva}|{}|{date_start}|{date_end}|{}",
        evas.join(","),
        intent.trim().to_lowercase()
    );
    let mut h = 0u64;
    for b in stem.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*b as u64);
    }
    format!("trip:session:{:016x}", h)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TripRow {
    pub id: String,
    pub status: String,
    pub origin_eva: String,
    pub destination_eva: String,
    pub trigger_reason: String,
    pub total_duration_minutes: Option<u32>,
    pub total_price: Option<f64>,
    pub created_at: String,
    /// Phase 3: which `trip_sessions` row owns this trip, if any -- `None`
    /// for a manual/`transit_fare`-adapter trip (Phase 2 shape).
    pub session_id: Option<String>,
    /// When this fare was last actually seen. `None` for a row written before
    /// the column existed, and for a trip stored without a price at all.
    pub priced_at: Option<String>,
    /// Endpoint coordinates, from the journey's own first-leg origin /
    /// last-leg destination `Station` (both hafas.rs parse paths set
    /// `start_station`/`end_station` that way). `None` for a row written
    /// before the columns existed and for the dbweb parse path, which
    /// carries no coordinates -- absent, never guessed.
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub destination_latitude: Option<f64>,
    pub destination_longitude: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TripLegRow {
    pub origin_eva: String,
    pub origin_name: String,
    pub destination_eva: String,
    pub destination_name: String,
    pub departure_time: String,
    pub arrival_time: String,
    pub train_name: String,
    pub train_number: String,
    pub train_category: String,
    pub platform: Option<String>,
    pub is_regional: bool,
    /// `Station::latitude`/`longitude` of this leg's endpoints, persisted
    /// instead of dropped (survey 2026-08-25). `None` under the same
    /// conditions as the trip-level columns above.
    pub origin_latitude: Option<f64>,
    pub origin_longitude: Option<f64>,
    pub destination_latitude: Option<f64>,
    pub destination_longitude: Option<f64>,
}

#[cfg(test)]
mod unit_tests {
    use super::{stable_session_id, CandidateDest};

    #[test]
    fn stable_session_id_is_deterministic_and_input_orderindependent() {
        let cands = vec![
            CandidateDest {
                eva: "8300003".into(),
                name: "Barcelona".into(),
            },
            CandidateDest {
                eva: "8600206".into(),
                name: "Valencia".into(),
            },
        ];
        let id_a = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "Valencia or Barcelona",
        );
        // Candidate order should NOT change the id (the helper sorts first).
        let reversed = vec![
            CandidateDest {
                eva: "8600206".into(),
                name: "Valencia".into(),
            },
            CandidateDest {
                eva: "8300003".into(),
                name: "Barcelona".into(),
            },
        ];
        let id_b = stable_session_id(
            "8000044",
            &reversed,
            "2026-09-01",
            "2026-09-30",
            "Valencia or Barcelona",
        );
        assert_eq!(id_a, id_b, "candidate order must not change the session id");
        // Intent is trimmed + lowercased so whitespace/case differences collapse.
        let id_c = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "  VALENCIA or barcelona  ",
        );
        assert_eq!(
            id_a, id_c,
            "intent should be trimmed+lowercased before hashing"
        );
        // A different window or origin or candidate set changes the id.
        assert_ne!(
            id_a,
            stable_session_id(
                "8000044",
                &cands,
                "2026-10-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        assert_ne!(
            id_a,
            stable_session_id(
                "8000050",
                &cands,
                "2026-09-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        let only_val = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        assert_ne!(
            id_a,
            stable_session_id(
                "8000044",
                &only_val,
                "2026-09-01",
                "2026-09-30",
                "Valencia or Barcelona"
            )
        );
        assert!(
            id_a.starts_with("trip:session:"),
            "session ids should be namespaced"
        );
    }
}

/// Database-backed; named for the selector CI splits on — see
/// `capabilities/scouting/src/store.rs` for why the name is the contract. It
/// was `postgres_tests` until PRD Q45.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::travel::{Leg, Station};

    /// A file per test, in a directory this process owns.
    ///
    /// This replaces a schema-per-test guard that dropped a Postgres schema on
    /// unwind, and the leak it existed for cannot happen here: four abandoned
    /// schemas were sitting in the shared database on 2026-07-28, from two
    /// long-finished processes, and every one would have gone into the next
    /// pg_dumpall. A temp file is not in the backup set and is not shared.
    fn open_test_store(name: &str) -> TransitStore {
        let dir = std::env::temp_dir().join(format!("transit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        // The directory is named by pid, and a pid is recycled eventually. A
        // previous run's rows must not arrive in this one.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        TransitStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    fn mk_journey(id: &str) -> Journey {
        // Coordinates carried so the round-trip test can prove they survive
        // persistence -- the dbnav parse path fills them (hafas.rs
        // `dbnav_station`), and the store used to drop them. Values are the
        // same fixtures hafas.rs's own tests use for these stations.
        let bonn = Station {
            id: "8000044".into(),
            name: "Bonn Hbf".into(),
            latitude: Some(50.731964),
            longitude: Some(7.096678),
        };
        let berlin = Station {
            id: "8098160".into(),
            name: "Berlin Hbf".into(),
            latitude: Some(52.52585),
            longitude: Some(13.368892),
        };
        Journey {
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
            id: id.into(),
            start_station: bonn.clone(),
            end_station: berlin.clone(),
            legs: vec![Leg {
                on_time_probability: None,
                origin: bonn,
                destination: berlin,
                departure_time: "2026-08-01T08:00:00".into(),
                arrival_time: "2026-08-01T12:00:00".into(),
                departure_utc: None,
                arrival_utc: None,
                train_name: "ICE 691".into(),
                train_number: "691".into(),
                train_category: "ICE".into(),
                platform: Some("3".into()),
                is_regional: false,
                scheduled_departure: None,
                realtime_departure: None,
                scheduled_arrival: None,
                realtime_arrival: None,
                cancelled: false,
            }],
            total_duration_minutes: 240,
            total_price: Some(79.90),
            delay_risk_score: None,
            arrival_punctuality: None,
        }
    }

    #[test]
    fn record_journey_is_idempotent_and_round_trips() {
        let store = open_test_store("idempotent");

        let journey = mk_journey("journey:test:1");
        let is_new1 = store
            .record_journey(&journey, "8000044", "8098160", "auto", None)
            .unwrap();
        assert!(is_new1, "first record should be new");
        let is_new2 = store
            .record_journey(&journey, "8000044", "8098160", "auto", None)
            .unwrap();
        assert!(
            !is_new2,
            "re-recording the same journey id should not be new"
        );

        assert_eq!(store.count().unwrap(), 1);

        let (trip, legs) = store
            .get_trip("journey:test:1")
            .unwrap()
            .expect("trip should exist");
        assert_eq!(trip.origin_eva, "8000044");
        assert_eq!(trip.destination_eva, "8098160");
        assert_eq!(trip.trigger_reason, "auto");
        assert_eq!(
            trip.status, "new",
            "freshly recorded trip defaults to 'new'"
        );
        assert_eq!(trip.total_duration_minutes, Some(240));
        assert!((trip.total_price.unwrap() - 79.90).abs() < 1e-9);

        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].train_number, "691");
        assert_eq!(legs[0].platform.as_deref(), Some("3"));
        assert!(!legs[0].is_regional);

        // Coordinates round-trip on both tables -- they were parsed into
        // `Station` and then dropped at this exact boundary before.
        assert_eq!(trip.origin_latitude, Some(50.731964));
        assert_eq!(trip.origin_longitude, Some(7.096678));
        assert_eq!(trip.destination_latitude, Some(52.52585));
        assert_eq!(trip.destination_longitude, Some(13.368892));
        assert_eq!(legs[0].origin_latitude, Some(50.731964));
        assert_eq!(legs[0].origin_longitude, Some(7.096678));
        assert_eq!(legs[0].destination_latitude, Some(52.52585));
        assert_eq!(legs[0].destination_longitude, Some(13.368892));
    }

    #[test]
    fn record_journey_replaces_legs_on_update() {
        let store = open_test_store("replace_legs");

        let mut journey = mk_journey("journey:test:2");
        store
            .record_journey(&journey, "8000044", "8098160", "manual", None)
            .unwrap();

        // Re-recording with a different leg set should leave exactly the new
        // legs, not the old ones appended alongside them.
        journey.legs.push(journey.legs[0].clone());
        journey.total_duration_minutes = 300;
        // This refresh carries no destination coordinates (the dbweb parse
        // path never does). The trip row must follow the refresh -- legs are
        // replaced wholesale in the same transaction, and a kept-stale
        // coordinate next to fresh legs would let the two disagree (see the
        // ON CONFLICT comment in record_journey).
        journey.end_station.latitude = None;
        journey.end_station.longitude = None;
        store
            .record_journey(&journey, "8000044", "8098160", "manual", None)
            .unwrap();

        let (trip, legs) = store.get_trip("journey:test:2").unwrap().unwrap();
        assert_eq!(trip.total_duration_minutes, Some(300));
        assert_eq!(
            legs.len(),
            2,
            "leg set should be fully replaced, not appended to"
        );
        assert_eq!(
            trip.origin_latitude,
            Some(50.731964),
            "origin coordinates refresh from the re-recorded journey"
        );
        assert_eq!(
            trip.destination_latitude, None,
            "a refresh without coordinates must not keep the stale ones"
        );
        assert_eq!(trip.destination_longitude, None);
    }

    #[test]
    fn record_journey_rejects_invalid_trigger_reason() {
        let store = open_test_store("invalid_trigger");
        let journey = mk_journey("journey:test:3");
        let result = store.record_journey(&journey, "8000044", "8098160", "scheduled", None);
        assert!(
            result.is_err(),
            "an unrecognized trigger_reason must error, not silently accept"
        );
    }

    // ── Phase 3: trip sessions ───────────────────────────────────────────

    fn mk_session_journey(
        id: &str,
        dest_eva: &str,
        dest_name: &str,
        price: Option<f64>,
        dur: u32,
    ) -> Journey {
        let bonn = Station {
            id: "8000044".into(),
            name: "Bonn Hbf".into(),
            latitude: None,
            longitude: None,
        };
        let dest = Station {
            id: dest_eva.into(),
            name: dest_name.into(),
            latitude: None,
            longitude: None,
        };
        Journey {
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
            id: id.into(),
            start_station: bonn.clone(),
            end_station: dest.clone(),
            legs: vec![Leg {
                on_time_probability: None,
                origin: bonn,
                destination: dest,
                departure_time: "2026-09-10T08:00:00".into(),
                arrival_time: "2026-09-10T14:00:00".into(),
                departure_utc: None,
                arrival_utc: None,
                train_name: "ICE 691".into(),
                train_number: "691".into(),
                train_category: "ICE".into(),
                platform: Some("3".into()),
                is_regional: false,
                scheduled_departure: None,
                realtime_departure: None,
                scheduled_arrival: None,
                realtime_arrival: None,
                cancelled: false,
            }],
            total_duration_minutes: dur,
            total_price: price,
            delay_risk_score: None,
            arrival_punctuality: None,
        }
    }

    #[test]
    fn upsert_session_is_idempotent_and_round_trips() {
        let store = open_test_store("session_upsert");
        let cands = vec![
            CandidateDest {
                eva: "8600206".into(),
                name: "Valencia".into(),
            },
            CandidateDest {
                eva: "8300003".into(),
                name: "Barcelona".into(),
            },
        ];
        let id = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "Valencia or Barcelona",
        );
        let is_new1 = store
            .upsert_session(
                &id,
                "8000044",
                "Valencia or Barcelona",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();
        assert!(is_new1, "first upsert of a session should be new");
        // Tweaked intent (same shape) should UPDATE, not create a second row.
        let is_new2 = store
            .upsert_session(
                &id,
                "8000044",
                "Valencia or Barcelona, open to nearby",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();
        assert!(
            !is_new2,
            "re-upserting the same session id should update, not insert"
        );

        let s = store
            .get_session(&id)
            .unwrap()
            .expect("session should exist");
        assert_eq!(s.origin_eva, "8000044");
        assert_eq!(s.intent, "Valencia or Barcelona, open to nearby");
        assert_eq!(s.date_start, "2026-09-01");
        assert_eq!(s.date_end, "2026-09-30");
        assert_eq!(s.status, "new");
        assert_eq!(s.candidates.len(), 2);
        assert!(s
            .candidates
            .iter()
            .any(|c| c.eva == "8600206" && c.name == "Valencia"));
        assert!(s
            .candidates
            .iter()
            .any(|c| c.eva == "8300003" && c.name == "Barcelona"));
    }

    #[test]
    fn session_journeys_are_tagged_owned_and_ranked_by_price() {
        let store = open_test_store("session_rank");
        let cands = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        let sid = stable_session_id(
            "8000044",
            &cands,
            "2026-09-01",
            "2026-09-30",
            "Valencia in Sept",
        );
        store
            .upsert_session(
                &sid,
                "8000044",
                "Valencia in Sept",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();

        // Three found journeys -- record them session-scoped, prices mixed
        // so the ranking order is provably cheapest-first, not insert order.
        store
            .record_journey(
                &mk_session_journey("j:expensive", "8600206", "Valencia", Some(129.50), 360),
                "8000044",
                "8600206",
                "session",
                Some(&sid),
            )
            .unwrap();
        store
            .record_journey(
                &mk_session_journey("j:cheap", "8600206", "Valencia", Some(49.90), 380),
                "8000044",
                "8600206",
                "session",
                Some(&sid),
            )
            .unwrap();
        store
            .record_journey(
                &mk_session_journey("j:mid", "8600206", "Valencia", Some(89.00), 340),
                "8000044",
                "8600206",
                "session",
                Some(&sid),
            )
            .unwrap();

        let trips = store.list_session_trips(&sid).unwrap();
        assert_eq!(
            trips.len(),
            3,
            "all three session journeys should be owned by the session"
        );
        // Cheapest-first.
        assert_eq!(trips[0].0.id, "j:cheap");
        assert!((trips[0].0.total_price.unwrap() - 49.90).abs() < 1e-9);
        assert_eq!(trips[1].0.id, "j:mid");
        assert_eq!(trips[2].0.id, "j:expensive");
        // Each carries trigger_reason='session' and the session_id back.
        assert_eq!(trips[0].0.trigger_reason, "session");
        assert_eq!(trips[0].0.session_id.as_deref(), Some(sid.as_str()));
        // Legs round-trip too.
        assert_eq!(trips[0].1.len(), 1);
        assert_eq!(trips[0].1[0].destination_name, "Valencia");
        // A journey whose stations carry no coordinates (dbweb parse path,
        // and every row written before the columns existed) reads back as
        // None -- absent, never zero.
        assert_eq!(trips[0].0.origin_latitude, None);
        assert_eq!(trips[0].0.destination_longitude, None);
        assert_eq!(trips[0].1[0].origin_latitude, None);
        assert_eq!(trips[0].1[0].destination_longitude, None);
    }

    /// A refreshed fare restamps priced_at; a refresh that came back without one
    /// must not, or an old price would read as freshly observed.
    #[test]
    fn a_priceless_refresh_does_not_restamp_an_observed_fare() {
        let store = open_test_store("priced_at");
        let priced = mk_journey("j:priced");
        store
            .record_journey(&priced, "8000044", "8098160", "auto", None)
            .unwrap();
        let (row, _) = store.get_trip("j:priced").unwrap().unwrap();
        let first = row.priced_at.clone().expect("a priced trip records when");
        assert!(row.total_price.is_some());

        // Same journey, no fare this time.
        let mut priceless = priced.clone();
        priceless.total_price = None;
        store
            .record_journey(&priceless, "8000044", "8098160", "auto", None)
            .unwrap();
        let (row, _) = store.get_trip("j:priced").unwrap().unwrap();
        assert_eq!(
            row.priced_at.as_deref(),
            Some(first.as_str()),
            "a refresh with no fare must leave the observation time where it was"
        );
    }

    /// `GET /api/trips` used to answer `{"count": n, "trips": []}` with HTTP 200
    /// because the store had no unfiltered read. This is that read: it must see
    /// manual trips (`session_id IS NULL`) and session trips together, keep the
    /// cheapest-first ranking across both, and honour a limit while `count_trips`
    /// still reports the full total, so `truncated` can be computed honestly.
    #[test]
    fn list_trips_sees_manual_and_session_trips_and_bounds_the_read() {
        let store = open_test_store("list_trips_all");
        let cands = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia");
        store
            .upsert_session(
                &sid,
                "8000044",
                "Valencia",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();
        // One session trip and one manual trip: the manual one is what the old
        // handler could never have returned even if it had tried a session read.
        store
            .record_journey(
                &mk_session_journey("j:session", "8600206", "Valencia", Some(89.00), 340),
                "8000044",
                "8600206",
                "session",
                Some(&sid),
            )
            .unwrap();
        store
            .record_journey(&mk_journey("j:manual"), "8000044", "8098160", "auto", None)
            .unwrap();

        let all = store.list_trips(None, None).unwrap();
        assert_eq!(all.len(), 2, "unfiltered read must see both trips");
        // 79.90 (manual) before 89.00 (session): one ranking across both origins.
        assert_eq!(all[0].0.id, "j:manual");
        assert_eq!(all[1].0.id, "j:session");
        assert_eq!(all[0].1.len(), 1, "legs come back with the trip");
        assert_eq!(store.count_trips(None).unwrap(), 2);

        // The session filter still narrows, and agrees with the old entry point.
        let scoped = store.list_trips(Some(&sid), None).unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0.id, "j:session");
        assert_eq!(store.count_trips(Some(&sid)).unwrap(), 1);
        assert_eq!(store.list_session_trips(&sid).unwrap(), scoped);

        // A bounded read returns the cheapest one and leaves the total intact,
        // which is what lets the handler say truncated: true rather than imply
        // it returned everything.
        let bounded = store.list_trips(None, Some(1)).unwrap();
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].0.id, "j:manual");
        assert_eq!(store.count_trips(None).unwrap(), 2);
    }

    #[test]
    fn replanning_session_refreshes_fares_without_losing_status() {
        // A re-found journey (same id, fresh price) under the SAME session:
        // the price/duration refresh, but a hand-set 'saved' status survives
        // -- same invariant scouting::store::upsert_preserves_status guards.
        let store = open_test_store("session_refresh");
        let cands = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia");
        store
            .upsert_session(
                &sid,
                "8000044",
                "Valencia",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();
        let j = mk_session_journey("j:refresh", "8600206", "Valencia", Some(70.00), 360);
        store
            .record_journey(&j, "8000044", "8600206", "session", Some(&sid))
            .unwrap();

        // Hand-mark saved out-of-band (there's no CLI for it yet -- direct SQL
        // is the test seam; same pattern scouting::store::db_tests uses to test
        // the status-preservation path before a CLI exists).
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE transit_trips SET status='saved' WHERE id='j:refresh'",
                [],
            )
            .unwrap();
        let (trip, _) = store.get_trip("j:refresh").unwrap().unwrap();
        assert_eq!(trip.status, "saved");

        // Re-plan: same journey id, cheaper fare now.
        let mut j2 = j.clone();
        j2.total_price = Some(55.00);
        store
            .record_journey(&j2, "8000044", "8600206", "session", Some(&sid))
            .unwrap();

        let (trip2, _) = store.get_trip("j:refresh").unwrap().unwrap();
        assert_eq!(trip2.status, "saved", "status must survive a fare refresh");
        assert!(
            (trip2.total_price.unwrap() - 55.00).abs() < 1e-9,
            "price should refresh"
        );
    }

    #[test]
    fn session_scope_isolates_trips_from_manual_and_auto() {
        // A manual/auto trip (session_id=None) should NOT show up in a
        // session's list even if it shares origin/destination with session
        // trips -- the session_id filter is what keeps query #2's results
        // separate from #1's background-scan results.
        let store = open_test_store("session_isolation");
        let cands = vec![CandidateDest {
            eva: "8600206".into(),
            name: "Valencia".into(),
        }];
        let sid = stable_session_id("8000044", &cands, "2026-09-01", "2026-09-30", "Valencia");
        store
            .upsert_session(
                &sid,
                "8000044",
                "Valencia",
                &cands,
                "2026-09-01",
                "2026-09-30",
            )
            .unwrap();

        store
            .record_journey(
                &mk_session_journey("j:manual", "8600206", "Valencia", Some(70.00), 360),
                "8000044",
                "8600206",
                "manual",
                None,
            )
            .unwrap();
        store
            .record_journey(
                &mk_session_journey("j:session", "8600206", "Valencia", Some(70.00), 360),
                "8000044",
                "8600206",
                "session",
                Some(&sid),
            )
            .unwrap();

        let trips = store.list_session_trips(&sid).unwrap();
        assert_eq!(
            trips.len(),
            1,
            "manual trip must not leak into the session list"
        );
        assert_eq!(trips[0].0.id, "j:session");
        assert!(store
            .get_trip("j:manual")
            .unwrap()
            .unwrap()
            .0
            .session_id
            .is_none());
    }
}

//! Persistence under the table prefix `calendar` in the one shared SQLite file
//! (PRD Q45), same pattern as trips/scouting/transit. Instants stay TEXT (naive
//! local, lexicographically ordered — see README's time model) and structured
//! fields stay JSON text, so nothing here needed a column type SQLite lacks.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::Value;
use sjel_store::QueryAll;

use crate::correlate;
use crate::date;
use crate::google;
use crate::model::{
    Commitment, Context, Entry, ExportOptIn, NewContext, NewEntry, NewRhythm, Rhythm,
    UpdateContext, UpdateEntry, UpdateRhythm,
};
use crate::rhythm;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

type StoreResult<T> = Result<T, Box<dyn std::error::Error>>;

fn generated_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}:{nanos:x}{sequence:04x}")
}

/// Orders a stored commitment column as SQL, matching `COMMITMENTS`. An unknown
/// value ranks as `possible`, the same lenient direction `Commitment` reads it.
fn commitment_rank_sql(column: &str) -> String {
    format!("(CASE {column} WHEN 'committed' THEN 2 WHEN 'planned' THEN 1 ELSE 0 END)")
}

fn now_text() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// The prefix is interpolated into DDL and every statement, so it is checked
/// rather than trusted. Production passes the literal `calendar`; only a test
/// passes anything else.
fn validate_prefix(prefix: &str) -> StoreResult<()> {
    let ok = !prefix.is_empty()
        && prefix
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && prefix.chars().next().is_some_and(|c| !c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(format!("unsafe table prefix '{prefix}'").into())
    }
}

pub struct CalendarStore {
    /// Shared with every other store in this process on the same file, so
    /// opening one is a checkout rather than an open.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `calendar` here means `calendar_entries` and its four siblings.
    prefix: String,
}

impl CalendarStore {
    pub fn open(database_path: &Path) -> StoreResult<Self> {
        Self::open_with_prefix(database_path, "calendar")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> StoreResult<Self> {
        validate_prefix(prefix)?;
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/sjel-store/README.md has why.
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
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
    fn conn(&self) -> StoreResult<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126).
    pub fn ping(&self) -> StoreResult<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the five tables, not the history that produced them.
    ///
    /// The `ALTER TABLE ... ADD COLUMN commitment` and the DROP/ADD CONSTRAINT pair
    /// that widened its CHECK are gone: SQLite has neither form, and both are already
    /// stated in `CREATE TABLE`. That is only correct because no deployed SQLite file
    /// predates this migration.
    ///
    /// `rhythms` is declared before `entries` because `entries` references it, and a
    /// batch executes in order.
    fn run_migration(conn: &Connection, prefix: &str) -> StoreResult<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_rhythms (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                location TEXT,
                byweekday TEXT NOT NULL,
                start_time TEXT,
                end_time TEXT,
                valid_from TEXT NOT NULL,
                valid_until TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {prefix}_entries (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                -- 'possible' is the deliberate default: an entry that arrived via
                -- scouting promotion was never an operator decision, and a bug in
                -- this direction hands out a free day rather than silently walling
                -- one off.
                commitment TEXT NOT NULL DEFAULT 'possible'
                    CHECK (commitment IN ('possible','planned','committed')),
                title TEXT NOT NULL,
                starts_at TEXT NOT NULL,
                ends_at TEXT NOT NULL,
                all_day INTEGER NOT NULL DEFAULT 0,
                location TEXT,
                notes TEXT,
                source TEXT NOT NULL DEFAULT 'manual',
                external_id TEXT,
                rhythm_id TEXT REFERENCES {prefix}_rhythms(id) ON DELETE SET NULL,
                payload TEXT NOT NULL DEFAULT '{{}}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            -- Index names carry the prefix too: one file is one namespace, so two
            -- prefixes sharing an index name would collide where two schemas did not.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_entries_starts
                ON {prefix}_entries(starts_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_entries_external
                ON {prefix}_entries(source, external_id)
                WHERE external_id IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_entries_rhythm_slot
                ON {prefix}_entries(rhythm_id, starts_at)
                WHERE rhythm_id IS NOT NULL;
            CREATE TABLE IF NOT EXISTS {prefix}_contexts (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                details TEXT NOT NULL DEFAULT '',
                valid_from TEXT NOT NULL,
                valid_until TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'manual',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_contexts_window
                ON {prefix}_contexts(valid_from, valid_until);
            -- Which entries already became a trip, so materialising twice
            -- returns the plan that exists instead of making a second one.
            -- Same ledger shape as google_exports, and for the same reason:
            -- the fact belongs to calendar, the plan belongs to trips, and
            -- neither store reaches into the other.
            CREATE TABLE IF NOT EXISTS {prefix}_trip_materializations (
                entry_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_entries(id) ON DELETE CASCADE,
                plan_id TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {prefix}_google_exports (
                entry_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_entries(id) ON DELETE CASCADE,
                google_calendar_id TEXT NOT NULL,
                google_event_id TEXT,
                pushed_at TEXT,
                created_at TEXT NOT NULL
            );
            ",
            prefix = prefix
        ))?;
        Ok(())
    }

    // ---- entries ----------------------------------------------------------

    /// Entries trips wrote for one plan: `source = 'trips'` and `payload.plan_id = plan_id`.
    ///
    /// A payload is inert provider evidence (`model.rs`, `Entry::payload`). This reads one key of
    /// it, and only on rows trips itself wrote, because that key is the one reference to a plan an
    /// entry carries (libs/links/ISA.md, LNK-8). Every other provider's payload stays inert.
    pub fn entries_for_plan(&self, plan_id: &str) -> StoreResult<Vec<Entry>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_entries \
                 WHERE source = 'trips' AND json_extract(payload, '$.plan_id') = ?1 \
                 ORDER BY starts_at",
                prefix = self.prefix
            ),
            params![plan_id],
            entry_from_row,
        )?)
    }

    /// Entries overlapping the day window [from, to) — both "YYYY-MM-DD",
    /// from inclusive, to exclusive. Day granularity is what the correlation
    /// layer asks in ("is 14 Aug feasible?"); finer slicing is presentation.
    pub fn list_entries(&self, from: &str, to: &str, kinds: &[String]) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let conn = self.conn()?;
        if kinds.is_empty() {
            return Ok(conn.query_all(
                &format!(
                    "SELECT * FROM {prefix}_entries \
                     WHERE starts_at < ?1 AND ends_at > ?2 \
                     ORDER BY starts_at",
                    prefix = self.prefix
                ),
                params![&to_boundary, &from_boundary],
                entry_from_row,
            )?);
        }
        // `kind = ANY($3)` had no direct translation: SQLite binds one value per
        // placeholder and has no array type. `json_each` over a JSON array keeps the
        // statement a constant with a fixed parameter count, where an `IN (?3,?4,...)`
        // built per call makes the placeholder count depend on the input.
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_entries \
                 WHERE starts_at < ?1 AND ends_at > ?2 \
                   AND kind IN (SELECT value FROM json_each(?3)) \
                 ORDER BY starts_at",
                prefix = self.prefix
            ),
            params![&to_boundary, &from_boundary, &serde_json::to_string(kinds)?],
            entry_from_row,
        )?)
    }

    /// Provider drafts are not a `kind`: the source tells us where they came
    /// from and `possible` is the deliberately non-blocking adoption state.
    /// Keeping this predicate here stops each dashboard surface from inventing
    /// its own idea of what "waiting for review" means.
    pub fn list_google_drafts(&self, from: &str, to: &str) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_entries \
                 WHERE starts_at < ?1 AND ends_at > ?2 \
                   AND source = ?3 AND commitment = ?4 \
                 ORDER BY starts_at",
                prefix = self.prefix
            ),
            params![&to_boundary, &from_boundary, google::SOURCE, "possible"],
            entry_from_row,
        )?)
    }

    /// External contributions that still need an Axon decision, except Google
    /// imports. Google has its own inbox because its refresh/ownership rules
    /// are special; all other providers share the ordinary Calendar proposal
    /// flow. A user-created `possible` block is intentionally not a proposal.
    pub fn list_external_proposals(&self, from: &str, to: &str) -> StoreResult<Vec<Entry>> {
        let (from_boundary, to_boundary) = day_window_bounds(from, to)?;
        let conn = self.conn()?;
        let proposals: Vec<Entry> = conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_entries \
                 WHERE starts_at < ?1 AND ends_at > ?2 \
                   AND commitment = ?3 AND external_id IS NOT NULL AND source <> ?4 \
                 ORDER BY starts_at",
                prefix = self.prefix
            ),
            params![&to_boundary, &from_boundary, "possible", google::SOURCE],
            entry_from_row,
        )?;
        if proposals.is_empty() {
            return Ok(proposals);
        }

        // Anything the operator has already decided on, whatever key it came in
        // under. `(source, external_id)` only dedupes a source against itself,
        // so this is the one check that can tell the inbox an event is already
        // in the calendar. The window is the caller's: a twin that overlaps a
        // proposal *outside* the requested range is not read, which costs a
        // suppression only for a proposal the window already cuts in half.
        let adopted: Vec<Entry> = conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_entries \
                 WHERE starts_at < ?1 AND ends_at > ?2 AND commitment <> ?3",
                prefix = self.prefix
            ),
            params![&to_boundary, &from_boundary, "possible"],
            entry_from_row,
        )?;
        Ok(correlate::without_already_adopted(proposals, &adopted)?)
    }

    /// The entry a provider contributed under this key, if any. Phase E's
    /// conflict policy has to read before it writes — `upsert_external_entry`
    /// overwrites unconditionally, and "Axon wins" means deciding *not* to
    /// call it. Reads the same `(source, external_id)` pair that index is
    /// unique on, so there is at most one.
    pub fn get_entry_by_external(
        &self,
        source: &str,
        external_id: &str,
    ) -> StoreResult<Option<Entry>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT * FROM {prefix}_entries WHERE source = ?1 AND external_id = ?2",
                    prefix = self.prefix
                ),
                params![&source, &external_id],
                entry_from_row,
            )
            .optional()?)
    }

    pub fn get_entry(&self, id: &str) -> StoreResult<Option<Entry>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT * FROM {prefix}_entries WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                entry_from_row,
            )
            .optional()?)
    }

    pub fn create_entry(&self, input: &NewEntry) -> StoreResult<Entry> {
        input.validate()?;
        let entry = entry_from_input(input);
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at, \
                  commitment) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                prefix = self.prefix
            ),
            params![
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                entry.all_day as i32,
                &entry.location,
                &entry.notes,
                &entry.source,
                &entry.external_id,
                &entry.rhythm_id,
                &serde_json::to_string(&entry.payload)?,
                &entry.created_at,
                &entry.updated_at,
                entry.commitment.as_str(),
            ],
        )?;
        Ok(entry)
    }

    /// Idempotent contribution boundary for Feed/Scouting/Google and future
    /// providers. A stable `(source, external_id)` updates the contributed
    /// entry in place while preserving its Axon id and original `created_at`.
    /// Manual CRUD stays on `create_entry` and never acquires upsert semantics.
    pub fn upsert_external_entry(&self, input: &NewEntry) -> StoreResult<Entry> {
        input.validate()?;
        let external_id = input
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or("external_id is required for external upsert")?;
        let mut entry = entry_from_input(input);
        entry.external_id = Some(external_id.to_string());

        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "INSERT INTO {prefix}_entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at, \
                  commitment) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) \
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL \
                 DO UPDATE SET kind=excluded.kind, title=excluded.title, \
                   starts_at=excluded.starts_at, ends_at=excluded.ends_at, \
                   all_day=excluded.all_day, location=excluded.location, \
                   notes=excluded.notes, rhythm_id=excluded.rhythm_id, \
                   payload=excluded.payload, updated_at=excluded.updated_at, \
                   commitment=CASE WHEN {rank_new} > {rank_old} \
                     THEN excluded.commitment ELSE commitment END \
                 RETURNING *",
                // `commitment` only ratchets up. A provider re-running its import
                // re-states what an event is, never lowers how committed the
                // operator is to it: once they raise a promoted event to planned
                // or committed, the next `scout --promote-calendar` must not
                // quietly hand it back down. It may raise it: a trip stage that
                // becomes `booked` must reach `committed` on the next sync, which
                // it did not before 2026-10-08.
                rank_new = commitment_rank_sql("excluded.commitment"),
                rank_old = commitment_rank_sql("commitment"),
                prefix = self.prefix
            ),
            params![
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                entry.all_day as i32,
                &entry.location,
                &entry.notes,
                &entry.source,
                &entry.external_id,
                &entry.rhythm_id,
                &serde_json::to_string(&entry.payload)?,
                &entry.created_at,
                &entry.updated_at,
                entry.commitment.as_str(),
            ],
            entry_from_row,
        )?)
    }

    /// Applies a patch. Any patch to a rhythm-linked entry detaches it from
    /// its rhythm (rhythm_id → NULL): re-materialization regenerates linked
    /// instances, so keeping the link would silently drop the override.
    pub fn update_entry(&self, id: &str, patch: &UpdateEntry) -> StoreResult<Option<Entry>> {
        let mut entry = match self.get_entry(id)? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            entry.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            entry.title = title.trim().to_string();
        }
        if let Some(starts_at) = &patch.starts_at {
            entry.starts_at = starts_at.clone();
        }
        if let Some(ends_at) = &patch.ends_at {
            entry.ends_at = ends_at.clone();
        }
        if let Some(all_day) = patch.all_day {
            entry.all_day = all_day;
        }
        if let Some(location) = &patch.location {
            entry.location = location.clone();
        }
        if let Some(notes) = &patch.notes {
            entry.notes = notes.clone();
        }
        if let Some(commitment) = patch.commitment {
            entry.commitment = commitment;
        }
        if patch != &UpdateEntry::default() && entry.rhythm_id.is_some() {
            entry.rhythm_id = None;
        }
        entry_as_new(&entry).validate()?;
        entry.updated_at = now_text();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "UPDATE {prefix}_entries SET kind=?2, title=?3, starts_at=?4, ends_at=?5, \
                 all_day=?6, location=?7, notes=?8, rhythm_id=?9, updated_at=?10, \
                 commitment=?11 WHERE id=?1",
                prefix = self.prefix
            ),
            params![
                &entry.id,
                &entry.kind,
                &entry.title,
                &entry.starts_at,
                &entry.ends_at,
                entry.all_day as i32,
                &entry.location,
                &entry.notes,
                &entry.rhythm_id,
                &entry.updated_at,
                entry.commitment.as_str(),
            ],
        )?;
        Ok(Some(entry))
    }

    pub fn delete_entry(&self, id: &str) -> StoreResult<bool> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {prefix}_entries WHERE id = ?1",
                prefix = self.prefix
            ),
            params![&id],
        )?;
        Ok(count > 0)
    }

    // ---- bounded planning context ----------------------------------------

    /// Contexts overlapping the visible date window. Their end is inclusive
    /// because they describe a human horizon ("through September"), not a
    /// schedulable half-open instant.
    pub fn list_contexts(&self, from: &str, to: &str) -> StoreResult<Vec<Context>> {
        let from_day = date::parse_date(from).ok_or("from must be YYYY-MM-DD")?;
        let to_day = date::parse_date(to).ok_or("to must be YYYY-MM-DD")?;
        if to_day < from_day {
            return Err("to must be on or after from".into());
        }
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_contexts \
                 WHERE valid_from < ?1 AND valid_until >= ?2 \
                 ORDER BY valid_from, title",
                prefix = self.prefix
            ),
            params![&to, &from],
            context_from_row,
        )?)
    }

    pub fn get_context(&self, id: &str) -> StoreResult<Option<Context>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT * FROM {prefix}_contexts WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                context_from_row,
            )
            .optional()?)
    }

    pub fn create_context(&self, input: &NewContext) -> StoreResult<Context> {
        input.validate()?;
        let context = context_from_input(input);
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_contexts \
                 (id, kind, title, details, valid_from, valid_until, source, created_at, updated_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                prefix = self.prefix
            ),
            params![
                &context.id,
                &context.kind,
                &context.title,
                &context.details,
                &context.valid_from,
                &context.valid_until,
                &context.source,
                &context.created_at,
                &context.updated_at,
            ],
        )?;
        Ok(context)
    }

    pub fn update_context(&self, id: &str, patch: &UpdateContext) -> StoreResult<Option<Context>> {
        let mut context = match self.get_context(id)? {
            Some(context) => context,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            context.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            context.title = title.trim().to_string();
        }
        if let Some(details) = &patch.details {
            context.details = details.trim().to_string();
        }
        if let Some(valid_from) = &patch.valid_from {
            context.valid_from = valid_from.clone();
        }
        if let Some(valid_until) = &patch.valid_until {
            context.valid_until = valid_until.clone();
        }
        context_as_new(&context).validate()?;
        context.updated_at = now_text();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "UPDATE {prefix}_contexts SET kind=?2, title=?3, details=?4, \
                 valid_from=?5, valid_until=?6, updated_at=?7 WHERE id=?1",
                prefix = self.prefix
            ),
            params![
                &context.id,
                &context.kind,
                &context.title,
                &context.details,
                &context.valid_from,
                &context.valid_until,
                &context.updated_at,
            ],
        )?;
        Ok(Some(context))
    }

    pub fn delete_context(&self, id: &str) -> StoreResult<bool> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {prefix}_contexts WHERE id = ?1",
                prefix = self.prefix
            ),
            params![&id],
        )?;
        Ok(count > 0)
    }

    // ---- google export ledger (Phase E) ------------------------------------

    /// Opts one entry in to Google export.
    ///
    /// A row in `google_exports` *is* the opt-in — nothing exports by default
    /// because the table starts empty, and opting out is deleting the row.
    /// The table also carries the Google event id the first push returns,
    /// which is a genuinely new fact with nowhere to live on `entries`: without
    /// it, the second push would create a second Google event instead of
    /// updating the first.
    ///
    /// Refusals (an imported entry, a rhythm instance) come from
    /// `google::export_refusal` so the rule is stated once and unit-tested
    /// without a database.
    pub fn opt_in_export(&self, entry_id: &str, calendar_id: &str) -> StoreResult<ExportOptIn> {
        let entry = self
            .get_entry(entry_id)?
            .ok_or_else(|| format!("entry {entry_id} not found"))?;
        if let Some(reason) = google::export_refusal(&entry) {
            return Err(reason.into());
        }
        if calendar_id.trim().is_empty() {
            return Err("google_calendar_id is required".into());
        }
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "INSERT INTO {prefix}_google_exports \
                 (entry_id, google_calendar_id, google_event_id, pushed_at, created_at) \
                 VALUES (?1,?2,NULL,NULL,?3) \
                 ON CONFLICT (entry_id) DO UPDATE SET \
                   google_calendar_id = excluded.google_calendar_id \
                 RETURNING *",
                prefix = self.prefix
            ),
            params![&entry.id, calendar_id.trim(), &now_text()],
            export_from_row,
        )?)
    }

    pub fn opt_out_export(&self, entry_id: &str) -> StoreResult<bool> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {prefix}_google_exports WHERE entry_id = ?1",
                prefix = self.prefix
            ),
            params![&entry_id],
        )?;
        Ok(count > 0)
    }

    /// The plan an entry already belongs to, if any.
    pub fn trip_plan_for(&self, entry_id: &str) -> StoreResult<Option<String>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT plan_id FROM {prefix}_trip_materializations WHERE entry_id = ?1",
                    prefix = self.prefix
                ),
                params![&entry_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Records that these entries became one plan. Written only after trips
    /// confirms the plan exists, so a failed call leaves no claim behind.
    pub fn record_trip_materialization(
        &self,
        entry_ids: &[String],
        plan_id: &str,
    ) -> StoreResult<()> {
        let now = now_text();
        let conn = self.conn()?;
        for entry_id in entry_ids {
            conn.execute(
                &format!(
                    "INSERT INTO {prefix}_trip_materializations (entry_id, plan_id, created_at) \
                     VALUES (?1,?2,?3) ON CONFLICT (entry_id) DO NOTHING",
                    prefix = self.prefix
                ),
                params![entry_id, &plan_id, &now],
            )?;
        }
        Ok(())
    }

    /// Drops ledger rows whose plan trips no longer has. Called only after
    /// trips has *said* the plan is gone, never on a failure to reach it.
    pub fn forget_trip_materialization(&self, plan_id: &str) -> StoreResult<u64> {
        let conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "DELETE FROM {prefix}_trip_materializations WHERE plan_id = ?1",
                prefix = self.prefix
            ),
            params![&plan_id],
        )? as u64)
    }

    pub fn list_export_optins(&self) -> StoreResult<Vec<ExportOptIn>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_google_exports ORDER BY created_at",
                prefix = self.prefix
            ),
            [],
            export_from_row,
        )?)
    }

    /// Every opted-in entry with the entry itself, for the export run. The
    /// `ON DELETE CASCADE` on `entry_id` means a deleted entry takes its
    /// opt-in with it, so this join never sees an orphan.
    ///
    /// The ledger's columns are aliased: both tables carry a `created_at`, and
    /// `row.get("created_at")` on an unaliased join takes whichever came
    /// first, which would silently stamp the entry with the opt-in's date.
    pub fn export_queue(&self) -> StoreResult<Vec<(ExportOptIn, Entry)>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT e.*, \
                        x.entry_id AS x_entry_id, \
                        x.google_calendar_id AS x_google_calendar_id, \
                        x.google_event_id AS x_google_event_id, \
                        x.pushed_at AS x_pushed_at, \
                        x.created_at AS x_created_at \
                 FROM {prefix}_google_exports x \
                 JOIN {prefix}_entries e ON e.id = x.entry_id \
                 ORDER BY e.starts_at",
                prefix = self.prefix
            ),
            [],
            |row| {
                let optin = ExportOptIn {
                    entry_id: row.get("x_entry_id")?,
                    google_calendar_id: row.get("x_google_calendar_id")?,
                    google_event_id: row.get("x_google_event_id")?,
                    pushed_at: row.get("x_pushed_at")?,
                    created_at: row.get("x_created_at")?,
                };
                Ok((optin, entry_from_row(row)?))
            },
        )?)
    }

    /// Records what a push produced. `google_event_id` is what turns the next
    /// push into an update instead of a duplicate.
    pub fn record_export_push(
        &self,
        entry_id: &str,
        google_event_id: &str,
    ) -> StoreResult<Option<ExportOptIn>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "UPDATE {prefix}_google_exports \
                     SET google_event_id = ?2, pushed_at = ?3 WHERE entry_id = ?1 RETURNING *",
                    prefix = self.prefix
                ),
                params![&entry_id, &google_event_id, &now_text()],
                export_from_row,
            )
            .optional()?)
    }

    // ---- rhythms ----------------------------------------------------------

    pub fn list_rhythms(&self) -> StoreResult<Vec<Rhythm>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT * FROM {prefix}_rhythms ORDER BY valid_from",
                prefix = self.prefix
            ),
            [],
            rhythm_from_row,
        )?)
    }

    pub fn get_rhythm(&self, id: &str) -> StoreResult<Option<Rhythm>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT * FROM {prefix}_rhythms WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                rhythm_from_row,
            )
            .optional()?)
    }

    /// Creates the rhythm and materializes its future instances atomically.
    /// Returns the rhythm plus how many instances were created.
    pub fn create_rhythm(&self, input: &NewRhythm) -> StoreResult<(Rhythm, usize)> {
        input.validate()?;
        let rhythm = Rhythm {
            id: generated_id("cal:rhythm"),
            kind: input.kind.clone(),
            title: input.title.trim().to_string(),
            location: input.location.clone(),
            byweekday: input.byweekday.clone(),
            start_time: input.start_time.clone(),
            end_time: input.end_time.clone(),
            valid_from: input.valid_from.clone(),
            valid_until: input.valid_until.clone(),
            active: input.active,
            created_at: now_text(),
            updated_at: now_text(),
        };
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            &format!(
                "INSERT INTO {prefix}_rhythms \
                 (id, kind, title, location, byweekday, start_time, end_time, \
                  valid_from, valid_until, active, created_at, updated_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                prefix = self.prefix
            ),
            params![
                &rhythm.id,
                &rhythm.kind,
                &rhythm.title,
                &rhythm.location,
                &rhythm.byweekday.join(","),
                &rhythm.start_time,
                &rhythm.end_time,
                &rhythm.valid_from,
                &rhythm.valid_until,
                rhythm.active as i32,
                &rhythm.created_at,
                &rhythm.updated_at,
            ],
        )?;
        let created = insert_instances(&tx, &self.prefix, &rhythm)?;
        tx.commit()?;
        Ok((rhythm, created))
    }

    /// Applies a patch and re-materializes forward: future linked instances
    /// are deleted and regenerated from the new rule, past ones stay as
    /// history. Detached (user-overridden) instances are untouched — they no
    /// longer carry the rhythm_id.
    pub fn update_rhythm(
        &self,
        id: &str,
        patch: &UpdateRhythm,
    ) -> StoreResult<Option<(Rhythm, usize)>> {
        let mut rhythm = match self.get_rhythm(id)? {
            Some(rhythm) => rhythm,
            None => return Ok(None),
        };
        if let Some(kind) = &patch.kind {
            rhythm.kind = kind.clone();
        }
        if let Some(title) = &patch.title {
            rhythm.title = title.trim().to_string();
        }
        if let Some(location) = &patch.location {
            rhythm.location = Some(location.clone());
        }
        if let Some(byweekday) = &patch.byweekday {
            rhythm.byweekday = byweekday.clone();
        }
        if let Some(start_time) = &patch.start_time {
            rhythm.start_time = Some(start_time.clone());
        }
        if let Some(end_time) = &patch.end_time {
            rhythm.end_time = Some(end_time.clone());
        }
        if let Some(valid_from) = &patch.valid_from {
            rhythm.valid_from = valid_from.clone();
        }
        if let Some(valid_until) = &patch.valid_until {
            rhythm.valid_until = valid_until.clone();
        }
        if let Some(active) = patch.active {
            rhythm.active = active;
        }
        crate::model::validate_rhythm_fields(
            &rhythm.kind,
            &rhythm.title,
            &rhythm.byweekday,
            rhythm.start_time.as_deref(),
            rhythm.end_time.as_deref(),
            &rhythm.valid_from,
            &rhythm.valid_until,
        )?;
        rhythm.updated_at = now_text();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            &format!(
                "UPDATE {prefix}_rhythms SET kind=?2, title=?3, location=?4, byweekday=?5, \
                 start_time=?6, end_time=?7, valid_from=?8, valid_until=?9, active=?10, \
                 updated_at=?11 WHERE id=?1",
                prefix = self.prefix
            ),
            params![
                &rhythm.id,
                &rhythm.kind,
                &rhythm.title,
                &rhythm.location,
                &rhythm.byweekday.join(","),
                &rhythm.start_time,
                &rhythm.end_time,
                &rhythm.valid_from,
                &rhythm.valid_until,
                rhythm.active as i32,
                &rhythm.updated_at,
            ],
        )?;
        let created = if rhythm.active {
            delete_future_instances(&tx, &self.prefix, &rhythm.id)?;
            insert_instances(&tx, &self.prefix, &rhythm)?
        } else {
            // Pausing a rhythm clears its future, keeps its history.
            delete_future_instances(&tx, &self.prefix, &rhythm.id)?
        };
        tx.commit()?;
        Ok(Some((rhythm, created)))
    }

    /// Deletes the rhythm. With `delete_instances`, future generated instances
    /// go too; otherwise they stay (the FK sets their rhythm_id NULL) as
    /// ordinary manual-looking entries.
    pub fn delete_rhythm(&self, id: &str, delete_instances: bool) -> StoreResult<bool> {
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        if delete_instances {
            delete_future_instances(&tx, &self.prefix, id)?;
        }
        let count = tx.execute(
            &format!(
                "DELETE FROM {prefix}_rhythms WHERE id = ?1",
                prefix = self.prefix
            ),
            params![&id],
        )?;
        tx.commit()?;
        Ok(count > 0)
    }

    /// Explicit re-materialization (POST /api/rhythms/:id/materialize).
    /// Idempotent: the (rhythm_id, starts_at) slot unique index turns repeats
    /// into no-ops, and unlike update_rhythm it does not delete anything.
    pub fn materialize_rhythm(&self, id: &str) -> StoreResult<Option<usize>> {
        let rhythm = match self.get_rhythm(id)? {
            Some(rhythm) if rhythm.active => rhythm,
            Some(_) => return Ok(Some(0)),
            None => return Ok(None),
        };
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let created = insert_instances(&tx, &self.prefix, &rhythm)?;
        tx.commit()?;
        Ok(Some(created))
    }
}

fn delete_future_instances(tx: &Transaction, prefix: &str, rhythm_id: &str) -> StoreResult<usize> {
    let today = date::format_date(date::today_days());
    Ok(tx.execute(
        &format!(
            "DELETE FROM {prefix}_entries \
             WHERE rhythm_id = ?1 AND substr(starts_at, 1, 10) >= ?2",
            prefix = prefix
        ),
        params![&rhythm_id, &today],
    )?)
}

fn insert_instances(tx: &Transaction, prefix: &str, rhythm: &Rhythm) -> StoreResult<usize> {
    let now = now_text();
    let mut created = 0;
    for instance in rhythm::instance_entries(rhythm, date::today_days())? {
        created += tx.execute(
            &format!(
                "INSERT INTO {prefix}_entries \
                 (id, kind, title, starts_at, ends_at, all_day, location, notes, \
                  source, external_id, rhythm_id, payload, created_at, updated_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) \
                 ON CONFLICT (rhythm_id, starts_at) WHERE rhythm_id IS NOT NULL DO NOTHING",
                prefix = prefix
            ),
            params![
                &generated_id("cal:entry"),
                &instance.kind,
                &instance.title,
                &instance.starts_at,
                &instance.ends_at,
                instance.all_day as i32,
                &instance.location,
                &instance.notes,
                &instance.source,
                &instance.external_id,
                &instance.rhythm_id,
                &serde_json::to_string(&instance.payload)?,
                &now,
                &now,
            ],
        )?;
    }
    Ok(created)
}

fn entry_as_new(entry: &Entry) -> NewEntry {
    NewEntry {
        kind: entry.kind.clone(),
        commitment: entry.commitment,
        title: entry.title.clone(),
        starts_at: entry.starts_at.clone(),
        ends_at: entry.ends_at.clone(),
        all_day: entry.all_day,
        location: entry.location.clone(),
        notes: entry.notes.clone(),
        source: entry.source.clone(),
        external_id: entry.external_id.clone(),
        rhythm_id: entry.rhythm_id.clone(),
        payload: entry.payload.clone(),
    }
}

fn context_as_new(context: &Context) -> NewContext {
    NewContext {
        kind: context.kind.clone(),
        title: context.title.clone(),
        details: context.details.clone(),
        valid_from: context.valid_from.clone(),
        valid_until: context.valid_until.clone(),
        source: context.source.clone(),
    }
}

fn day_window_bounds(from: &str, to: &str) -> StoreResult<(String, String)> {
    let from_day = date::parse_date(from).ok_or("from must be YYYY-MM-DD")?;
    let to_day = date::parse_date(to).ok_or("to must be YYYY-MM-DD")?;
    if to_day < from_day {
        return Err("to must be on or after from".into());
    }
    Ok((format!("{from}T00:00:00"), format!("{to}T00:00:00")))
}

fn entry_from_input(input: &NewEntry) -> Entry {
    let now = now_text();
    Entry {
        id: generated_id("cal:entry"),
        kind: input.kind.clone(),
        commitment: input.commitment,
        title: input.title.trim().to_string(),
        starts_at: input.starts_at.clone(),
        ends_at: input.ends_at.clone(),
        all_day: input.all_day,
        location: input.location.clone(),
        notes: input.notes.clone(),
        source: input.source.clone(),
        external_id: input.external_id.clone(),
        rhythm_id: input.rhythm_id.clone(),
        payload: input.payload.clone(),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn context_from_input(input: &NewContext) -> Context {
    let now = now_text();
    Context {
        id: generated_id("cal:context"),
        kind: input.kind.clone(),
        title: input.title.trim().to_string(),
        details: input.details.trim().to_string(),
        valid_from: input.valid_from.clone(),
        valid_until: input.valid_until.clone(),
        source: input.source.clone(),
        created_at: now.clone(),
        updated_at: now,
    }
}

/// Columns are read by name, not by index: every caller here is a `SELECT *` or a
/// `RETURNING *`, so the order is the table's and a column added in the middle of
/// `CREATE TABLE` would silently re-number a positional read.
fn entry_from_row(row: &Row) -> rusqlite::Result<Entry> {
    let payload: String = row.get("payload")?;
    Ok(Entry {
        id: row.get("id")?,
        kind: row.get("kind")?,
        commitment: Commitment::from_db(&row.get::<_, String>("commitment")?),
        title: row.get("title")?,
        starts_at: row.get("starts_at")?,
        ends_at: row.get("ends_at")?,
        all_day: row.get::<_, i32>("all_day")? != 0,
        location: row.get("location")?,
        notes: row.get("notes")?,
        source: row.get("source")?,
        external_id: row.get("external_id")?,
        rhythm_id: row.get("rhythm_id")?,
        // Unparseable payload reads as Null rather than failing the row, which is
        // deliberate and predates the port: the payload is provider decoration, and
        // one bad blob must not take the entry it hangs off out of the calendar.
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn context_from_row(row: &Row) -> rusqlite::Result<Context> {
    Ok(Context {
        id: row.get("id")?,
        kind: row.get("kind")?,
        title: row.get("title")?,
        details: row.get("details")?,
        valid_from: row.get("valid_from")?,
        valid_until: row.get("valid_until")?,
        source: row.get("source")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn export_from_row(row: &Row) -> rusqlite::Result<ExportOptIn> {
    Ok(ExportOptIn {
        entry_id: row.get("entry_id")?,
        google_calendar_id: row.get("google_calendar_id")?,
        google_event_id: row.get("google_event_id")?,
        pushed_at: row.get("pushed_at")?,
        created_at: row.get("created_at")?,
    })
}

fn rhythm_from_row(row: &Row) -> rusqlite::Result<Rhythm> {
    let byweekday: String = row.get("byweekday")?;
    Ok(Rhythm {
        id: row.get("id")?,
        kind: row.get("kind")?,
        title: row.get("title")?,
        location: row.get("location")?,
        byweekday: byweekday
            .split(',')
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .collect(),
        start_time: row.get("start_time")?,
        end_time: row.get("end_time")?,
        valid_from: row.get("valid_from")?,
        valid_until: row.get("valid_until")?,
        active: row.get::<_, i32>("active")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_unique() {
        let first = generated_id("cal:entry");
        let second = generated_id("cal:entry");
        assert!(first.starts_with("cal:entry:"));
        assert_ne!(first, second);
    }

    #[test]
    fn table_prefixes_are_restricted() {
        assert!(validate_prefix("calendar_test").is_ok());
        assert!(validate_prefix("calendar; DROP TABLE calendar_entries").is_err());
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn day_window_uses_midnight_boundaries_for_timed_entries() {
        let (from, to) = day_window_bounds("2026-07-30", "2026-07-31").unwrap();
        assert_eq!(from, "2026-07-30T00:00:00");
        assert_eq!(to, "2026-07-31T00:00:00");

        let same_day_end = "2026-07-30T12:00:00";
        assert!(same_day_end > from.as_str());
    }
}

/// Database-backed; the module name is the selector CI splits the suite on. New here:
/// every statement in this file used to need a running Postgres, so the SQL was only
/// ever exercised through the dashboard. A temp file costs nothing.
#[cfg(test)]
mod db_tests {
    use super::*;

    fn open_test_store(suffix: &str) -> CalendarStore {
        let dir = std::env::temp_dir().join(format!("calendar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        // The directory is named by pid, and a pid is recycled eventually. A previous
        // run's rows arriving in this one is the failure the old TRUNCATE prevented.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        CalendarStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    fn an_entry(title: &str, day: &str, kind: &str) -> NewEntry {
        NewEntry {
            kind: kind.to_string(),
            commitment: Commitment::Possible,
            title: title.to_string(),
            starts_at: format!("{day}T09:00:00"),
            ends_at: format!("{day}T10:00:00"),
            all_day: false,
            location: None,
            notes: None,
            source: "manual".to_string(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
        }
    }

    #[test]
    fn entries_for_plan_reads_only_what_trips_wrote() {
        let store = open_test_store("entries_for_plan");
        let mut leg = an_entry("Bonn → Stuttgart", "2026-10-07", "away");
        leg.source = "trips".into();
        leg.payload = serde_json::json!({ "plan_id": "trip:plan:1" });
        store.create_entry(&leg).expect("a trips leg");
        // Same key, another provider: its payload stays inert and is not read.
        let mut foreign = an_entry("Imported", "2026-10-07", "away");
        foreign.payload = serde_json::json!({ "plan_id": "trip:plan:1" });
        store.create_entry(&foreign).expect("a manual entry");
        let mut other = an_entry("Other trip", "2026-10-08", "away");
        other.source = "trips".into();
        other.payload = serde_json::json!({ "plan_id": "trip:plan:2" });
        store.create_entry(&other).expect("another plan's leg");

        let found = store.entries_for_plan("trip:plan:1").expect("a query");
        let titles: Vec<_> = found.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Bonn → Stuttgart"]);
    }

    #[test]
    fn an_external_upsert_raises_commitment_but_never_lowers_it() {
        let store = open_test_store("commitment_ratchet");
        let mut leg = an_entry("Bonn → Stuttgart", "2026-10-08", "away");
        leg.source = "trips".into();
        leg.external_id = Some("trip:stage:stuttgart".into());
        store.upsert_external_entry(&leg).expect("a planning leg");

        leg.commitment = Commitment::Committed;
        let booked = store.upsert_external_entry(&leg).expect("the leg, booked");
        assert_eq!(booked.commitment, Commitment::Committed);

        leg.commitment = Commitment::Possible;
        let again = store.upsert_external_entry(&leg).expect("a stale re-sync");
        assert_eq!(again.commitment, Commitment::Committed);
    }

    #[test]
    fn ping_reaches_the_database() {
        open_test_store("ping")
            .ping()
            .expect("a live store answers");
    }

    /// The readiness handler turns exactly this into a 503, instead of the 200 the
    /// stateless liveness handler answers while every query behind it fails (#126).
    /// It replaces "port 1 is unreachable": there is no port any more, so an
    /// unusable path is the failure a deployment can actually have.
    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        let blocker = std::env::temp_dir().join(format!("calendar-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            CalendarStore::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    /// The window is half-open on days but compared against instants, so an entry
    /// that ends exactly at the boundary is outside it and one that straddles it is in.
    #[test]
    fn the_day_window_is_half_open_and_catches_overlaps() {
        let store = open_test_store("window");
        store
            .create_entry(&an_entry("inside", "2026-08-15", "event"))
            .unwrap();
        store
            .create_entry(&an_entry("after", "2026-08-16", "event"))
            .unwrap();

        let titles: Vec<String> = store
            .list_entries("2026-08-15", "2026-08-16", &[])
            .unwrap()
            .into_iter()
            .map(|entry| entry.title)
            .collect();
        assert_eq!(titles, vec!["inside"]);
        assert!(store.list_entries("2026-08-16", "2026-08-15", &[]).is_err());
    }

    /// `kind = ANY($3)` became `IN (SELECT value FROM json_each(?3))`, and an empty
    /// filter still means "every kind" rather than "no rows".
    #[test]
    fn the_kind_filter_matches_any_of_the_named_kinds() {
        let store = open_test_store("kinds");
        for (title, kind) in [("a", "event"), ("b", "busy"), ("c", "travel")] {
            store
                .create_entry(&an_entry(title, "2026-08-15", kind))
                .unwrap();
        }
        let window = ("2026-08-15", "2026-08-16");

        assert_eq!(
            store.list_entries(window.0, window.1, &[]).unwrap().len(),
            3
        );
        let filter = ["event".to_string(), "travel".to_string()];
        let mut kinds: Vec<String> = store
            .list_entries(window.0, window.1, &filter)
            .unwrap()
            .into_iter()
            .map(|entry| entry.kind)
            .collect();
        kinds.sort();
        assert_eq!(kinds, vec!["event", "travel"]);
        // A kind nobody used is not an error and not everything.
        let none = ["nonexistent".to_string()];
        assert!(store
            .list_entries(window.0, window.1, &none)
            .unwrap()
            .is_empty());
    }

    /// The contribution boundary. A re-import under the same `(source, external_id)`
    /// updates in place, and it must not undo an operator's commitment decision --
    /// which is why `commitment` is absent from the DO UPDATE list.
    #[test]
    fn an_external_upsert_updates_in_place_and_leaves_commitment_alone() {
        let store = open_test_store("upsert");
        let mut input = an_entry("Meetup", "2026-09-01", "event");
        input.source = "scouting".into();
        input.external_id = Some("evt-1".into());

        let first = store.upsert_external_entry(&input).unwrap();
        store
            .update_entry(
                &first.id,
                &UpdateEntry {
                    commitment: Some(Commitment::Committed),
                    ..Default::default()
                },
            )
            .unwrap();

        input.title = "Meetup (renamed upstream)".into();
        let second = store.upsert_external_entry(&input).unwrap();
        assert_eq!(second.id, first.id, "the Axon id survives a re-import");
        assert_eq!(second.title, "Meetup (renamed upstream)");
        assert_eq!(
            second.commitment,
            Commitment::Committed,
            "a provider re-import must not hand a committed event back down"
        );
        assert_eq!(
            store
                .list_entries("2026-09-01", "2026-09-02", &[])
                .unwrap()
                .len(),
            1
        );
    }

    /// The one test that proves `PRAGMA foreign_keys` is applied: SQLite parses
    /// `ON DELETE CASCADE` either way and silently does not enforce it.
    #[test]
    fn deleting_an_entry_takes_its_export_optin_with_it() {
        let store = open_test_store("cascade");
        let entry = store
            .create_entry(&an_entry("Standup", "2026-09-02", "event"))
            .unwrap();
        store.opt_in_export(&entry.id, "primary").unwrap();
        assert_eq!(store.list_export_optins().unwrap().len(), 1);

        assert!(store.delete_entry(&entry.id).unwrap());
        assert!(
            store.list_export_optins().unwrap().is_empty(),
            "the opt-in outlived its entry, so foreign_keys is off"
        );
    }

    /// Both tables carry `created_at`. Without the aliases, the unaliased join would
    /// stamp the entry with the opt-in's date and nothing would say so.
    #[test]
    fn the_export_queue_keeps_each_side_of_the_join_distinct() {
        let store = open_test_store("queue");
        let entry = store
            .create_entry(&an_entry("Talk", "2026-09-03", "event"))
            .unwrap();
        store
            .opt_in_export(&entry.id, "work@group.calendar.google.com")
            .unwrap();

        let queue = store.export_queue().unwrap();
        assert_eq!(queue.len(), 1);
        let (optin, joined) = &queue[0];
        assert_eq!(joined.id, entry.id);
        assert_eq!(joined.created_at, entry.created_at);
        assert_eq!(optin.entry_id, entry.id);
        assert_eq!(optin.google_calendar_id, "work@group.calendar.google.com");
        assert!(
            optin.google_event_id.is_none(),
            "nothing has been pushed yet"
        );

        let pushed = store
            .record_export_push(&entry.id, "goog-1")
            .unwrap()
            .unwrap();
        assert_eq!(pushed.google_event_id.as_deref(), Some("goog-1"));
    }

    /// Materialising is idempotent through the (rhythm_id, starts_at) partial unique
    /// index, and re-running it must add nothing rather than duplicate the week.
    #[test]
    fn materializing_a_rhythm_twice_creates_each_slot_once() {
        let store = open_test_store("rhythm");
        let far_future = date::format_date(date::today_days() + 28);
        let (rhythm, created) = store
            .create_rhythm(&NewRhythm {
                kind: "busy".into(),
                title: "Gym".into(),
                location: None,
                byweekday: vec!["mo".into(), "we".into()],
                start_time: Some("07:00".into()),
                end_time: Some("08:00".into()),
                valid_from: date::format_date(date::today_days()),
                valid_until: far_future,
                active: true,
            })
            .unwrap();
        assert!(created > 0, "a live rhythm materializes its own future");

        let again = store.materialize_rhythm(&rhythm.id).unwrap().unwrap();
        assert_eq!(again, 0, "the slot index turns a repeat into a no-op");
    }

    /// Deleting the rhythm without its instances leaves them as ordinary entries:
    /// that is `ON DELETE SET NULL`, and it is the other half of the FK enforcement.
    #[test]
    fn deleting_a_rhythm_can_leave_its_instances_behind_detached() {
        let store = open_test_store("detach");
        let (rhythm, created) = store
            .create_rhythm(&NewRhythm {
                kind: "busy".into(),
                title: "Choir".into(),
                location: None,
                byweekday: vec!["tu".into()],
                start_time: Some("19:00".into()),
                end_time: Some("21:00".into()),
                valid_from: date::format_date(date::today_days()),
                valid_until: date::format_date(date::today_days() + 21),
                active: true,
            })
            .unwrap();
        assert!(created > 0);

        assert!(store.delete_rhythm(&rhythm.id, false).unwrap());
        let survivors = store
            .list_entries(
                &date::format_date(date::today_days()),
                &date::format_date(date::today_days() + 22),
                &[],
            )
            .unwrap();
        assert_eq!(survivors.len(), created);
        assert!(
            survivors.iter().all(|entry| entry.rhythm_id.is_none()),
            "SET NULL did not fire, so the rows still point at a rhythm that is gone"
        );
    }

    /// The trip ledger is a fact calendar owns about a plan trips owns. Recording it
    /// twice must not claim a second plan for the same entry.
    #[test]
    fn a_trip_materialization_is_recorded_once_per_entry() {
        let store = open_test_store("trips");
        let entry = store
            .create_entry(&an_entry("Berlin", "2026-09-04", "travel"))
            .unwrap();
        assert!(store.trip_plan_for(&entry.id).unwrap().is_none());

        store
            .record_trip_materialization(std::slice::from_ref(&entry.id), "plan-1")
            .unwrap();
        store
            .record_trip_materialization(std::slice::from_ref(&entry.id), "plan-2")
            .unwrap();
        assert_eq!(
            store.trip_plan_for(&entry.id).unwrap().as_deref(),
            Some("plan-1")
        );

        assert_eq!(store.forget_trip_materialization("plan-1").unwrap(), 1);
        assert!(store.trip_plan_for(&entry.id).unwrap().is_none());
    }

    /// Contexts end inclusively -- they describe a human horizon, not a schedulable
    /// instant -- which is the one place the window comparison differs from entries.
    #[test]
    fn contexts_overlap_the_window_with_an_inclusive_end() {
        let store = open_test_store("contexts");
        let created = store
            .create_context(&NewContext {
                kind: "focus".into(),
                title: "Thesis".into(),
                details: String::new(),
                valid_from: "2026-09-01".into(),
                valid_until: "2026-09-30".into(),
                source: "manual".into(),
            })
            .unwrap();

        assert_eq!(
            store
                .list_contexts("2026-09-30", "2026-10-01")
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .list_contexts("2026-10-01", "2026-10-31")
            .unwrap()
            .is_empty());
        assert!(store.delete_context(&created.id).unwrap());
        assert!(store.get_context(&created.id).unwrap().is_none());
    }
}

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sjel_store::QueryAll;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// What `list_places` accumulates per place id, in order: display name, number
/// of visits, first date seen, last date seen, and whether the place carries a
/// coordinate. A named alias rather than the bare tuple because the tuple is
/// built in one place and read positionally (`entry.1`, `entry.2`) in another,
/// which is precisely the shape clippy's `type_complexity` asks to be given a
/// name — the fields stay positional, but the signature says what they are.
type PlaceVisit = (String, usize, Option<String>, Option<String>, bool);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Address,
    Airport,
    City,
    #[default]
    Station,
    Venue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlaceRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: PlaceKind,
    #[serde(default)]
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Bike,
    Bus,
    Car,
    Ferry,
    Flight,
    Train,
    Walk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Booked,
    Completed,
    OptionSelected,
    #[default]
    Planning,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct TripStage {
    pub id: String,
    pub sequence: usize,
    pub origin: PlaceRef,
    pub destination: PlaceRef,
    pub date: Option<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub status: StageStatus,
    #[serde(default)]
    pub selected_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlanSource {
    pub kind: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlan {
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub stages: Vec<TripStage>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub source: Option<PlanSource>,
}

/// Present-and-null becomes `Some(None)`; an absent key stays `None`.
///
/// Copied from `capabilities/calendar/src/model.rs:162-168`, which needed the
/// same distinction for the same reason: a PATCH that cannot express "clear it"
/// makes a field write-once by accident.
fn present_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdatePlan {
    pub title: Option<String>,
    pub origin: Option<PlaceRef>,
    pub destinations: Option<Vec<PlaceRef>>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub interests: Option<String>,
    pub status: Option<String>,
    pub travelers: Option<Vec<String>>,
    pub transport_modes: Option<Vec<TransportMode>>,
    pub stages: Option<Vec<TripStage>>,
    pub cover_image_url: Option<String>,
    /// What the trip is meant to cost, in minor units. Finance keeps every
    /// actual cent and already tags postings with an `axon-trip-id`; this is the
    /// intention those actuals get compared against, which had no home at all.
    ///
    /// `Option<Option<_>>` so an omitted key ("leave it alone") and an explicit
    /// JSON null ("clear it") are different edits: with a plain `Option` a
    /// budget could be set and never removed, and the editor sends null for an
    /// empty field. Same shape and same reason as `UpdateEntry.location`
    /// (capabilities/calendar/src/model.rs:183-190).
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub budget_cents: Option<Option<i64>>,
    /// Clearable for the same reason, and it matters more: `currency` gates
    /// whether a retrospective may record a cost at all
    /// (`put_retrospective`), so a wrong one that cannot be removed is a
    /// wrong unit on every cost that follows.
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub currency: Option<Option<String>>,
    /// The `updated_at` the caller believes it is editing. Omitted keeps the old
    /// last-write-wins behaviour, so nothing that already works breaks.
    ///
    /// `stages` is accepted wholesale, so changing one stage means reading the
    /// plan, editing the array and writing it back. A browser holds that read for
    /// milliseconds. An agent holds it across turns while it calls transit and
    /// reasons, and every stage another writer changed in between is silently
    /// reverted. Same shape as comms' 409-on-hash-mismatch and calendar
    /// rejecting a changed Google revision.
    pub expected_updated_at: Option<String>,
}

/// A plan revision the caller did not expect is a lost update waiting to happen.
///
/// The error text carries `stale_plan` so a caller can branch on it without
/// parsing prose, and names both revisions so it can tell "someone else wrote"
/// apart from "I sent the wrong id".
fn check_expected_revision(
    expected: Option<&str>,
    actual: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match expected {
        Some(expected) if expected != actual => Err(format!(
            "stale_plan: expected_updated_at {expected} but the plan is at {actual}; \
             re-read it and re-apply your change"
        )
        .into()),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripPlan {
    pub id: String,
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    pub interests: String,
    pub status: String,
    pub travelers: Vec<String>,
    pub transport_modes: Vec<TransportMode>,
    pub stages: Vec<TripStage>,
    pub cover_image_url: Option<String>,
    pub source: Option<PlanSource>,
    pub created_at: String,
    pub updated_at: String,
    /// What this trip is meant to cost, in minor units, beside what it actually
    /// did. Finance keeps every actual cent and already tags postings with an
    /// `axon-trip-id`; the intention had no home at all, so the two halves of
    /// "did I overspend" were one HTTP call apart and never compared.
    #[serde(default)]
    pub budget_cents: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlanItem {
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanItem {
    pub id: String,
    pub plan_id: String,
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    pub payload: Value,
    pub created_at: String,
}

/// One plan's close-out record: exactly the three fields PRD §8.2 rules, plus
/// the plan key and the stamp.
///
/// `currency` is ECHOED from `trips_plans.currency` and is not a column here.
/// `cost_cents` is denominated in the plan's own currency, so the money is named
/// once and a second column cannot disagree with it; a plan with no currency
/// refuses a cost with a message naming the fix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Retrospective {
    pub plan_id: String,
    pub cost_cents: Option<i64>,
    pub currency: Option<String>,
    pub again: String,
    pub change_note: String,
    pub filled_at: String,
}

/// A closed trip the ladder should raise today. Carries what a row needs to
/// render and nothing more — no traveler, which is what keeps this the same
/// exposure `GET /api/plans` already has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingRetrospective {
    pub plan_id: String,
    pub title: String,
    pub destinations: Vec<String>,
    pub date_start: String,
    pub date_end: String,
    pub days_since_close: i64,
}

/// How long after a trip closes the ladder keeps asking.
///
/// The rule lives here and nowhere else. A retrospective filled from a memory
/// that is gone is a fabrication, so the prompt expires; the travel page still
/// offers the form for any past plan without one, because navigating to a plan
/// is not the same as being prompted.
pub const RETROSPECTIVE_WINDOW_DAYS: i64 = 45;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanDetails {
    #[serde(flatten)]
    pub plan: TripPlan,
    pub items: Vec<PlanItem>,
    /// One more TOP-LEVEL key, because `plan` is flattened: the wire shape stays
    /// the flat object `schemas/trip-plan.schema.json` and
    /// `dashboard/src/lib/api.ts`'s `PlanDetails extends TripPlan` both encode.
    #[serde(default)]
    pub retrospective: Option<Retrospective>,
}

pub struct TripsStore {
    /// Shared with every other store in this process on the same file, so
    /// opening one is a checkout rather than an open.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `trips` here means `trips_plans` and `trips_plan_items`.
    prefix: String,
}

fn generated_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}:{nanos:x}{sequence:04x}")
}

/// [`generated_id`] for the sibling module. One id minter per capability.
pub fn new_id(prefix: &str) -> String {
    generated_id(prefix)
}

/// [`now_text`] for the sibling module.
pub fn stamp() -> String {
    now_text()
}

fn now_text() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    seconds.to_string()
}

fn generated_stages(input: &CreatePlan) -> Vec<TripStage> {
    let mut previous = input.origin.clone();
    input
        .destinations
        .iter()
        .enumerate()
        .map(|(sequence, destination)| {
            let stage = TripStage {
                id: format!("stage:{}", sequence + 1),
                sequence,
                origin: previous.clone(),
                destination: destination.clone(),
                date: Some(input.date_start.clone()),
                transport_modes: input.transport_modes.clone(),
                travelers: input.travelers.clone(),
                status: StageStatus::Planning,
                selected_option_id: None,
                branch_note: None,
            };
            previous = destination.clone();
            stage
        })
        .collect()
}

fn validate_plan_fields(
    title: &str,
    destinations: &[PlaceRef],
    date_start: &str,
    date_end: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if title.trim().is_empty() {
        return Err("title is required".into());
    }
    if destinations.is_empty() || destinations.len() > 4 {
        return Err("choose between one and four destinations".into());
    }
    if date_start > date_end {
        return Err("date_start must be before or equal to date_end".into());
    }
    Ok(())
}

/// The prefix is interpolated into DDL and every statement, so it is checked
/// rather than trusted. Production passes the literal `trips`; only a test
/// passes anything else.
fn validate_prefix(prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !prefix
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("prefix must contain only ASCII letters, digits, or underscore".into());
    }
    Ok(())
}

impl TripsStore {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "trips")
    }

    pub fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
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
    fn conn(&self) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126).
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The tables as they are, not the history that produced them.
    ///
    /// Postgres reached this shape through `ADD COLUMN IF NOT EXISTS` and a
    /// dropped-and-re-added `CHECK`; SQLite has neither, and the file starts
    /// empty, so the columns those ALTERs added are declared here and the
    /// widened `item_type` list is the one the `CREATE TABLE` carries. Folding
    /// is only correct because no deployed SQLite file predates it — see
    /// libs/sjel-store/README.md, "Writing a capability's DDL".
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_plans (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                origin TEXT NOT NULL,
                destinations TEXT NOT NULL,
                date_start TEXT NOT NULL,
                date_end TEXT NOT NULL,
                interests TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'draft'
                    CHECK (status IN ('draft','saved','archived')),
                travelers TEXT NOT NULL DEFAULT '[]',
                transport_modes TEXT NOT NULL DEFAULT '[]',
                stages TEXT NOT NULL DEFAULT '[]',
                cover_image_url TEXT,
                source_kind TEXT,
                source_ref TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                budget_cents INTEGER,
                currency TEXT
            );
            CREATE TABLE IF NOT EXISTS {prefix}_plan_items (
                id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL REFERENCES {prefix}_plans(id) ON DELETE CASCADE,
                item_type TEXT NOT NULL
                    CHECK (item_type IN ('journey','transport','event','activity','place','stay','image','note','option_set','booking','outcome')),
                day TEXT,
                external_id TEXT NOT NULL,
                title TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE (plan_id, item_type, external_id)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_plan_updated
                ON {prefix}_plans(updated_at DESC);
            CREATE INDEX IF NOT EXISTS {prefix}_idx_item_plan
                ON {prefix}_plan_items(plan_id, day, created_at);
            CREATE UNIQUE INDEX IF NOT EXISTS {prefix}_idx_plan_source
                ON {prefix}_plans(source_kind, source_ref)
                WHERE source_kind IS NOT NULL AND source_ref IS NOT NULL;

            -- One retrospective per plan (PRD 8.2: cost, again?, change?).
            --
            -- A table rather than a twelfth plan item, for two reasons that
            -- stand on their own. The summary groups by destination ACROSS
            -- plans, which is a query with an index rather than a json_extract
            -- scan of every item row. And the three fields are ruled and closed,
            -- so `again` earns a CHECK and `cost_cents` earns INTEGER -- neither
            -- of which a JSON payload carries. The outcome payload next door is
            -- deliberately open because nobody knew its fields yet; here the PRD
            -- already knows them.
            --
            -- No `currency` column: `cost_cents` is denominated in the plan's own
            -- `currency`, so the money is named exactly once and a second column
            -- cannot disagree with it. plan_id as PRIMARY KEY is the whole
            -- idempotency story -- a second POST is a correction, not a row.
            CREATE TABLE IF NOT EXISTS {prefix}_retrospectives (
                plan_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_plans(id) ON DELETE CASCADE,
                cost_cents INTEGER CHECK (cost_cents IS NULL OR cost_cents >= 0),
                again TEXT NOT NULL CHECK (again IN ('yes','no','maybe','not_taken')),
                change_note TEXT NOT NULL DEFAULT '',
                filled_at TEXT NOT NULL
            );
            ",
            prefix = prefix
        ))?;
        // --- pack lists (own block, appended; see src/pack.rs) -------------
        // Its own statement rather than more text inside the batch above, so
        // two streams editing this file touch two hunks that merge cleanly.
        conn.execute_batch(&crate::pack::DDL.replace("{prefix}", prefix))?;
        Self::widen_again_vocabulary(conn, prefix)?;
        Ok(())
    }

    /// Widen the retrospective vocabulary on a file that predates it.
    ///
    /// `libs/sjel-store/README.md` states that folding a widened `CHECK` into
    /// the `CREATE TABLE` is the translation, "and it is only correct because no
    /// deployed SQLite file predates it". That held for every widening until
    /// this one: `not_taken` was added on 2026-09-23 to a table machines had
    /// already created, and SQLite cannot alter a constraint. So the table is
    /// rebuilt — rename, recreate, copy, drop — which is the only form SQLite
    /// offers for it.
    ///
    /// Idempotent, and a no-op on a fresh file: the `CREATE` above has already
    /// written the new vocabulary, so the stored DDL contains `not_taken` and
    /// this returns immediately. The test is on the stored SQL rather than on a
    /// version column because the constraint IS the fact being asked about, and
    /// a version number is a second copy of it that can disagree.
    ///
    /// Runs inside `migrate_once`'s `BEGIN IMMEDIATE`, so it opens no
    /// transaction of its own: nesting one is an error and the write lock is
    /// already held.
    fn widen_again_vocabulary(
        conn: &Connection,
        prefix: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let table = format!("{prefix}_retrospectives");
        let stored: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![&table],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored) = stored else {
            return Ok(());
        };
        if stored.contains("not_taken") {
            return Ok(());
        }
        let old = format!("{table}_before_not_taken");
        conn.execute_batch(&format!(
            "
            ALTER TABLE {table} RENAME TO {old};
            CREATE TABLE {table} (
                plan_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_plans(id) ON DELETE CASCADE,
                cost_cents INTEGER CHECK (cost_cents IS NULL OR cost_cents >= 0),
                again TEXT NOT NULL CHECK (again IN ('yes','no','maybe','not_taken')),
                change_note TEXT NOT NULL DEFAULT '',
                filled_at TEXT NOT NULL
            );
            INSERT INTO {table} (plan_id, cost_cents, again, change_note, filled_at)
                SELECT plan_id, cost_cents, again, change_note, filled_at FROM {old};
            DROP TABLE {old};
            "
        ))?;
        Ok(())
    }

    /// The table prefix, for the sibling module that owns its own tables.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// A pooled connection for `crate::pack`.
    ///
    /// The same escape hatch `capabilities/interior`'s store exposes as
    /// `borrow_connection`, for the same reason: the tables belong to this
    /// capability, and the queries belong beside the shape that reads them.
    pub fn borrow_connection(
        &self,
    ) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        self.conn()
    }

    pub fn create_plan(&self, input: &CreatePlan) -> Result<TripPlan, Box<dyn std::error::Error>> {
        validate_plan_fields(
            &input.title,
            &input.destinations,
            &input.date_start,
            &input.date_end,
        )?;

        let id = generated_id("trip:plan");
        let now = now_text();
        let origin = serde_json::to_string(&input.origin)?;
        let destinations = serde_json::to_string(&input.destinations)?;
        let travelers = serde_json::to_string(&input.travelers)?;
        let transport_modes = serde_json::to_string(&input.transport_modes)?;
        let stages = serde_json::to_string(&if input.stages.is_empty() {
            generated_stages(input)
        } else {
            input.stages.clone()
        })?;
        let source_kind = input.source.as_ref().map(|source| source.kind.as_str());
        let source_ref = input
            .source
            .as_ref()
            .map(|source| source.reference.as_str());
        let conn = self.conn()?;
        let plan = conn.query_row(
            &format!(
                "INSERT INTO {prefix}_plans
                    (id,title,origin,destinations,date_start,date_end,interests,status,travelers,
                     transport_modes,stages,cover_image_url,source_kind,source_ref,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'draft',?8,?9,?10,?11,?12,?13,?14,?14)
                 RETURNING {PLAN_COLUMNS}",
                prefix = self.prefix
            ),
            params![
                &id,
                &input.title.trim(),
                &origin,
                &destinations,
                &input.date_start,
                &input.date_end,
                &input.interests.trim(),
                &travelers,
                &transport_modes,
                &stages,
                &input.cover_image_url,
                &source_kind,
                &source_ref,
                &now,
            ],
            row_to_plan,
        )?;
        Ok(plan)
    }

    pub fn list_plans(&self) -> Result<Vec<TripPlan>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT {PLAN_COLUMNS}
                 FROM {prefix}_plans WHERE status != 'archived'
                 ORDER BY updated_at DESC",
                prefix = self.prefix
            ),
            [],
            row_to_plan,
        )?)
    }

    /// Every plan with its items, archived ones included.
    ///
    /// `list_plans` hides `status = 'archived'` because the dashboard should not show
    /// a finished trip in the workspace. A safety copy does not get to inherit that
    /// filter: an archived plan's `plan_items` are the same only-copy rows PRD Q47
    /// named, and letting `archive` delete the file would make archiving a data loss
    /// with no warning. The one caller is `crate::projection`.
    pub fn list_every_plan(&self) -> Result<Vec<PlanDetails>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let plans = conn.query_all(
            &format!(
                "SELECT {PLAN_COLUMNS} FROM {prefix}_plans ORDER BY created_at, id",
                prefix = self.prefix
            ),
            [],
            row_to_plan,
        )?;
        // Read once for the whole export rather than once per plan: the
        // projection is a safety copy of every plan, archived ones included.
        let filled = self.retrospectives()?;
        let mut out = Vec::with_capacity(plans.len());
        for plan in plans {
            let items = conn.query_all(
                &format!(
                    "SELECT {ITEM_COLUMNS}
                     FROM {prefix}_plan_items WHERE plan_id = ?1
                     ORDER BY day NULLS LAST, created_at",
                    prefix = self.prefix
                ),
                params![&plan.id],
                row_to_item,
            )?;
            let retrospective = filled.iter().find(|row| row.plan_id == plan.id).cloned();
            out.push(PlanDetails {
                plan,
                items,
                retrospective,
            });
        }
        Ok(out)
    }

    pub fn get_plan(&self, id: &str) -> Result<Option<PlanDetails>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let Some(plan) = conn
            .query_row(
                &format!(
                    "SELECT {PLAN_COLUMNS} FROM {prefix}_plans WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                row_to_plan,
            )
            .optional()?
        else {
            return Ok(None);
        };
        let items = conn.query_all(
            &format!(
                "SELECT {ITEM_COLUMNS}
                 FROM {prefix}_plan_items WHERE plan_id = ?1
                 ORDER BY day NULLS LAST, created_at",
                prefix = self.prefix
            ),
            params![&id],
            row_to_item,
        )?;
        let retrospective = self.retrospective(id)?;
        Ok(Some(PlanDetails {
            plan,
            items,
            retrospective,
        }))
    }

    /// The retrospective for one plan, with the plan's own currency echoed onto
    /// it. One keyed read; `get_plan` calls it for the top-level key.
    pub fn retrospective(
        &self,
        plan_id: &str,
    ) -> Result<Option<Retrospective>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT r.plan_id, r.cost_cents, p.currency, r.again, r.change_note,
                            r.filled_at
                     FROM {prefix}_retrospectives r
                     JOIN {prefix}_plans p ON p.id = r.plan_id
                     WHERE r.plan_id = ?1",
                    prefix = self.prefix
                ),
                params![&plan_id],
                row_to_retrospective,
            )
            .optional()?)
    }

    /// Every recorded retrospective. The summary's input; bounded by the number
    /// of trips a person takes, which is why it needs no page.
    pub fn retrospectives(&self) -> Result<Vec<Retrospective>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT r.plan_id, r.cost_cents, p.currency, r.again, r.change_note, r.filled_at
                 FROM {prefix}_retrospectives r
                 JOIN {prefix}_plans p ON p.id = r.plan_id
                 ORDER BY r.filled_at DESC, r.plan_id",
                prefix = self.prefix
            ),
            [],
            row_to_retrospective,
        )?)
    }

    /// Write or correct one plan's retrospective. Returns the row and whether it
    /// was created, because a second POST is a correction and answers 200.
    ///
    /// `Ok(None)` means the plan does not exist, which the handler turns into a
    /// 404; every other refusal is an `Err` naming the fix.
    pub fn put_retrospective(
        &self,
        plan_id: &str,
        cost_cents: Option<i64>,
        again: &str,
        change_note: &str,
    ) -> Result<Option<(Retrospective, bool)>, Box<dyn std::error::Error>> {
        if crate::retrospective::Again::parse(again).is_none() {
            return Err("again must be one of yes, no, maybe, not_taken".into());
        }
        if cost_cents.is_some_and(|cents| cents < 0) {
            return Err("cost_cents must not be negative".into());
        }
        let Some(plan) = self.get_plan(plan_id)?.map(|details| details.plan) else {
            return Ok(None);
        };
        // The money is denominated exactly once, on the plan. Without a currency
        // there is no unit for this number, and a number with no unit is the
        // thing this design refuses to store.
        if cost_cents.is_some() && plan.currency.is_none() {
            return Err(
                "cost_cents needs the plan to carry a currency; set it on the plan first".into(),
            );
        }
        let existed = self.retrospective(plan_id)?.is_some();
        let now = now_text();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_retrospectives
                    (plan_id, cost_cents, again, change_note, filled_at)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT (plan_id) DO UPDATE SET
                    cost_cents = excluded.cost_cents,
                    again = excluded.again,
                    change_note = excluded.change_note,
                    filled_at = excluded.filled_at",
                prefix = self.prefix
            ),
            params![&plan_id, &cost_cents, &again, &change_note.trim(), &now],
        )?;
        let row = self
            .retrospective(plan_id)?
            .ok_or("the retrospective vanished between write and read")?;
        Ok(Some((row, !existed)))
    }

    /// What the ladder should raise today: plans that closed inside the window
    /// and carry no retrospective.
    ///
    /// Four clauses, all of them deliberate. `date_end < today` because the trip
    /// is over. `date_end >= today - RETROSPECTIVE_WINDOW_DAYS` because a
    /// retrospective filled from a memory that is gone is a fabrication.
    /// `status != 'archived'` because archiving is the operator's explicit "I am
    /// done with this". And no existing row, because a filled one is not pending.
    pub fn pending_retrospectives(
        &self,
        today: &str,
    ) -> Result<Vec<PendingRetrospective>, Box<dyn std::error::Error>> {
        let Some(today_day) = crate::windows::day_number(today) else {
            return Err(format!("today must be ISO, got {today:?}").into());
        };
        let earliest = crate::windows::iso_of_day_number(today_day - RETROSPECTIVE_WINDOW_DAYS);
        let conn = self.conn()?;
        let rows: Vec<(String, String, String, String, String)> = conn.query_all(
            &format!(
                "SELECT p.id, p.title, p.destinations, p.date_start, p.date_end
                 FROM {prefix}_plans p
                 LEFT JOIN {prefix}_retrospectives r ON r.plan_id = p.id
                 WHERE p.date_end < ?1 AND p.date_end >= ?2
                   AND p.status != 'archived' AND r.plan_id IS NULL
                 ORDER BY p.date_end DESC",
                prefix = self.prefix
            ),
            params![&today, &earliest],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        Ok(rows
            .into_iter()
            .map(
                |(plan_id, title, destinations, date_start, date_end)| PendingRetrospective {
                    destinations: serde_json::from_str::<Vec<PlaceRef>>(&destinations)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|place| place.name)
                        .collect(),
                    days_since_close: crate::windows::day_number(&date_end)
                        .map(|closed| today_day - closed)
                        .unwrap_or_default(),
                    plan_id,
                    title,
                    date_start,
                    date_end,
                },
            )
            .collect())
    }

    pub fn update_plan(
        &self,
        plan_id: &str,
        input: &UpdatePlan,
    ) -> Result<Option<TripPlan>, Box<dyn std::error::Error>> {
        let Some(details) = self.get_plan(plan_id)? else {
            return Ok(None);
        };
        let current = details.plan;
        check_expected_revision(input.expected_updated_at.as_deref(), &current.updated_at)?;
        // Held before the `unwrap_or` moves each field out of `current`, because
        // "changed" is a comparison against what is stored, not a test of what
        // was sent.
        let origin_before = current.origin.clone();
        let destinations_before = current.destinations.clone();
        let date_start_before = current.date_start.clone();
        let previous_stages = current.stages.clone();
        let title = input.title.clone().unwrap_or(current.title);
        let origin = input.origin.clone().unwrap_or(current.origin);
        let destinations = input.destinations.clone().unwrap_or(current.destinations);
        let date_start = input.date_start.clone().unwrap_or(current.date_start);
        let date_end = input.date_end.clone().unwrap_or(current.date_end);
        let interests = input.interests.clone().unwrap_or(current.interests);
        let status = input.status.clone().unwrap_or(current.status);
        let travelers = input.travelers.clone().unwrap_or(current.travelers);
        let previous_transport_modes = current.transport_modes.clone();
        let transport_modes = input
            .transport_modes
            .clone()
            .unwrap_or(current.transport_modes);
        // Compare VALUES, not presence. PlanEditor sends origin, destinations and
        // date_start on every save, so a presence test made editing a title
        // regenerate every stage from scratch -- discarding `selected_option_id`
        // and the `booked` status of each one. That is the most plausible reason
        // no outcome has ever been recorded against a stage.
        let route_changed = input
            .origin
            .as_ref()
            .is_some_and(|value| *value != origin_before)
            || input
                .destinations
                .as_ref()
                .is_some_and(|value| *value != destinations_before)
            || input
                .date_start
                .as_ref()
                .is_some_and(|value| *value != date_start_before);
        let stages = input.stages.clone().unwrap_or_else(|| {
            if route_changed {
                preserve_stage_state(
                    &previous_stages,
                    generated_stages(&CreatePlan {
                        title: title.clone(),
                        origin: origin.clone(),
                        destinations: destinations.clone(),
                        date_start: date_start.clone(),
                        date_end: date_end.clone(),
                        interests: interests.clone(),
                        travelers: travelers.clone(),
                        transport_modes: transport_modes.clone(),
                        stages: Vec::new(),
                        cover_image_url: current.cover_image_url.clone(),
                        source: current.source.clone(),
                    }),
                )
            } else if input.transport_modes.is_some() {
                propagate_default_transport_modes(
                    current.stages,
                    &previous_transport_modes,
                    &transport_modes,
                )
            } else {
                current.stages
            }
        });
        let cover_image_url = input.cover_image_url.clone().or(current.cover_image_url);
        // `unwrap_or`, not `or`: the outer Some is "the caller said something",
        // and what it said may be null.
        let budget_cents = input.budget_cents.unwrap_or(current.budget_cents);
        let currency = input.currency.clone().unwrap_or(current.currency);
        if budget_cents.is_some_and(|cents| cents < 0) {
            return Err("budget_cents must not be negative".into());
        }

        validate_plan_fields(&title, &destinations, &date_start, &date_end)?;
        if !["draft", "saved", "archived"].contains(&status.as_str()) {
            return Err("status must be draft, saved, or archived".into());
        }

        let now = now_text();
        let origin_json = serde_json::to_string(&origin)?;
        let destinations_json = serde_json::to_string(&destinations)?;
        let travelers_json = serde_json::to_string(&travelers)?;
        let transport_modes_json = serde_json::to_string(&transport_modes)?;
        let stages_json = serde_json::to_string(&stages)?;
        let conn = self.conn()?;
        let plan = conn.query_row(
            &format!(
                "UPDATE {prefix}_plans SET
                    title=?1,origin=?2,destinations=?3,date_start=?4,date_end=?5,interests=?6,
                    status=?7,travelers=?8,transport_modes=?9,stages=?10,cover_image_url=?11,
                    updated_at=?12,budget_cents=?14,currency=?15
                 WHERE id=?13
                 RETURNING {PLAN_COLUMNS}",
                prefix = self.prefix
            ),
            params![
                &title.trim(),
                &origin_json,
                &destinations_json,
                &date_start,
                &date_end,
                &interests.trim(),
                &status,
                &travelers_json,
                &transport_modes_json,
                &stages_json,
                &cover_image_url,
                &now,
                &plan_id,
                &budget_cents,
                &currency,
            ],
            row_to_plan,
        )?;
        Ok(Some(plan))
    }

    /// Moves one item to a day, or clears it.
    ///
    /// `plan_items` has had a `day` column and an index on `(plan_id, day,
    /// created_at)` since the start, and no write path ever decided a value for
    /// it: a saved journey is stamped with the plan's `date_start` even when its
    /// stage runs on a different date, and every saved place gets `null` with
    /// nothing in the system able to fill it in. An index on a column nobody
    /// maintains sorts a multi-day trip into one heap.
    pub fn set_item_day(
        &self,
        plan_id: &str,
        item_id: &str,
        day: Option<&str>,
    ) -> Result<Option<PlanItem>, Box<dyn std::error::Error>> {
        if let Some(day) = day {
            // A date the index can order. Anything else silently sorts wrong.
            if day.len() != 10
                || !day.as_bytes().iter().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        *b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                })
            {
                return Err("day must be YYYY-MM-DD, or null to unset it".into());
            }
        }
        let conn = self.conn()?;
        let item = conn
            .query_row(
                &format!(
                    "UPDATE {prefix}_plan_items SET day = ?3
                 WHERE plan_id = ?1 AND id = ?2
                 RETURNING {ITEM_COLUMNS}",
                    prefix = self.prefix
                ),
                params![&plan_id, &item_id, &day],
                row_to_item,
            )
            .optional()?;
        Ok(item)
    }

    /// Records how a stage actually went, against the intent it was chosen under.
    ///
    /// `StageStatus::Completed` has existed since the start and nothing anywhere
    /// sets it: the past/upcoming split is pure date arithmetic, so the system
    /// has no memory of whether a connection was made, what it really cost, or
    /// whether the transfer was too tight. punctuality knows what every train
    /// does; nothing knows what these trips did.
    ///
    /// A stage with no `selected_option_id` is refused rather than recorded.
    /// With nothing chosen there is nothing to compare an actual against, and the
    /// row would be a hoard rather than a measurement.
    ///
    /// Stored as a plan item so it needs no new table and travels with the plan.
    /// The kill criterion is deliberately cheap to run: if two trips go by
    /// without this being filled in, the whole learning idea is answered and the
    /// endpoint goes away.
    pub fn record_outcome(
        &self,
        plan_id: &str,
        stage_id: &str,
        outcome: &Value,
    ) -> Result<PlanItem, Box<dyn std::error::Error>> {
        let Some(details) = self.get_plan(plan_id)? else {
            return Err("trip plan not found".into());
        };
        let stage = details
            .plan
            .stages
            .iter()
            .find(|stage| stage.id == stage_id)
            .ok_or_else(|| format!("no stage {stage_id} on {plan_id}"))?;
        let Some(selected) = stage.selected_option_id.as_deref() else {
            return Err(format!(
                "stage {stage_id} has no selected_option_id, so there is nothing to compare \
                 an outcome against"
            )
            .into());
        };

        let mut payload = outcome.clone();
        let object = payload
            .as_object_mut()
            .ok_or("outcome must be a JSON object")?;
        // The intent half, copied in at write time. Reading it back later from
        // the stage would compare an actual against whatever the plan says now,
        // which is not what was chosen.
        object.insert("stage_id".into(), Value::String(stage_id.to_string()));
        object.insert(
            "selected_option_id".into(),
            Value::String(selected.to_string()),
        );
        if let Some(date) = stage.date.clone() {
            object.insert("planned_date".into(), Value::String(date));
        }

        self.add_item(
            plan_id,
            &CreatePlanItem {
                item_type: "outcome".into(),
                day: stage.date.clone(),
                external_id: format!("outcome:{stage_id}"),
                title: format!(
                    "How {} → {} went",
                    stage.origin.name, stage.destination.name
                ),
                payload,
            },
        )
    }

    /// Where the operator actually goes, computed on read over the plans that
    /// already exist. No new table: a projection cannot drift from its source.
    ///
    /// `merge_candidates` is the part worth having on day one. The dashboard's
    /// place field slugifies typed text into `place:<slug>` with a null
    /// coordinate whenever the operator does not pick a suggested station, so the
    /// same city typed two ways is two places, and every coordinate-dependent
    /// behaviour downstream (the 75 km candidate match, the map, nearby places)
    /// degrades without saying so. This reports the collisions instead of
    /// guessing at a merge, because merging identities is not a read's decision.
    pub fn list_places(&self) -> Result<Value, Box<dyn std::error::Error>> {
        use std::collections::BTreeMap;

        let mut visits: BTreeMap<String, PlaceVisit> = BTreeMap::new();
        for plan in self.list_plans()? {
            let dated = std::iter::once(&plan.origin)
                .chain(plan.destinations.iter())
                .map(|place| (place, plan.date_start.clone()));
            for (place, date) in dated {
                let entry = visits.entry(place.id.clone()).or_insert_with(|| {
                    (
                        place.name.clone(),
                        0,
                        None,
                        None,
                        place.latitude.is_some() && place.longitude.is_some(),
                    )
                });
                entry.1 += 1;
                entry.2 = Some(match entry.2.take() {
                    Some(first) if first <= date => first,
                    _ => date.clone(),
                });
                entry.3 = Some(match entry.3.take() {
                    Some(last) if last >= date => last,
                    _ => date,
                });
            }
        }

        // Two ids whose names normalise to one string are one place typed twice.
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, (name, _, _, _, _)) in &visits {
            by_name
                .entry(normalize_place_name(name))
                .or_default()
                .push(id.clone());
        }
        let merge_candidates: Vec<Value> = by_name
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(name, ids)| json_value(name, ids))
            .collect();

        let places: Vec<Value> = visits
            .iter()
            .map(|(id, (name, count, first, last, has_coordinate))| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "visits": count,
                    "first": first,
                    "last": last,
                    "has_coordinate": has_coordinate,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "places": places,
            "merge_candidates": merge_candidates,
        }))
    }

    pub fn find_plan_by_source(
        &self,
        kind: &str,
        reference: &str,
    ) -> Result<Option<TripPlan>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let plan = conn
            .query_row(
                &format!(
                    "SELECT {PLAN_COLUMNS}
                 FROM {prefix}_plans WHERE source_kind = ?1 AND source_ref = ?2",
                    prefix = self.prefix
                ),
                params![&kind, &reference],
                row_to_plan,
            )
            .optional()?;
        Ok(plan)
    }

    pub fn add_item(
        &self,
        plan_id: &str,
        input: &CreatePlanItem,
    ) -> Result<PlanItem, Box<dyn std::error::Error>> {
        if !ITEM_TYPES.contains(&input.item_type.as_str()) {
            return Err(format!("item_type must be one of {}", ITEM_TYPES.join(", ")).into());
        }
        if input.external_id.trim().is_empty() || input.title.trim().is_empty() {
            return Err("external_id and title are required".into());
        }
        validate_payload(&input.item_type, &input.payload)?;

        let id = generated_id("trip:item");
        let now = now_text();
        let payload = serde_json::to_string(&input.payload)?;
        let conn = self.conn()?;
        let item = conn.query_row(
            &format!(
                "INSERT INTO {prefix}_plan_items
                    (id,plan_id,item_type,day,external_id,title,payload,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT (plan_id,item_type,external_id) DO UPDATE SET
                    day = excluded.day,
                    title = excluded.title,
                    payload = excluded.payload
                 RETURNING {ITEM_COLUMNS}",
                prefix = self.prefix
            ),
            params![
                &id,
                &plan_id,
                &input.item_type,
                &input.day,
                &input.external_id,
                &input.title.trim(),
                &payload,
                &now,
            ],
            row_to_item,
        )?;
        // Adding an item promotes a draft to saved, but it must not resurrect an
        // archived plan: archiving is the operator's explicit "I am done with
        // this", and an item write is not a request to undo it. Recording a
        // retrospective against a closed trip depends on this.
        conn.execute(
            &format!(
                "UPDATE {prefix}_plans
                    SET updated_at = ?1,
                        status = CASE WHEN status = 'archived' THEN 'archived' ELSE 'saved' END
                  WHERE id = ?2",
                prefix = self.prefix
            ),
            params![&now, &plan_id],
        )?;
        Ok(item)
    }

    pub fn delete_item(
        &self,
        plan_id: &str,
        item_id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {prefix}_plan_items WHERE plan_id = ?1 AND id = ?2",
                prefix = self.prefix
            ),
            params![&plan_id, &item_id],
        )?;
        Ok(count > 0)
    }

    pub fn delete_plan(&self, plan_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!(
                "DELETE FROM {prefix}_plans WHERE id = ?1",
                prefix = self.prefix
            ),
            params![&plan_id],
        )?;
        Ok(count > 0)
    }
}

/// Read back positionally by the two statements above, whose select list is the
/// contract: `plan_id, cost_cents, currency, again, change_note, filled_at`, and
/// the currency comes from the JOINed plan rather than from a column of its own.
fn row_to_retrospective(row: &Row) -> rusqlite::Result<Retrospective> {
    Ok(Retrospective {
        plan_id: row.get(0)?,
        cost_cents: row.get(1)?,
        currency: row.get(2)?,
        again: row.get(3)?,
        change_note: row.get(4)?,
        filled_at: row.get(5)?,
    })
}

/// Read back positionally by `row_to_plan`, so the order is the contract. One
/// declaration because four statements select it and a fifth returns it.
const PLAN_COLUMNS: &str = "id,title,origin,destinations,date_start,date_end,interests,status,
     travelers,transport_modes,stages,cover_image_url,source_kind,source_ref,
     created_at,updated_at,budget_cents,currency";

const ITEM_COLUMNS: &str = "id,plan_id,item_type,day,external_id,title,payload,created_at";

/// Every accepted `item_type`. One list, so the check and the error message it
/// prints cannot drift apart.
pub const ITEM_TYPES: &[&str] = &[
    "journey",
    "transport",
    "event",
    "activity",
    "place",
    "stay",
    "image",
    "note",
    "option_set",
    "booking",
    "outcome",
];

/// The `item_type`s whose payload shape this capability is willing to promise,
/// and the fields a caller must send for each.
///
/// Deliberately short. `payload` is stored as JSON text so provider evidence can
/// be preserved without becoming part of the durable contract, and the freedom
/// that buys is real: `event` alone is written by three different producers with
/// three different shapes (a scouting opportunity, a whole `ScoredResult`, and a
/// calendar anchor). Declaring a shape for `event` would reject two of the three.
///
/// So a variant is declared only where there is exactly one shape to promise:
///
/// - `transport` has one producer and one shape, and is the item an agent most
///   needs to write, because "hold this connection in the plan" is the request.
/// - `option_set` is new here and has no existing producer, so its shape can be
///   fixed from the start. It records the fares that were offered and not taken,
///   which cannot be recovered later at yesterday's prices.
///
/// Every other type stays permissive, and that is a statement rather than an
/// omission: an unmodelled payload is accepted as-is.
/// - `booking` is what makes a stage's `booked` status mean something. Before it,
///   `booked` was a string the API accepted with nothing behind it: no order
///   reference, no fare name, no refundability, no cancellation deadline. It
///   deliberately records `traveler_name_present` as a boolean rather than the
///   name, because whose name is on a ticket is personal data this repo has no
///   reason to hold.
/// - `stay` is declared for its intended producer, accommodation search
///   results entered through the agent surface (in-repo, the demo seeder is
///   the one writer), so its shape is fixed from the start the way
///   `option_set`'s was: where the stay is and when. Coordinates are required
///   because the place matching downstream runs on them; provider fields such
///   as the booking URL, price and rating ride along unvalidated.
const DECLARED_PAYLOADS: &[(&str, &[&str])] = &[
    ("transport", &["mode", "journey"]),
    ("option_set", &["query", "options"]),
    ("booking", &["provider", "order_ref"]),
    ("stay", &["check_in", "check_out", "latitude", "longitude"]),
];

/// Rejects a declared variant that is missing a required field, naming the field.
///
/// A caller that guesses a payload shape used to get a 201 and a row nobody could
/// read back. Naming the field is the whole point: "invalid payload" sends the
/// caller back to the source, one field name sends it back to its own request.
/// The states a meetup moves through. `idea` is where every meetup starts.
pub const MEETUP_STATUSES: &[&str] = &["idea", "asked", "confirmed", "declined", "done"];

/// A meetup is an `activity` whose payload names people in `with` (2026-09-25).
///
/// Not a new `item_type`: that is a `CHECK` rebuild of the one table holding rows that
/// exist nowhere else, for what is an activity with company. `activity` stays permissive,
/// because 54 rows on one plan already use it with another shape, so only the part a
/// meetup adds is checked: `with` is a non-empty list of `{person}`, where `person` is the
/// note name under `Atlas/People/` (the same key `places_person_places.person` uses), and
/// a `status`, when present, is one of [`MEETUP_STATUSES`].
fn validate_meetup(payload: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    let Some(with) = payload.get("with") else {
        return Ok(());
    };
    let people = with
        .as_array()
        .filter(|people| !people.is_empty())
        .ok_or("a meetup's 'with' must be a non-empty list of {person}")?;
    for entry in people {
        let named = entry
            .get("person")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|person| !person.trim().is_empty());
        if !named {
            return Err(
                "every entry in a meetup's 'with' needs a non-empty 'person' \
                 (the note name under Atlas/People/)"
                    .into(),
            );
        }
    }
    if let Some(status) = payload.get("status") {
        let known = status
            .as_str()
            .is_some_and(|s| MEETUP_STATUSES.contains(&s));
        if !known {
            return Err(format!(
                "a meetup's 'status' must be one of: {}",
                MEETUP_STATUSES.join(", ")
            )
            .into());
        }
    }
    Ok(())
}

fn validate_payload(
    item_type: &str,
    payload: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    if item_type == "activity" {
        validate_meetup(payload)?;
    }
    let Some((_, required)) = DECLARED_PAYLOADS.iter().find(|(t, _)| *t == item_type) else {
        return Ok(());
    };
    let Some(object) = payload.as_object() else {
        return Err(format!("payload for item_type '{item_type}' must be an object").into());
    };
    for field in *required {
        if !object.contains_key(*field) {
            return Err(format!(
                "payload for item_type '{item_type}' requires the field '{field}' \
                 (required: {})",
                required.join(", ")
            )
            .into());
        }
    }
    Ok(())
}

/// Case- and whitespace-insensitive, and nothing more. Stripping punctuation or
/// folding umlauts would collapse places that really are different, and this
/// reports collisions for a human to judge rather than merging them itself.
pub(crate) fn normalize_place_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn json_value(name: &str, ids: &[String]) -> Value {
    serde_json::json!({ "normalized_name": name, "ids": ids })
}

/// Carry `status` and `selected_option_id` across a stage regeneration, for each
/// origin→destination pair that survives the route change.
///
/// Only those two fields, and only onto the first unconsumed previous stage with
/// the same pair. Everything else about a regenerated stage is derived from the
/// new plan and must stay derived; a booked connection between two places that
/// are both still on the itinerary is a fact about the world that the edit did
/// not change.
///
/// This does NOT make a stage id durable. `generated_stages` numbers ids from
/// the sequence and mints them fresh on every real route change, so anything
/// storing a stage id inherits that; the decision to give a stage a stable
/// identity is deliberately not taken here.
fn preserve_stage_state(previous: &[TripStage], regenerated: Vec<TripStage>) -> Vec<TripStage> {
    let mut consumed = vec![false; previous.len()];
    regenerated
        .into_iter()
        .map(|mut stage| {
            let matched = previous.iter().enumerate().find(|(index, candidate)| {
                !consumed[*index]
                    && candidate.origin.name == stage.origin.name
                    && candidate.destination.name == stage.destination.name
            });
            if let Some((index, candidate)) = matched {
                consumed[index] = true;
                stage.status = candidate.status.clone();
                stage.selected_option_id = candidate.selected_option_id.clone();
            }
            stage
        })
        .collect()
}

fn propagate_default_transport_modes(
    stages: Vec<TripStage>,
    previous: &[TransportMode],
    updated: &[TransportMode],
) -> Vec<TripStage> {
    stages
        .into_iter()
        .map(|mut stage| {
            // A stage equal to the former plan default has not been customized.
            // Let it follow a global edit while preserving stage-specific choices.
            if stage.transport_modes == previous {
                stage.transport_modes = updated.to_vec();
            }
            stage
        })
        .collect()
}

fn row_to_plan(row: &Row) -> rusqlite::Result<TripPlan> {
    let source_kind: Option<String> = row.get(12)?;
    let source_ref: Option<String> = row.get(13)?;
    Ok(TripPlan {
        id: row.get(0)?,
        title: row.get(1)?,
        origin: sjel_store::json_column(row, 2)?,
        destinations: sjel_store::json_column(row, 3)?,
        date_start: row.get(4)?,
        date_end: row.get(5)?,
        interests: row.get(6)?,
        status: row.get(7)?,
        travelers: sjel_store::json_column(row, 8)?,
        transport_modes: sjel_store::json_column(row, 9)?,
        stages: sjel_store::json_column(row, 10)?,
        cover_image_url: row.get(11)?,
        source: source_kind
            .zip(source_ref)
            .map(|(kind, reference)| PlanSource { kind, reference }),
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        budget_cents: row.get(16)?,
        currency: row.get(17)?,
    })
}

fn row_to_item(row: &Row) -> rusqlite::Result<PlanItem> {
    Ok(PlanItem {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        item_type: row.get(2)?,
        day: row.get(3)?,
        external_id: row.get(4)?,
        title: row.get(5)?,
        payload: sjel_store::json_column(row, 6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A file whose retrospective table predates `not_taken`.
    ///
    /// The narrowing this guards is not hypothetical: `libs/sjel-store/README.md`
    /// states that folding a widened `CHECK` into the `CREATE TABLE` is correct
    /// "only because no deployed SQLite file predates it", and every machine
    /// running trips has a file that does. SQLite cannot alter a constraint, so
    /// without the rebuild the write fails on exactly the machines that have
    /// history — while a fresh test database passes.
    fn deployed_shape() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE trips_plans (id TEXT PRIMARY KEY);
            CREATE TABLE trips_retrospectives (
                plan_id TEXT PRIMARY KEY
                    REFERENCES trips_plans(id) ON DELETE CASCADE,
                cost_cents INTEGER CHECK (cost_cents IS NULL OR cost_cents >= 0),
                again TEXT NOT NULL CHECK (again IN ('yes','no','maybe')),
                change_note TEXT NOT NULL DEFAULT '',
                filled_at TEXT NOT NULL
            );
            INSERT INTO trips_plans (id) VALUES ('p1'), ('p2');
            INSERT INTO trips_retrospectives
                (plan_id, cost_cents, again, change_note, filled_at)
                VALUES ('p1', 1234, 'yes', 'kept me', '2026-01-03');
            ",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_deployed_table_refuses_not_taken_until_it_is_widened() {
        let conn = deployed_shape();
        let refused = conn.execute(
            "INSERT INTO trips_retrospectives
                 (plan_id, cost_cents, again, change_note, filled_at)
                 VALUES ('p2', NULL, 'not_taken', '', '2026-01-04')",
            [],
        );
        assert!(
            refused.is_err(),
            "the premise: the old CHECK is what makes the rebuild necessary"
        );
    }

    #[test]
    fn widening_a_deployed_table_keeps_every_row_and_admits_the_new_word() {
        let conn = deployed_shape();
        TripsStore::widen_again_vocabulary(&conn, "trips").unwrap();

        // The row that was already there survived the rebuild intact — the
        // copy is the step that would silently drop history.
        let (cost, again, note): (Option<i64>, String, String) = conn
            .query_row(
                "SELECT cost_cents, again, change_note FROM trips_retrospectives
                 WHERE plan_id = 'p1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(cost, Some(1234));
        assert_eq!(again, "yes");
        assert_eq!(note, "kept me");

        conn.execute(
            "INSERT INTO trips_retrospectives
                 (plan_id, cost_cents, again, change_note, filled_at)
                 VALUES ('p2', NULL, 'not_taken', 'was not there', '2026-01-04')",
            [],
        )
        .unwrap();

        // And the temporary table the rebuild used is gone rather than left
        // behind as a second copy of the same rows.
        let leftovers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name LIKE '%_before_not_taken'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn widening_is_idempotent_and_a_no_op_on_a_fresh_table() {
        let conn = deployed_shape();
        TripsStore::widen_again_vocabulary(&conn, "trips").unwrap();
        // The second call sees `not_taken` in the stored DDL and returns without
        // rebuilding, so a process that migrates twice cannot lose the rows the
        // first call copied.
        conn.execute(
            "INSERT INTO trips_retrospectives
                 (plan_id, cost_cents, again, change_note, filled_at)
                 VALUES ('p2', NULL, 'not_taken', 'still here', '2026-01-04')",
            [],
        )
        .unwrap();
        TripsStore::widen_again_vocabulary(&conn, "trips").unwrap();
        let note: String = conn
            .query_row(
                "SELECT change_note FROM trips_retrospectives WHERE plan_id = 'p2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(note, "still here");

        // A table that never existed is not created here — the `CREATE` in
        // `run_migration` owns that, and this function only ever widens.
        let absent = Connection::open_in_memory().unwrap();
        TripsStore::widen_again_vocabulary(&absent, "trips").unwrap();
        let created: i64 = absent
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'trips_retrospectives'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn generated_ids_unique() {
        let first = generated_id("trip:plan");
        let second = generated_id("trip:plan");
        assert!(first.starts_with("trip:plan:"));
        assert_ne!(first, second);
    }

    /// The lost-update guard. A browser holds a read for milliseconds; an agent
    /// holds it across turns while it calls transit and reasons, and `stages` is
    /// accepted wholesale, so a stale write silently reverts everything another
    /// writer changed in between.
    #[test]
    fn a_conditional_write_refuses_a_revision_it_did_not_expect() {
        // Omitted keeps last-write-wins, so nothing that already works breaks.
        assert!(check_expected_revision(None, "1786470000").is_ok());
        assert!(check_expected_revision(Some("1786470000"), "1786470000").is_ok());

        let stale = check_expected_revision(Some("1786470000"), "1786479999")
            .expect_err("a changed revision must be refused");
        let message = stale.to_string();
        // The prefix is what the server turns into 409 + code:stale_plan, so a
        // caller can branch without parsing prose.
        assert!(message.starts_with("stale_plan:"), "got: {message}");
        // Both revisions are named, so "someone else wrote" is distinguishable
        // from "I sent the wrong id".
        assert!(message.contains("1786470000") && message.contains("1786479999"));
    }

    /// Two ids whose names normalise to one string are one place typed twice.
    /// The dashboard mints `place:<slug>` with a null coordinate for any typed
    /// text, so this is how a plan quietly stops matching candidates within
    /// 75 km of itself.
    #[test]
    fn place_names_normalise_for_collision_detection_only() {
        assert_eq!(normalize_place_name("Bonn  Hbf"), "bonn hbf");
        assert_eq!(normalize_place_name("BONN HBF"), "bonn hbf");
        assert_eq!(normalize_place_name(" München "), "münchen");
        // Umlauts and punctuation are deliberately NOT folded: collapsing
        // Munchen and München would merge two places a human should judge, and
        // this detector reports collisions rather than deciding them.
        assert_ne!(
            normalize_place_name("Munchen"),
            normalize_place_name("München")
        );
        assert_ne!(
            normalize_place_name("Frankfurt(Main)Hbf"),
            normalize_place_name("Frankfurt Main Hbf")
        );
    }

    #[test]
    fn table_prefixes_are_restricted() {
        assert!(validate_prefix("trips_test").is_ok());
        assert!(validate_prefix("trips; DROP TABLE trips_plans").is_err());
    }

    /// The write paths the dashboard already uses, replayed field for field from
    /// `dashboard/src/routes/travel/+page.svelte`. A declared variant that
    /// rejected one of these would turn a working button into a 400 on deploy,
    /// which is the failure mode this test exists to catch.
    #[test]
    fn every_payload_the_dashboard_already_writes_still_validates() {
        let existing = [
            // saveJourney -- the one transport producer, and the shape now declared.
            (
                "transport",
                json!({ "mode": "train", "journey": { "id": "j:1" } }),
            ),
            // addTravelCandidate: a scouting opportunity.
            (
                "event",
                json!({ "opportunity_id": "o:1", "source": "luma", "url": "https://example.org" }),
            ),
            // saveEvent: the whole search result, shape not ours to fix.
            (
                "event",
                json!({ "title": "t", "score": 0.5, "date": "2026-09-01" }),
            ),
            // saveCalendarEvent: a calendar anchor.
            (
                "event",
                json!({ "calendar_entry_id": "e:1", "commitment": "planned" }),
            ),
            // savePlace.
            (
                "activity",
                json!({ "url": "https://example.org", "latitude": 50.7 }),
            ),
        ];
        for (item_type, payload) in existing {
            assert!(
                validate_payload(item_type, &payload).is_ok(),
                "existing dashboard write for '{item_type}' must keep working: {payload}"
            );
        }
    }

    /// A caller that guesses a declared payload's shape gets told which field it
    /// missed, not a 201 and an unreadable row.
    #[test]
    fn a_declared_variant_names_the_field_it_is_missing() {
        let missing_journey = validate_payload("transport", &json!({ "mode": "train" }))
            .expect_err("transport without a journey must be rejected");
        assert!(
            missing_journey.to_string().contains("journey"),
            "the error must name the missing field, got: {missing_journey}"
        );

        let missing_options = validate_payload(
            "option_set",
            &json!({ "query": { "from": "8000044", "to": "8000105" } }),
        )
        .expect_err("option_set without options must be rejected");
        assert!(missing_options.to_string().contains("options"));

        // A meetup is an activity with people. The existing activity shape (no
        // `with`) is untouched; a `with` must name people, and its status is a
        // meetup status.
        assert!(
            validate_payload("activity", &json!({ "status": "proposed", "venue": "x" })).is_ok()
        );
        assert!(validate_payload(
            "activity",
            &json!({ "with": [{ "person": "Jonas" }], "status": "idea", "activity": "bouldering" })
        )
        .is_ok());
        for bad in [
            json!({ "with": [] }),
            json!({ "with": [{ "person": " " }] }),
            json!({ "with": "Jonas" }),
        ] {
            let err = validate_payload("activity", &bad).expect_err("a meetup must name people");
            assert!(err.to_string().contains("person"), "{err}");
        }
        let wrong_status = validate_payload(
            "activity",
            &json!({ "with": [{ "person": "Jonas" }], "status": "proposed" }),
        )
        .expect_err("a meetup status comes from MEETUP_STATUSES");
        assert!(wrong_status.to_string().contains("idea"));

        // A stay without coordinates is exactly the row A2's coordinate matcher
        // could never use, so the write is refused and names what it lacks.
        let missing_latitude = validate_payload(
            "stay",
            &json!({ "check_in": "2026-09-14", "check_out": "2026-09-16", "longitude": 11.57 }),
        )
        .expect_err("stay without latitude must be rejected");
        assert!(missing_latitude.to_string().contains("latitude"));

        // A payload that is not an object at all is a different mistake, and says so.
        let not_an_object = validate_payload("transport", &json!("a string"))
            .expect_err("a non-object payload must be rejected");
        assert!(not_an_object.to_string().contains("must be an object"));

        // Complete payloads pass.
        assert!(validate_payload(
            "option_set",
            &json!({
                "query": { "from": "8000044", "to": "8000105", "time": "2026-09-01T08:00:00" },
                "options": [{ "id": "j:1", "total_price": 36.47, "chosen": true }],
                "observed_at": "2026-08-11T12:00:00Z"
            })
        )
        .is_ok());

        // A stay as the agent surface writes it from an accommodation search
        // result: the declared fields plus provider evidence riding along.
        assert!(validate_payload(
            "stay",
            &json!({
                "check_in": "2026-09-14",
                "check_out": "2026-09-16",
                "latitude": 48.1371,
                "longitude": 11.5754,
                "provider": "booking.com",
                "url": "https://www.booking.com/hotel/de/example.html",
                "amount_cents": 12200,
                "currency": "EUR"
            })
        )
        .is_ok());
    }

    /// An unmodelled type accepts anything, deliberately: the alternative is
    /// inventing a shape for `note` that no producer agreed to.
    #[test]
    fn undeclared_types_stay_permissive() {
        for item_type in ITEM_TYPES {
            if DECLARED_PAYLOADS.iter().any(|(t, _)| t == item_type) {
                continue;
            }
            assert!(
                validate_payload(item_type, &json!({ "anything": [1, 2, 3] })).is_ok(),
                "'{item_type}' is not declared and must accept any object"
            );
        }
        // And the declared list is a subset of the accepted types, so a variant
        // can never be declared for a type the CHECK constraint would reject.
        for (declared, _) in DECLARED_PAYLOADS {
            assert!(
                ITEM_TYPES.contains(declared),
                "'{declared}' is declared but not an accepted item_type"
            );
        }
    }

    #[test]
    fn global_mode_edit_preserves_custom_stage_modes() {
        let place = |id: &str| PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        };
        let stage = |id: &str, modes: Vec<TransportMode>| TripStage {
            id: id.into(),
            sequence: 1,
            origin: place("origin"),
            destination: place("destination"),
            date: None,
            transport_modes: modes,
            travelers: Vec::new(),
            status: StageStatus::Planning,
            selected_option_id: None,
            branch_note: None,
        };
        let stages = propagate_default_transport_modes(
            vec![
                stage("default", vec![TransportMode::Train]),
                stage("custom", vec![TransportMode::Flight]),
            ],
            &[TransportMode::Train],
            &[TransportMode::Train, TransportMode::Car],
        );
        assert_eq!(
            stages[0].transport_modes,
            vec![TransportMode::Train, TransportMode::Car]
        );
        assert_eq!(stages[1].transport_modes, vec![TransportMode::Flight]);
    }
}

/// Database-backed; named for the selector CI splits on — see
/// `capabilities/scouting/src/store.rs` for why the name is the contract.
///
/// New here. Under Postgres this capability had no store suite at all: every
/// statement needed a running server, so the twenty of them were only ever
/// exercised by the dashboard. A temp file costs nothing, so the port that
/// moved them to SQLite is the moment they get covered.
#[cfg(test)]
mod db_tests {
    use super::*;
    use serde_json::json;

    fn open_test_store(suffix: &str) -> TripsStore {
        let dir = std::env::temp_dir().join(format!("trips-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        // The directory is named by pid, and a pid is recycled eventually. A
        // previous run's rows must not arrive in this one.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        TripsStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    fn place(id: &str) -> PlaceRef {
        PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn a_plan() -> CreatePlan {
        CreatePlan {
            title: "Autumn in Valencia".into(),
            origin: place("bonn"),
            destinations: vec![place("valencia")],
            date_start: "2026-09-01".into(),
            date_end: "2026-09-08".into(),
            interests: "food".into(),
            travelers: vec!["me".into()],
            transport_modes: vec![TransportMode::Train],
            stages: Vec::new(),
            cover_image_url: None,
            source: None,
        }
    }

    #[test]
    fn ping_reaches_the_database() {
        open_test_store("ping")
            .ping()
            .expect("a live store answers its own ping");
    }

    /// Five columns hold JSON as TEXT, so a round trip is the only thing that
    /// proves the plan that comes back is the plan that went in.
    #[test]
    fn a_plan_round_trips_through_its_json_columns() {
        let store = open_test_store("round_trip");
        let written = store.create_plan(&a_plan()).unwrap();
        assert_eq!(written.status, "draft");
        assert_eq!(written.travelers, vec!["me".to_string()]);
        assert_eq!(written.transport_modes, vec![TransportMode::Train]);
        // One destination and no supplied stages, so one stage is generated.
        assert_eq!(written.stages.len(), 1);
        assert_eq!(written.origin.id, "bonn");

        let read_back = store.get_plan(&written.id).unwrap().expect("just written");
        assert_eq!(read_back.plan, written);
        assert!(read_back.items.is_empty());
        assert_eq!(store.list_plans().unwrap().len(), 1);
    }

    /// The lost-update guard, end to end rather than against the helper alone.
    #[test]
    fn a_stale_revision_is_refused_and_a_current_one_is_applied() {
        let store = open_test_store("revision");
        let plan = store.create_plan(&a_plan()).unwrap();

        let stale = store.update_plan(
            &plan.id,
            &UpdatePlan {
                title: Some("Renamed".into()),
                expected_updated_at: Some("not-the-revision".into()),
                ..Default::default()
            },
        );
        assert!(stale.is_err(), "a stale write must be refused");

        let updated = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    title: Some("Renamed".into()),
                    budget_cents: Some(Some(120_000)),
                    currency: Some(Some("EUR".into())),
                    expected_updated_at: Some(plan.updated_at.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.budget_cents, Some(120_000));
        assert_eq!(updated.currency.as_deref(), Some("EUR"));
    }

    /// A budget that can be set and never removed is a defect the editor can
    /// reach in two clicks: the form sends `null` for an empty field. Omitting
    /// the key still leaves the stored value alone, which is the other half of
    /// the same contract.
    #[test]
    fn a_budget_and_a_currency_can_be_cleared_and_an_omitted_key_leaves_them() {
        let store = open_test_store("budget_clear");
        let plan = store.create_plan(&a_plan()).unwrap();

        let set = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    budget_cents: Some(Some(120_000)),
                    currency: Some(Some("EUR".into())),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");
        assert_eq!(set.budget_cents, Some(120_000));

        // An unrelated edit must not touch either field.
        let renamed = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    title: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");
        assert_eq!(renamed.budget_cents, Some(120_000));
        assert_eq!(renamed.currency.as_deref(), Some("EUR"));

        // A present null clears. This is what an emptied field sends.
        let cleared = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    budget_cents: Some(None),
                    currency: Some(None),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");
        assert_eq!(cleared.budget_cents, None);
        assert_eq!(cleared.currency, None);
    }

    /// Present-and-null and absent are different edits on the wire too, not only
    /// in Rust: the handler reads this shape straight off the request body.
    #[test]
    fn an_omitted_budget_and_a_null_budget_deserialize_differently() {
        let omitted: UpdatePlan = serde_json::from_str(r#"{"title":"x"}"#).unwrap();
        assert_eq!(omitted.budget_cents, None);
        assert_eq!(omitted.currency, None);

        let cleared: UpdatePlan =
            serde_json::from_str(r#"{"budget_cents":null,"currency":null}"#).unwrap();
        assert_eq!(cleared.budget_cents, Some(None));
        assert_eq!(cleared.currency, Some(None));

        let set: UpdatePlan = serde_json::from_str(r#"{"budget_cents":9900}"#).unwrap();
        assert_eq!(set.budget_cents, Some(Some(9_900)));
    }

    /// `ON CONFLICT (plan_id,item_type,external_id)` — the same item saved
    /// twice is one row with the second payload, not two rows.
    #[test]
    fn re_adding_an_item_updates_it_rather_than_duplicating_it() {
        let store = open_test_store("item_upsert");
        let plan = store.create_plan(&a_plan()).unwrap();
        let item = CreatePlanItem {
            item_type: "transport".into(),
            day: Some("2026-09-01".into()),
            external_id: "journey:1".into(),
            title: "Bonn → Valencia".into(),
            payload: json!({ "mode": "train", "journey": { "id": "j:1" } }),
        };
        store.add_item(&plan.id, &item).unwrap();

        let again = CreatePlanItem {
            title: "Bonn → Valencia (cheaper)".into(),
            payload: json!({ "mode": "train", "journey": { "id": "j:2" } }),
            ..item.clone()
        };
        let second = store.add_item(&plan.id, &again).unwrap();
        assert_eq!(second.payload["journey"]["id"], "j:2");

        let details = store.get_plan(&plan.id).unwrap().unwrap();
        assert_eq!(details.items.len(), 1, "the upsert must not add a row");
        assert_eq!(details.items[0].title, "Bonn → Valencia (cheaper)");
        // Adding an item promotes a draft, which is what the dashboard reads.
        assert_eq!(details.plan.status, "saved");

        let moved = store
            .set_item_day(&plan.id, &second.id, Some("2026-09-02"))
            .unwrap()
            .expect("the item exists");
        assert_eq!(moved.day.as_deref(), Some("2026-09-02"));
        assert!(store
            .set_item_day(&plan.id, &second.id, Some("02.09.2026"))
            .is_err());
    }

    /// `ON DELETE CASCADE` is enforced only when `PRAGMA foreign_keys` is on,
    /// and SQLite parses the clause happily either way. Deleting a plan with an
    /// item is the cheapest proof that the pool really sets it.
    #[test]
    fn deleting_a_plan_takes_its_items_with_it() {
        let store = open_test_store("cascade");
        let plan = store.create_plan(&a_plan()).unwrap();
        store
            .add_item(
                &plan.id,
                &CreatePlanItem {
                    item_type: "note".into(),
                    day: None,
                    external_id: "note:1".into(),
                    title: "Book the ferry".into(),
                    payload: json!({ "text": "before September" }),
                },
            )
            .unwrap();

        assert!(store.delete_plan(&plan.id).unwrap());
        assert!(store.get_plan(&plan.id).unwrap().is_none());
        // The orphan check: a surviving item would still be readable by id.
        let conn = store.conn().unwrap();
        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM trips_plan_items", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(orphans, 0, "PRAGMA foreign_keys is not being applied");
    }

    /// The partial unique index: a plan with a source is findable by it, and
    /// the plans without one do not collide on their shared NULL.
    #[test]
    fn a_sourced_plan_is_findable_and_unsourced_ones_do_not_collide() {
        let store = open_test_store("source");
        let mut sourced = a_plan();
        sourced.source = Some(PlanSource {
            kind: "obsidian".into(),
            reference: "Atlas/Events/valencia.md".into(),
        });
        let plan = store.create_plan(&sourced).unwrap();
        assert_eq!(
            store
                .find_plan_by_source("obsidian", "Atlas/Events/valencia.md")
                .unwrap()
                .map(|found| found.id),
            Some(plan.id)
        );
        assert!(store
            .find_plan_by_source("obsidian", "missing")
            .unwrap()
            .is_none());

        store.create_plan(&a_plan()).unwrap();
        store.create_plan(&a_plan()).unwrap();
        assert_eq!(store.list_plans().unwrap().len(), 3);
    }

    /// Marks a stage as booked against a chosen option, the state every one of
    /// the next three tests is about not losing.
    fn book_the_first_stage(store: &TripsStore, plan: &TripPlan) -> TripPlan {
        let mut stages = plan.stages.clone();
        stages[0].status = StageStatus::Booked;
        stages[0].selected_option_id = Some("option:chosen".into());
        store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    stages: Some(stages),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists")
    }

    /// The defect: `route_changed` tested whether a field was PRESENT, and
    /// PlanEditor sends origin, destinations and date_start on every save. So
    /// renaming a trip regenerated every stage from scratch and discarded the
    /// booking. It compares values now.
    #[test]
    fn a_title_edit_keeps_a_booked_stage() {
        let store = open_test_store("title_edit");
        let plan = store.create_plan(&a_plan()).unwrap();
        let booked = book_the_first_stage(&store, &plan);

        // Exactly what the editor sends: the route fields unchanged, beside a
        // new title.
        let renamed = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    title: Some("Renamed but the same trip".into()),
                    origin: Some(booked.origin.clone()),
                    destinations: Some(booked.destinations.clone()),
                    date_start: Some(booked.date_start.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");

        assert_eq!(renamed.title, "Renamed but the same trip");
        assert_eq!(
            renamed.stages, booked.stages,
            "a title edit must leave the stages byte-identical"
        );
        assert_eq!(renamed.stages[0].status, StageStatus::Booked);
        assert_eq!(
            renamed.stages[0].selected_option_id.as_deref(),
            Some("option:chosen")
        );
    }

    /// A real route change does regenerate the stages — and a surviving
    /// origin-to-destination pair keeps its state across the regeneration.
    #[test]
    fn a_route_change_carries_the_booked_state_of_a_surviving_stage() {
        let store = open_test_store("route_change");
        let mut input = a_plan();
        input.destinations = vec![place("valencia"), place("madrid")];
        let plan = store.create_plan(&input).unwrap();
        let booked = book_the_first_stage(&store, &plan);
        assert_eq!(booked.stages.len(), 2);

        // Moving the start date is a real route change: the stages regenerate.
        let moved = store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    date_start: Some("2026-09-03".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .expect("the plan exists");

        assert_eq!(moved.stages[0].date.as_deref(), Some("2026-09-03"));
        assert_eq!(
            moved.stages[0].status,
            StageStatus::Booked,
            "bonn to valencia still exists, so its booking survives"
        );
        assert_eq!(
            moved.stages[0].selected_option_id.as_deref(),
            Some("option:chosen")
        );
        assert_eq!(
            moved.stages[1].status,
            StageStatus::Planning,
            "the unbooked stage stays unbooked"
        );
    }

    /// The defect: `add_item` ended with an unconditional `status = 'saved'`, so
    /// writing anything to an archived plan resurrected it. Recording a
    /// retrospective against a closed trip depends on this fix.
    #[test]
    fn adding_an_item_to_an_archived_plan_leaves_it_archived() {
        let store = open_test_store("archived_item");
        let plan = store.create_plan(&a_plan()).unwrap();
        store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    status: Some("archived".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        store
            .add_item(
                &plan.id,
                &CreatePlanItem {
                    item_type: "note".into(),
                    day: None,
                    external_id: "note:1".into(),
                    title: "A thought after the fact".into(),
                    payload: json!({}),
                },
            )
            .unwrap();

        let after = store.get_plan(&plan.id).unwrap().expect("still there");
        assert_eq!(
            after.plan.status, "archived",
            "archiving is the operator's explicit 'I am done'; an item write is not a request to undo it"
        );

        // The control: a draft still gets promoted.
        let draft = store.create_plan(&a_plan()).unwrap();
        store
            .add_item(
                &draft.id,
                &CreatePlanItem {
                    item_type: "note".into(),
                    day: None,
                    external_id: "note:1".into(),
                    title: "A plan taking shape".into(),
                    payload: json!({}),
                },
            )
            .unwrap();
        assert_eq!(
            store.get_plan(&draft.id).unwrap().unwrap().plan.status,
            "saved"
        );
    }

    /// One row per plan: a second POST is a correction, not a second record.
    #[test]
    fn a_second_retrospective_corrects_the_first() {
        let store = open_test_store("retro_correct");
        let plan = store.create_plan(&a_plan()).unwrap();

        let (first, created) = store
            .put_retrospective(&plan.id, None, "maybe", "too rushed")
            .unwrap()
            .expect("the plan exists");
        assert!(created, "the first write is a creation");
        assert_eq!(first.again, "maybe");

        let (second, created) = store
            .put_retrospective(&plan.id, None, "yes", "go back in autumn")
            .unwrap()
            .expect("the plan exists");
        assert!(!created, "a second write is a correction and answers 200");
        assert_eq!(second.again, "yes");
        assert_eq!(second.change_note, "go back in autumn");
        assert_eq!(
            store.retrospectives().unwrap().len(),
            1,
            "one row, corrected"
        );

        // And it reaches the plan as a top-level key.
        let details = store.get_plan(&plan.id).unwrap().unwrap();
        assert_eq!(
            details.retrospective.map(|row| row.again),
            Some("yes".into())
        );

        // A plan that does not exist is not an error, it is a 404 upstream.
        assert!(store
            .put_retrospective("trip:plan:missing", None, "yes", "")
            .unwrap()
            .is_none());
    }

    /// The money is denominated exactly once, on the plan. A number with no unit
    /// is the thing this design refuses to store.
    #[test]
    fn a_cost_needs_the_plan_to_carry_a_currency() {
        let store = open_test_store("retro_currency");
        let plan = store.create_plan(&a_plan()).unwrap();
        assert_eq!(plan.currency, None, "a new plan carries no currency");

        let refused = store
            .put_retrospective(&plan.id, Some(42_000), "yes", "")
            .expect_err("a cost with no unit must be refused")
            .to_string();
        assert!(
            refused.contains("currency"),
            "the 400 names the fix: {refused}"
        );

        // The same POST without a cost succeeds.
        assert!(store
            .put_retrospective(&plan.id, None, "yes", "")
            .unwrap()
            .is_some());

        // With a currency on the plan, the cost lands and echoes the unit.
        store
            .update_plan(
                &plan.id,
                &UpdatePlan {
                    currency: Some(Some("EUR".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        let (row, _) = store
            .put_retrospective(&plan.id, Some(42_000), "yes", "")
            .unwrap()
            .unwrap();
        assert_eq!(row.cost_cents, Some(42_000));
        assert_eq!(row.currency.as_deref(), Some("EUR"));

        // The closed vocabulary and the non-negative cost, at the store level.
        assert!(store.put_retrospective(&plan.id, None, "sure", "").is_err());
        assert!(store
            .put_retrospective(&plan.id, Some(-1), "yes", "")
            .is_err());
    }

    /// The 45-day window lives in `pending_retrospectives` and nowhere else.
    #[test]
    fn the_pending_list_holds_a_closed_plan_drops_it_once_filled_and_forgets_it_after_45_days() {
        let store = open_test_store("retro_pending");
        let closed = |start: &str, end: &str| CreatePlan {
            date_start: start.into(),
            date_end: end.into(),
            ..a_plan()
        };
        let recent = store
            .create_plan(&closed("2026-08-25", "2026-09-02"))
            .unwrap();
        let filled = store
            .create_plan(&closed("2026-08-25", "2026-09-02"))
            .unwrap();
        let old = store
            .create_plan(&closed("2026-07-01", "2026-07-07"))
            .unwrap();
        let upcoming = store
            .create_plan(&closed("2026-09-20", "2026-09-27"))
            .unwrap();
        store
            .put_retrospective(&filled.id, None, "yes", "")
            .unwrap()
            .unwrap();

        let pending = store.pending_retrospectives("2026-09-05").unwrap();
        assert_eq!(
            pending
                .iter()
                .map(|row| row.plan_id.clone())
                .collect::<Vec<_>>(),
            vec![recent.id.clone()],
            "closed 60 days ago ({}) is forgotten, filled ({}) is not pending, \
             and a trip still ahead ({}) has not happened",
            old.id,
            filled.id,
            upcoming.id
        );
        assert_eq!(pending[0].days_since_close, 3);
        assert_eq!(pending[0].destinations, vec!["valencia".to_string()]);

        // Archiving is the operator's explicit "I am done with this".
        store
            .update_plan(
                &recent.id,
                &UpdatePlan {
                    status: Some("archived".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store
            .pending_retrospectives("2026-09-05")
            .unwrap()
            .is_empty());
    }
}

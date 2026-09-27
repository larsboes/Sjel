//! The phone's local store: a read-only copy of C1 answers and an outbox of item edits
//! (PRD §10 A5, sync step 2).
//!
//! ## What this is, and what it is not
//!
//! The Mac is the authority (PRD Q113). This store never accepts state. It keeps two things so
//! the Interior page works while the Mac cannot be reached:
//!
//! - **Snapshot**: the last successful answer to each GET below [`C1_PREFIXES`]. Q110 limits the
//!   first offline slice to C1 apartment and item data, so calendar, comms, finance and places
//!   are never cached here. Trips is the one exception, PRD Q115 (2026-09-25, amends Q110): the
//!   plans that have not ended, read-only, so an itinerary with its booking references is
//!   readable on a train with no signal. See [`is_trip_offline`]. Media and the RoomPlan USDZ are cached too, under the caps
//!   [`MAX_SNAPSHOT_ENTRY_BYTES`] and [`MAX_MEDIA_TOTAL_BYTES`] (least recently used goes first).
//! - **Outbox**: a `PUT`/`PATCH /interior/api/items/:id` that could not reach the Mac. It is
//!   queued only with `If-Match`: without a revision the Mac cannot refuse a stale write, and a
//!   conflict would be overwritten instead of shown. A `409` becomes a visible conflict and is
//!   never retried on its own.
//!
//! The database file is app-private (`<app_data_dir>/`[`DB_FILE`]). It is a cache and a queue,
//! never synced as a file (Q110). Migrations are in [`MIGRATIONS`]; `PRAGMA user_version`
//! records how many have run.
//!
//! The HTTP side sits behind [`Transport`], so the rules here are tested with a fake and no
//! network (`mac_bridge::ReqwestTransport` is the real one).

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::mac_bridge::MacResponse;

/// The local database, in the app data directory.
pub const DB_FILE: &str = "axon-local.db";

/// The paths whose GET answers are kept for offline reads. These are bounded read projections,
/// not full offline parity: edits still require the canonical node.
pub const C1_PREFIXES: &[&str] = &["/interior/api/"];
pub const OFFLINE_PROJECTION_PATHS: &[&str] = &[
    "/vault/api/tasks",
    "/vault/api/people",
    "/calendar/api/entries",
];

/// The largest single answer the snapshot keeps. A larger one is served online and not kept.
pub const MAX_SNAPSHOT_ENTRY_BYTES: usize = 20 * 1024 * 1024;

/// The total for media and USDZ bytes. Above it, the least recently used entries are deleted.
pub const MAX_MEDIA_TOTAL_BYTES: i64 = 200 * 1024 * 1024;

/// The total for JSON answers. Each query string is its own entry, so this is capped too.
pub const MAX_TEXT_TOTAL_BYTES: i64 = 50 * 1024 * 1024;

/// The status a byte answer from the snapshot carries: 203 Non-Authoritative Information, a
/// 2xx that says "a copy, not the origin". `dashboard/src/lib/mac-bridge.ts` reads it as stale.
pub const STALE_BYTES_STATUS: u16 = 203;

/// The status of a write that went into the outbox instead of to the Mac.
pub const QUEUED_STATUS: u16 = 202;

const ITEM_PREFIX: &str = "/interior/api/items/";
const INVENTORY_PATH: &str = "/interior/api/inventory";
const WISHLIST_PATH: &str = "/interior/api/wishlist";

/// Each entry runs once, in order, inside one transaction. Never edit a shipped entry; add one.
pub const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE snapshot (
        path         TEXT    NOT NULL,
        kind         TEXT    NOT NULL CHECK (kind IN ('text', 'bytes')),
        status       INTEGER NOT NULL,
        content_type TEXT,
        body         BLOB    NOT NULL,
        size         INTEGER NOT NULL,
        fetched_at   INTEGER NOT NULL,
        used_at      INTEGER NOT NULL,
        PRIMARY KEY (path, kind)
    );
    CREATE TABLE outbox (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        item_id    TEXT    NOT NULL,
        method     TEXT    NOT NULL CHECK (method IN ('PUT', 'PATCH')),
        path       TEXT    NOT NULL,
        body       TEXT    NOT NULL,
        if_match   TEXT    NOT NULL,
        state      TEXT    NOT NULL CHECK (state IN ('pending', 'conflict', 'failed')),
        error      TEXT,
        current    TEXT,
        attempts   INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX outbox_item_state ON outbox (item_id, state);
",
    "
    ALTER TABLE outbox ADD COLUMN operation_id TEXT;
    UPDATE outbox SET operation_id = 'op_legacy_' || id WHERE operation_id IS NULL;
    CREATE UNIQUE INDEX outbox_operation_id ON outbox (operation_id);
",
];

// ─── Transport ────────────────────────────────────────────────────────────────

/// One request to the Mac, before the bridge's checks.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// One answer from the Mac.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Why a request got no answer.
#[derive(Debug, Clone, PartialEq)]
pub enum SendError {
    /// The Mac could not be reached: connect, DNS or timeout. Only this opens the offline path.
    Unreachable(String),
    /// Anything else: a refused path, a bad header, a TLS or read failure. Fails as before.
    Failed(String),
}

impl SendError {
    pub fn message(self) -> String {
        match self {
            SendError::Unreachable(m) | SendError::Failed(m) => m,
        }
    }
}

/// The way to the Mac. The real one is `mac_bridge::ReqwestTransport`; tests use a fake.
pub trait Transport: Sync {
    /// Sends one request. An answer above `max_bytes` is refused as [`SendError::Failed`].
    fn send(
        &self,
        request: &Outgoing,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Reply, SendError>> + Send;
}

// ─── Paths ────────────────────────────────────────────────────────────────────

fn route_of(path: &str) -> &str {
    path.split_once('?').map_or(path, |(route, _)| route)
}

/// True for a path whose answer the snapshot may keep.
pub fn is_c1(path: &str) -> bool {
    let route = route_of(path);
    C1_PREFIXES
        .iter()
        .any(|prefix| route.starts_with(prefix) && route.len() > prefix.len())
}

/// The first offline projections beyond the apartment cache. Exact list endpoints avoid
/// accidentally caching a provider action or an unbounded graph dump.
pub fn is_offline_projection(path: &str) -> bool {
    let route = route_of(path);
    is_c1(path)
        || OFFLINE_PROJECTION_PATHS.contains(&route)
        || (route.starts_with("/knowledge-graph/api/graph/unit/")
            && route.len() > "/knowledge-graph/api/graph/unit/".len())
        || is_trip_offline(path)
}

/// The plan list. Its offline copy holds only the plans that have not ended.
pub const TRIPS_LIST: &str = "/trips/api/plans";
const TRIPS_PLAN_PREFIX: &str = "/trips/api/plans/";

/// The plan id in `/trips/api/plans/<id>`, or `None` for the list, a sub-route (`/items`,
/// `/cost`) or any other path.
fn trip_plan_id(path: &str) -> Option<&str> {
    let id = route_of(path).strip_prefix(TRIPS_PLAN_PREFIX)?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// True for a trips answer the snapshot may keep: the plan list and one plan's detail. PRD Q115
/// (2026-09-25, amends Q110) admits these and nothing else from trips. Nothing
/// under trips is ever queued for writing.
pub fn is_trip_offline(path: &str) -> bool {
    route_of(path) == TRIPS_LIST || trip_plan_id(path).is_some()
}

/// `encodeURIComponent`, which is how `dashboard/src/lib/api.ts` builds a plan's path. The
/// prefetch must store under the same key the page later reads.
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The civil date (UTC) of a millisecond timestamp, as `YYYY-MM-DD`. Howard Hinnant's
/// days-to-civil algorithm, so no date crate is added for one conversion.
fn utc_date(now_ms: i64) -> String {
    let z = now_ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Whether a plan is over. Its last day is `date_end`, else `date_start`; a plan with neither
/// has not ended. One day of slack, because `today` is the UTC date and the traveller's last
/// evening can already be tomorrow in UTC.
fn plan_ended(plan: &Value, now_ms: i64) -> bool {
    let yesterday = utc_date(now_ms - 86_400_000);
    let last = plan
        .get("date_end")
        .and_then(Value::as_str)
        .or_else(|| plan.get("date_start").and_then(Value::as_str));
    last.is_some_and(|day| day < yesterday.as_str())
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The item id in `/interior/api/items/<id>`, or `None` for any other path (a sub-route such as
/// `/state` or `/impact`, or a query string).
pub fn item_id_of(path: &str) -> Option<String> {
    if path.contains('?') {
        return None;
    }
    let rest = path.strip_prefix(ITEM_PREFIX)?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    percent_decode(rest).filter(|id| !id.is_empty())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

// ─── Store ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Bytes,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Bytes => "bytes",
        }
    }
    fn cap(self) -> i64 {
        match self {
            Kind::Text => MAX_TEXT_TOTAL_BYTES,
            Kind::Bytes => MAX_MEDIA_TOTAL_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    /// Unix milliseconds of the answer this copy holds.
    pub fetched_at: i64,
}

/// One queued edit.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OutboxEntry {
    pub id: i64,
    /// Stable protocol operation id. Retries reuse this value for server-side deduplication.
    pub operation_id: String,
    pub item_id: String,
    pub method: String,
    pub path: String,
    /// The body that will be sent: a JSON object.
    pub body: Value,
    /// The revision this edit was made against, as the `If-Match` value.
    pub if_match: String,
    /// `pending`, `conflict` or `failed`. A sent edit is deleted, not kept as `done`.
    pub state: String,
    pub error: Option<String>,
    /// On `conflict`: the Mac's `current` from the 409 body, `{item, state}`.
    pub current: Option<Value>,
    pub attempts: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Whether the Mac answered the last request. In memory only: it describes this run.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Reach {
    pub offline: bool,
    /// When the Mac stopped answering, unix ms.
    pub offline_since: Option<i64>,
    /// The oldest `fetched_at` among the snapshot answers served since then.
    pub showing_from: Option<i64>,
}

/// What the sync status line shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncStatus {
    pub offline: bool,
    pub offline_since: Option<i64>,
    pub showing_from: Option<i64>,
    pub pending: i64,
    pub conflicts: i64,
    pub failed: i64,
    /// Set when the local database could not be opened; the app then works as before step 2.
    pub store_error: Option<String>,
}

pub struct LocalStore {
    conn: Mutex<Connection>,
    reach: Mutex<Reach>,
    flushing: AtomicBool,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("local store: {e}")
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let done: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        if (index as i64) < done {
            continue;
        }
        // Immediate, the form tools/check-store-transactions.sh requires: the lock is taken up
        // front instead of upgraded mid-transaction. This store is the phone's own file and cannot
        // link axon_store, so the behaviour is named here.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

impl LocalStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        Self::from_connection(Connection::open(path).map_err(db_err)?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self, String> {
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_err)?;
        migrate(&mut conn).map_err(db_err)?;
        Ok(Self {
            conn: Mutex::new(conn),
            reach: Mutex::new(Reach::default()),
            flushing: AtomicBool::new(false),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent (each write is one statement
        // or one transaction), so the poisoned guard is still usable.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn reach_mut(&self) -> std::sync::MutexGuard<'_, Reach> {
        self.reach.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn reach(&self) -> Reach {
        self.reach_mut().clone()
    }

    pub fn mark_online(&self) {
        *self.reach_mut() = Reach::default();
    }

    pub fn mark_offline(&self, now: i64) {
        let mut reach = self.reach_mut();
        if !reach.offline {
            reach.offline = true;
            reach.offline_since = Some(now);
            reach.showing_from = None;
        }
    }

    fn note_served(&self, fetched_at: i64) {
        let mut reach = self.reach_mut();
        reach.showing_from = Some(reach.showing_from.map_or(fetched_at, |t| t.min(fetched_at)));
    }

    // ─── Snapshot ───

    /// Keeps one answer, then trims its kind to the total cap. An answer above
    /// [`MAX_SNAPSHOT_ENTRY_BYTES`] is not kept, and an older copy of it is dropped so an
    /// offline read cannot serve something the Mac has since replaced.
    pub fn put_snapshot(
        &self,
        path: &str,
        kind: Kind,
        reply: &Reply,
        now: i64,
    ) -> Result<bool, String> {
        let conn = self.conn();
        if reply.body.len() > MAX_SNAPSHOT_ENTRY_BYTES {
            conn.execute(
                "DELETE FROM snapshot WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str()],
            )
            .map_err(db_err)?;
            return Ok(false);
        }
        conn.execute(
            "INSERT OR REPLACE INTO snapshot
                (path, kind, status, content_type, body, size, fetched_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                path,
                kind.as_str(),
                reply.status,
                reply.content_type,
                reply.body,
                reply.body.len() as i64,
                now
            ],
        )
        .map_err(db_err)?;
        evict(&conn, kind).map_err(db_err)?;
        Ok(true)
    }

    /// The kept answer for `path`, and marks it used for the LRU order.
    pub fn get_snapshot(
        &self,
        path: &str,
        kind: Kind,
        now: i64,
    ) -> Result<Option<Snapshot>, String> {
        let conn = self.conn();
        let found = conn
            .query_row(
                "SELECT status, content_type, body, fetched_at FROM snapshot
                 WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str()],
                |r| {
                    Ok(Snapshot {
                        status: r.get(0)?,
                        content_type: r.get(1)?,
                        body: r.get(2)?,
                        fetched_at: r.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(db_err)?;
        if found.is_some() {
            conn.execute(
                "UPDATE snapshot SET used_at = ?3 WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str(), now],
            )
            .map_err(db_err)?;
        }
        Ok(found)
    }

    /// Deletes one kept answer, if there is one.
    pub fn drop_snapshot(&self, path: &str, kind: Kind) -> Result<(), String> {
        self.conn()
            .execute(
                "DELETE FROM snapshot WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str()],
            )
            .map(|_| ())
            .map_err(db_err)
    }

    /// When `path` was last stored, without marking it used.
    pub fn snapshot_fetched_at(&self, path: &str, kind: Kind) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT fetched_at FROM snapshot WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Deletes every kept plan detail whose path is not in `keep`. The list and anything
    /// outside trips are not touched.
    pub fn drop_trip_details_except(&self, keep: &[String]) -> Result<(), String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT path FROM snapshot WHERE kind = 'text' AND path LIKE '/trips/api/plans/%'",
            )
            .map_err(db_err)?;
        let stored: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(db_err)?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);
        for path in stored {
            if trip_plan_id(&path).is_some() && !keep.contains(&path) {
                conn.execute(
                    "DELETE FROM snapshot WHERE path = ?1 AND kind = 'text'",
                    params![path],
                )
                .map_err(db_err)?;
            }
        }
        Ok(())
    }

    // ─── Outbox ───

    fn entry_where(
        &self,
        clause: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<OutboxEntry>, String> {
        entries_in(&self.conn(), clause, args)
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> Result<Vec<OutboxEntry>, String> {
        self.entry_where("", &[])
    }

    pub fn entry(&self, id: i64) -> Result<Option<OutboxEntry>, String> {
        Ok(self
            .entry_where("WHERE id = ?1", &[&id])?
            .into_iter()
            .next())
    }

    /// Entries waiting to be sent, in creation order.
    pub fn pending(&self) -> Result<Vec<OutboxEntry>, String> {
        self.entry_where("WHERE state = 'pending'", &[])
    }

    fn pending_for(&self, item_id: &str) -> Result<Option<OutboxEntry>, String> {
        Ok(self
            .entry_where("WHERE state = 'pending' AND item_id = ?1", &[&item_id])?
            .into_iter()
            .next())
    }

    pub fn has_pending(&self) -> bool {
        self.conn()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM outbox WHERE state = 'pending')",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    pub fn status(&self) -> Result<SyncStatus, String> {
        let conn = self.conn();
        let count = |state: &str| -> Result<i64, String> {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE state = ?1",
                params![state],
                |r| r.get(0),
            )
            .map_err(db_err)
        };
        let reach = self.reach();
        Ok(SyncStatus {
            offline: reach.offline,
            offline_since: reach.offline_since,
            showing_from: reach.showing_from,
            pending: count("pending")?,
            conflicts: count("conflict")?,
            failed: count("failed")?,
            store_error: None,
        })
    }

    /// Queues an edit, or folds it into the pending edit for the same item.
    ///
    /// A `PATCH` onto a pending entry merges its fields into the pending body and keeps the
    /// pending `If-Match`: the revision the operator first read is the one the Mac must still
    /// hold. A `PUT` replaces the pending body and becomes a `PUT`. Returns the entry id.
    pub fn enqueue(
        &self,
        item_id: &str,
        method: &str,
        path: &str,
        body: Map<String, Value>,
        if_match: &str,
        now: i64,
    ) -> Result<i64, String> {
        // One lock for the lookup and the write, so two edits of one item cannot both insert.
        let conn = self.conn();
        let pending = entries_in(
            &conn,
            "WHERE state = 'pending' AND item_id = ?1",
            &[&item_id],
        )?
        .into_iter()
        .next();
        if let Some(pending) = pending {
            let (method, merged) = match method {
                "PUT" => ("PUT", body),
                _ => {
                    let mut merged = match pending.body {
                        Value::Object(m) => m,
                        _ => Map::new(),
                    };
                    merged.extend(body);
                    (pending.method.as_str(), merged)
                }
            };
            conn.execute(
                "UPDATE outbox SET method = ?2, body = ?3, updated_at = ?4 WHERE id = ?1",
                params![pending.id, method, Value::Object(merged).to_string(), now],
            )
            .map_err(db_err)?;
            return Ok(pending.id);
        }
        let operation_id = new_operation_id()?;
        conn.execute(
            "INSERT INTO outbox
                (operation_id, item_id, method, path, body, if_match, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?7)",
            params![
                operation_id,
                item_id,
                method,
                path,
                Value::Object(body).to_string(),
                if_match,
                now
            ],
        )
        .map_err(db_err)?;
        Ok(conn.last_insert_rowid())
    }

    /// The Mac accepted `sent`. Deletes the entry, unless an edit was folded into it while the
    /// request was on its way: then the entry stays pending against the revision the Mac just
    /// returned, so the folded fields are sent next and nothing is lost. Without a returned
    /// revision it keeps its old `If-Match`, and the next send is a visible 409.
    fn mark_sent(&self, sent: &OutboxEntry, revision: Option<i64>, now: i64) -> Result<(), String> {
        let conn = self.conn();
        let stored: Option<String> = conn
            .query_row(
                "SELECT body FROM outbox WHERE id = ?1",
                params![sent.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        let unchanged = stored
            .as_deref()
            .and_then(|b| serde_json::from_str::<Value>(b).ok())
            .map_or(true, |b| b == sent.body);
        if unchanged {
            conn.execute("DELETE FROM outbox WHERE id = ?1", params![sent.id])
                .map_err(db_err)?;
        } else if let Some(revision) = revision {
            conn.execute(
                "UPDATE outbox SET if_match = ?2, attempts = attempts + 1, updated_at = ?3
                 WHERE id = ?1",
                params![sent.id, format!("\"{revision}\""), now],
            )
            .map_err(db_err)?;
        }
        Ok(())
    }

    fn delete_entry(&self, id: i64) -> Result<bool, String> {
        Ok(self
            .conn()
            .execute("DELETE FROM outbox WHERE id = ?1", params![id])
            .map_err(db_err)?
            > 0)
    }

    fn note_attempt(&self, id: i64, error: Option<&str>, now: i64) -> Result<(), String> {
        self.conn()
            .execute(
                "UPDATE outbox SET attempts = attempts + 1, error = ?2, updated_at = ?3
                 WHERE id = ?1",
                params![id, error, now],
            )
            .map_err(db_err)?;
        Ok(())
    }

    fn set_state(
        &self,
        id: i64,
        state: &str,
        error: Option<&str>,
        current: Option<&Value>,
        now: i64,
    ) -> Result<(), String> {
        self.conn()
            .execute(
                "UPDATE outbox SET state = ?2, error = ?3, current = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![id, state, error, current.map(Value::to_string), now],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// "Keep mine": the operator saw the Mac's value and still wants theirs. The entry goes
    /// back to `pending` with the Mac's current revision as `If-Match`, so the next flush
    /// overwrites exactly the state that was shown, and a further change on the Mac in between
    /// is again a conflict.
    pub fn keep_mine(&self, id: i64, now: i64) -> Result<(), String> {
        let entry = self
            .entry(id)?
            .ok_or_else(|| format!("outbox entry {id} does not exist"))?;
        if entry.state != "conflict" {
            return Err(format!(
                "outbox entry {id} is {}, not a conflict",
                entry.state
            ));
        }
        let revision = entry
            .current
            .as_ref()
            .and_then(|c| c.pointer("/item/revision"))
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("outbox entry {id} has no current revision from the Mac"))?;
        if let Some(other) = self.pending_for(&entry.item_id)? {
            return Err(format!(
                "outbox entry {} for the same item is still waiting; discard one of them first",
                other.id
            ));
        }
        self.conn()
            .execute(
                "UPDATE outbox SET state = 'pending', if_match = ?2, error = NULL, current = NULL,
                        updated_at = ?3
                 WHERE id = ?1",
                params![id, format!("\"{revision}\""), now],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// "Discard mine": the edit is dropped and the Mac's value stands.
    pub fn discard(&self, id: i64) -> Result<(), String> {
        if self.delete_entry(id)? {
            Ok(())
        } else {
            Err(format!("outbox entry {id} does not exist"))
        }
    }

    // ─── Overlay ───

    /// Lays queued edits over an inventory or wishlist answer, so the page shows what the
    /// operator typed. An item with a pending edit gets its pending fields and `pending: true`;
    /// one with an unresolved conflict keeps the Mac's values and gets `conflict: true`. The
    /// revision is never changed: it stays the one the Mac last confirmed.
    ///
    /// Any other path, or a body that is not the expected shape, comes back unchanged.
    pub fn overlay(&self, path: &str, body: &str) -> String {
        let route = route_of(path);
        if route != INVENTORY_PATH && route != WISHLIST_PATH {
            return body.to_string();
        }
        let Ok(entries) = self.entries() else {
            return body.to_string();
        };
        if entries.is_empty() {
            return body.to_string();
        }
        let Ok(mut parsed) = serde_json::from_str::<Value>(body) else {
            return body.to_string();
        };
        let mut changed = false;
        if route == INVENTORY_PATH {
            if let Some(rows) = parsed.as_array_mut() {
                for row in rows {
                    changed |= overlay_row(row, &entries);
                }
            }
        } else if let Some(items) = parsed.get_mut("items").and_then(Value::as_array_mut) {
            for item in items {
                changed |= overlay_item(item, &entries).is_some();
            }
        }
        if changed {
            parsed.to_string()
        } else {
            body.to_string()
        }
    }
}

fn new_operation_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("local store: secure random source: {error}"))?;
    let mut id = String::from("op_");
    for byte in bytes {
        id.push_str(&format!("{byte:02x}"));
    }
    Ok(id)
}

fn entries_in(
    conn: &Connection,
    clause: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<OutboxEntry>, String> {
    let sql = format!(
        "SELECT id, operation_id, item_id, method, path, body, if_match, state, error, current,
                attempts, created_at, updated_at
         FROM outbox {clause} ORDER BY id"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(args, |r| {
            let body: String = r.get(5)?;
            let current: Option<String> = r.get(9)?;
            Ok(OutboxEntry {
                id: r.get(0)?,
                operation_id: r.get(1)?,
                item_id: r.get(2)?,
                method: r.get(3)?,
                path: r.get(4)?,
                body: serde_json::from_str(&body).unwrap_or(Value::Null),
                if_match: r.get(6)?,
                state: r.get(7)?,
                error: r.get(8)?,
                current: current.and_then(|c| serde_json::from_str(&c).ok()),
                attempts: r.get(10)?,
                created_at: r.get(11)?,
                updated_at: r.get(12)?,
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<_, _>>().map_err(db_err)
}

/// Deletes the least recently used entries of one kind until the kind fits its cap.
fn evict(conn: &Connection, kind: Kind) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT path, size FROM snapshot WHERE kind = ?1 ORDER BY used_at DESC, fetched_at DESC",
    )?;
    let rows: Vec<(String, i64)> = stmt
        .query_map(params![kind.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut total = 0i64;
    for (path, size) in rows {
        total += size;
        if total > kind.cap() {
            conn.execute(
                "DELETE FROM snapshot WHERE path = ?1 AND kind = ?2",
                params![path, kind.as_str()],
            )?;
        }
    }
    Ok(())
}

/// Applies the entries for one item object. Returns the flag it set, if any.
fn overlay_item(item: &mut Value, entries: &[OutboxEntry]) -> Option<&'static str> {
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let object = item.as_object_mut()?;
    let mut flag = None;
    for entry in entries.iter().filter(|e| e.item_id == id) {
        match entry.state.as_str() {
            "pending" => {
                if let Value::Object(fields) = &entry.body {
                    for (key, value) in fields {
                        if key == "id" || key == "revision" || key == "expected_revision" {
                            continue;
                        }
                        // A PUT names every field; a PATCH only the changed ones. Either way
                        // a field the entry names is what the operator typed.
                        object.insert(key.clone(), value.clone());
                    }
                }
                flag = Some("pending");
            }
            "conflict" if flag.is_none() => flag = Some("conflict"),
            _ => {}
        }
    }
    if let Some(flag) = flag {
        object.insert(flag.to_string(), Value::Bool(true));
    }
    flag
}

fn overlay_row(row: &mut Value, entries: &[OutboxEntry]) -> bool {
    let Some(item) = row.get_mut("item") else {
        return false;
    };
    let Some(flag) = overlay_item(item, entries) else {
        return false;
    };
    if let Some(object) = row.as_object_mut() {
        object.insert(flag.to_string(), Value::Bool(true));
    }
    true
}

/// True when a 409's current item already carries every field of a queued PATCH. That happens
/// when the first attempt reached the Mac but its answer was lost: nothing differs, so there is
/// nothing to show. A PUT is never judged this way, because it also clears what it omits.
fn already_applied(entry: &OutboxEntry, current: Option<&Value>) -> bool {
    if entry.method != "PATCH" {
        return false;
    }
    let (Some(item), Value::Object(fields)) = (current.and_then(|c| c.get("item")), &entry.body)
    else {
        return false;
    };
    fields
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "id" | "revision" | "expected_revision"))
        .all(|(k, v)| item.get(k) == Some(v))
}

// ─── Requests ─────────────────────────────────────────────────────────────────

fn text_response(reply: Reply) -> MacResponse {
    MacResponse {
        status: reply.status,
        content_type: reply.content_type,
        body: String::from_utf8_lossy(&reply.body).into_owned(),
        stale: false,
        fetched_at: None,
    }
}

fn queued_response(outbox_id: i64, state: &str, revision: Option<i64>) -> MacResponse {
    let mut body = serde_json::json!({ "queued": true, "outbox_id": outbox_id, "state": state });
    if let Some(revision) = revision {
        body["revision"] = revision.into();
    }
    MacResponse {
        status: QUEUED_STATUS,
        content_type: Some("application/json".into()),
        body: body.to_string(),
        stale: false,
        fetched_at: None,
    }
}

fn record_reach(store: &LocalStore, result: &Result<Reply, SendError>, now: i64) {
    match result {
        Ok(_) => store.mark_online(),
        Err(SendError::Unreachable(_)) => store.mark_offline(now),
        Err(SendError::Failed(_)) => {}
    }
}

/// The text path without a local store: what `mac_request` did before step 2.
pub async fn plain_text<T: Transport>(t: &T, request: Outgoing) -> Result<MacResponse, String> {
    t.send(&request, usize::MAX)
        .await
        .map(text_response)
        .map_err(SendError::message)
}

/// `mac_request` with the snapshot and the outbox.
pub async fn request_text<T: Transport>(
    store: &LocalStore,
    t: &T,
    request: Outgoing,
    now: i64,
) -> Result<MacResponse, String> {
    let method = request.method.trim().to_ascii_uppercase();
    if method == "GET" {
        return read_text(store, t, request, now).await;
    }
    if method == "PUT" || method == "PATCH" {
        if let Some(item_id) = item_id_of(&request.path) {
            return write_item(store, t, request, &method, &item_id, now).await;
        }
    }
    let result = t.send(&request, usize::MAX).await;
    record_reach(store, &result, now);
    result.map(text_response).map_err(SendError::message)
}

async fn read_text<T: Transport>(
    store: &LocalStore,
    t: &T,
    request: Outgoing,
    now: i64,
) -> Result<MacResponse, String> {
    let path = request.path.as_str();
    let result = t.send(&request, usize::MAX).await;
    record_reach(store, &result, now);
    match result {
        Ok(reply) => {
            if is_success(reply.status) && is_offline_projection(path) {
                if let Err(e) = store.put_snapshot(path, Kind::Text, &reply, now) {
                    log::warn!("{e}");
                }
            }
            if is_success(reply.status) && is_trip_offline(path) {
                keep_trips(store, t, path, &reply, now).await;
            }
            let ok = is_success(reply.status);
            let mut response = text_response(reply);
            if ok {
                response.body = store.overlay(path, &response.body);
            }
            Ok(response)
        }
        Err(SendError::Unreachable(message)) if is_offline_projection(path) => {
            match store.get_snapshot(path, Kind::Text, now) {
                Ok(Some(snap)) => {
                    let Ok(body) = String::from_utf8(snap.body) else {
                        return Err(message);
                    };
                    store.note_served(snap.fetched_at);
                    Ok(MacResponse {
                        status: snap.status,
                        content_type: snap.content_type,
                        body: store.overlay(path, &body),
                        stale: true,
                        fetched_at: Some(snap.fetched_at),
                    })
                }
                Ok(None) => Err(format!(
                    "{message} (no offline copy of {path} on this device)"
                )),
                Err(e) => Err(format!("{message} ({e})")),
            }
        }
        Err(e) => Err(e.message()),
    }
}

/// Keeps the offline copy of trips current after a successful read.
///
/// - The list is stored with the ended plans taken out. Every plan still in it is fetched and
///   stored too, so a plan is readable offline without having been opened first. A detail
///   stored in the last ten minutes is not fetched again.
/// - A plan's detail is stored while the plan has not ended, and deleted once it has.
/// - A stored detail whose plan is no longer in the list (ended, deleted) is deleted.
///
/// Failures are logged and never fail the read: the caller already has its answer.
async fn keep_trips<T: Transport>(store: &LocalStore, t: &T, path: &str, reply: &Reply, now: i64) {
    let Ok(body) = serde_json::from_slice::<Value>(&reply.body) else {
        return;
    };
    if let Some(id) = trip_plan_id(path) {
        let detail = format!("{TRIPS_PLAN_PREFIX}{id}");
        let result = if plan_ended(&body, now) {
            store.drop_snapshot(&detail, Kind::Text)
        } else {
            store
                .put_snapshot(&detail, Kind::Text, reply, now)
                .map(|_| ())
        };
        if let Err(e) = result {
            log::warn!("{e}");
        }
        return;
    }
    let Some(plans) = body.as_array() else {
        return;
    };
    let upcoming: Vec<Value> = plans
        .iter()
        .filter(|plan| !plan_ended(plan, now))
        .cloned()
        .collect();
    let ids: Vec<String> = upcoming
        .iter()
        .filter_map(|plan| plan.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let filtered = Reply {
        status: reply.status,
        content_type: reply.content_type.clone(),
        body: Value::Array(upcoming).to_string().into_bytes(),
    };
    if let Err(e) = store.put_snapshot(TRIPS_LIST, Kind::Text, &filtered, now) {
        log::warn!("{e}");
    }
    let keep: Vec<String> = ids
        .iter()
        .map(|id| format!("{TRIPS_PLAN_PREFIX}{}", encode_component(id)))
        .collect();
    if let Err(e) = store.drop_trip_details_except(&keep) {
        log::warn!("{e}");
    }
    for detail in keep {
        if store
            .snapshot_fetched_at(&detail, Kind::Text)
            .is_some_and(|at| now - at < 600_000)
        {
            continue;
        }
        let request = Outgoing {
            method: "GET".into(),
            path: detail.clone(),
            headers: vec![],
            body: None,
        };
        match t.send(&request, MAX_SNAPSHOT_ENTRY_BYTES).await {
            Ok(answer) if is_success(answer.status) => {
                if let Err(e) = store.put_snapshot(&detail, Kind::Text, &answer, now) {
                    log::warn!("{e}");
                }
            }
            Ok(answer) => log::warn!("trips prefetch {detail}: HTTP {}", answer.status),
            Err(e) => {
                log::warn!("trips prefetch {detail}: {}", e.message());
                return;
            }
        }
    }
}

async fn write_item<T: Transport>(
    store: &LocalStore,
    t: &T,
    request: Outgoing,
    method: &str,
    item_id: &str,
    now: i64,
) -> Result<MacResponse, String> {
    let if_match = header(&request.headers, "if-match").map(str::to_string);

    // An edit for this item is already waiting. Sending this one directly would race it, so
    // it joins the queued edit and the queue is flushed now.
    if store.pending_for(item_id)?.is_some() {
        let Some(if_match) = if_match else {
            return Err(format!(
                "an edit of `{item_id}` is waiting to be sent; a further edit needs If-Match"
            ));
        };
        let body = object_body(&request)?;
        let id = store.enqueue(item_id, method, &request.path, body, &if_match, now)?;
        let report = flush(store, t, now).await;
        return Ok(queued_outcome(store, id, &report));
    }

    let result = t.send(&request, usize::MAX).await;
    record_reach(store, &result, now);
    match result {
        Ok(reply) => Ok(text_response(reply)),
        Err(SendError::Unreachable(message)) => {
            let Some(if_match) = if_match else {
                return Err(format!(
                    "{message}. The edit was not queued: it carries no If-Match revision, so \
                     the Mac could not refuse it if the item changed there meanwhile"
                ));
            };
            let body = object_body(&request)?;
            let id = store.enqueue(item_id, method, &request.path, body, &if_match, now)?;
            Ok(queued_response(id, "pending", None))
        }
        Err(e) => Err(e.message()),
    }
}

fn object_body(request: &Outgoing) -> Result<Map<String, Value>, String> {
    match request.body.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(Value::Object(map))) => Ok(map),
        _ => Err("the edit was not queued: its body is not a JSON object".into()),
    }
}

fn queued_outcome(store: &LocalStore, id: i64, report: &FlushReport) -> MacResponse {
    match report.outcomes.iter().find(|(entry, _)| *entry == id) {
        Some((_, Outcome::Sent { revision })) => queued_response(id, "sent", *revision),
        Some((_, Outcome::Conflict)) => queued_response(id, "conflict", None),
        Some((_, Outcome::Failed)) => queued_response(id, "failed", None),
        _ => {
            let state = store
                .entry(id)
                .ok()
                .flatten()
                .map_or_else(|| "pending".to_string(), |e| e.state);
            queued_response(id, &state, None)
        }
    }
}

/// A byte answer, with `stale` set when it came from the snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct BytesAnswer {
    pub reply: Reply,
    pub stale: bool,
}

/// `mac_request_bytes` with the snapshot: pictures and the RoomPlan USDZ.
pub async fn request_bytes<T: Transport>(
    store: &LocalStore,
    t: &T,
    request: Outgoing,
    max_bytes: usize,
    now: i64,
) -> Result<BytesAnswer, String> {
    let path = request.path.clone();
    let result = t.send(&request, max_bytes).await;
    record_reach(store, &result, now);
    match result {
        Ok(reply) => {
            if is_success(reply.status) && is_c1(&path) {
                if let Err(e) = store.put_snapshot(&path, Kind::Bytes, &reply, now) {
                    log::warn!("{e}");
                }
            }
            Ok(BytesAnswer {
                reply,
                stale: false,
            })
        }
        Err(SendError::Unreachable(message)) if is_c1(&path) => {
            match store.get_snapshot(&path, Kind::Bytes, now) {
                Ok(Some(snap)) => {
                    store.note_served(snap.fetched_at);
                    Ok(BytesAnswer {
                        reply: Reply {
                            status: STALE_BYTES_STATUS,
                            content_type: snap.content_type,
                            body: snap.body,
                        },
                        stale: true,
                    })
                }
                Ok(None) => Err(format!(
                    "{message} (no offline copy of {path} on this device)"
                )),
                Err(e) => Err(format!("{message} ({e})")),
            }
        }
        Err(e) => Err(e.message()),
    }
}

// ─── Flush ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// 2xx, or a 409 whose current item already holds the edit. The entry is deleted.
    Sent { revision: Option<i64> },
    /// 409: the Mac's item changed. Waits for "keep mine" or "discard mine".
    Conflict,
    /// Another 4xx: the Mac refused the edit. Waits for "discard mine".
    Failed,
    /// No answer, or a 5xx. Stays pending; the flush stops so the order holds.
    Deferred,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlushReport {
    /// Entry id and what became of it, in the order sent.
    pub outcomes: Vec<(i64, Outcome)>,
    /// Another flush was running; this one did nothing.
    pub busy: bool,
}

struct FlushGuard<'a>(&'a AtomicBool);
impl Drop for FlushGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Sends pending edits in creation order.
///
/// - 2xx: deleted, and the inventory snapshot is fetched again afterwards.
/// - 409: `conflict`, with the Mac's current item kept for the conflicts view. Never retried
///   by the flush; only "keep mine" puts it back in the queue.
/// - other 4xx: `failed`, with the Mac's message.
/// - no answer or 5xx: stays `pending`, and the flush stops there.
pub async fn flush<T: Transport>(store: &LocalStore, t: &T, now: i64) -> FlushReport {
    if store.flushing.swap(true, Ordering::SeqCst) {
        return FlushReport {
            busy: true,
            ..FlushReport::default()
        };
    }
    let _guard = FlushGuard(&store.flushing);
    let mut report = FlushReport::default();
    let pending = match store.pending() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("{e}");
            return report;
        }
    };
    for entry in pending {
        let outcome = send_entry(store, t, &entry, now).await;
        let stop = outcome == Outcome::Deferred;
        report.outcomes.push((entry.id, outcome));
        if stop {
            break;
        }
    }
    if report
        .outcomes
        .iter()
        .any(|(_, o)| matches!(o, Outcome::Sent { .. }))
    {
        let refresh = Outgoing {
            method: "GET".into(),
            path: INVENTORY_PATH.into(),
            headers: vec![("accept".into(), "application/json".into())],
            body: None,
        };
        if let Ok(reply) = t.send(&refresh, usize::MAX).await {
            if is_success(reply.status) {
                if let Err(e) = store.put_snapshot(INVENTORY_PATH, Kind::Text, &reply, now) {
                    log::warn!("{e}");
                }
            }
        }
    }
    report
}

/// Flushes the mobile outbox through the authenticated axon-sync/v1 endpoint.
///
/// The desktop keeps the legacy canonical REST path because it is the canonical node itself;
/// paired mobile nodes use this envelope so the device signature and operation id cover the whole
/// mutation.
#[cfg(mobile)]
pub async fn flush_protocol<T: Transport>(
    store: &LocalStore,
    t: &T,
    now: i64,
    origin_node_id: &str,
    target_node_id: &str,
    actor_device_id: &str,
) -> FlushReport {
    if store.flushing.swap(true, Ordering::SeqCst) {
        return FlushReport {
            busy: true,
            ..FlushReport::default()
        };
    }
    let _guard = FlushGuard(&store.flushing);
    let pending = match store.pending() {
        Ok(pending) => pending,
        Err(error) => {
            log::warn!("{error}");
            return FlushReport::default();
        }
    };
    if pending.is_empty() {
        return FlushReport::default();
    }
    let sent_at = now.div_euclid(1000);
    let mutations: Vec<Value> = pending
        .iter()
        .map(|entry| {
            let fields = entry.body.clone();
            serde_json::json!({
                "operation_id": entry.operation_id,
                "entity_type": "interior.item",
                "entity_id": entry.item_id,
                "action": if entry.method == "PUT" { "upsert" } else { "patch" },
                "base_revision": entry.if_match.trim_matches('"'),
                "fields": fields,
                "actor_device_id": actor_device_id,
                "created_at": entry.created_at.div_euclid(1000)
            })
        })
        .collect();
    let body = serde_json::json!({
        "protocol_version": "axon-sync/v1",
        "envelope_id": new_operation_id().unwrap_or_else(|_| format!("env_{sent_at}")),
        "origin_node_id": origin_node_id,
        "target_node_id": target_node_id,
        "actor_device_id": actor_device_id,
        "cursor": null,
        "mutations": mutations,
        "acknowledgements": [],
        "conflicts": [],
        "sent_at": sent_at
    });
    let request = Outgoing {
        method: "POST".into(),
        path: "/interior/api/sync".into(),
        headers: vec![("content-type".into(), "application/json".into())],
        body: Some(body.to_string()),
    };
    let result = t.send(&request, MAX_SNAPSHOT_ENTRY_BYTES).await;
    record_reach(store, &result, now);
    let reply = match result {
        Ok(reply) if is_success(reply.status) => reply,
        Ok(reply) => {
            let message = String::from_utf8_lossy(&reply.body).into_owned();
            for entry in &pending {
                let _ = store.note_attempt(entry.id, Some(&message), now);
            }
            return FlushReport {
                outcomes: pending
                    .into_iter()
                    .map(|entry| (entry.id, Outcome::Deferred))
                    .collect(),
                busy: false,
            };
        }
        Err(error) => {
            let message = error.message();
            for entry in &pending {
                let _ = store.note_attempt(entry.id, Some(&message), now);
            }
            return FlushReport {
                outcomes: pending
                    .into_iter()
                    .map(|entry| (entry.id, Outcome::Deferred))
                    .collect(),
                busy: false,
            };
        }
    };
    let parsed: Value = match serde_json::from_slice(&reply.body) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("sync response was not JSON: {error}");
            return FlushReport {
                outcomes: pending
                    .into_iter()
                    .map(|entry| (entry.id, Outcome::Deferred))
                    .collect(),
                busy: false,
            };
        }
    };
    let conflicts = parsed
        .get("conflicts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut outcomes = Vec::with_capacity(pending.len());
    for entry in pending {
        let ack = parsed
            .get("acknowledgements")
            .and_then(Value::as_array)
            .and_then(|acks| {
                acks.iter().find(|ack| {
                    ack.get("operation_id").and_then(Value::as_str)
                        == Some(entry.operation_id.as_str())
                })
            });
        let Some(ack) = ack else {
            outcomes.push((entry.id, Outcome::Deferred));
            continue;
        };
        let status = ack
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("rejected");
        let revision = ack
            .get("revision")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<i64>().ok());
        let outcome = match status {
            "accepted" | "duplicate" => store
                .mark_sent(&entry, revision, now)
                .map(|_| Outcome::Sent { revision }),
            "conflict" => {
                let current = conflicts.iter().find(|conflict| {
                    conflict.get("operation_id").and_then(Value::as_str)
                        == Some(entry.operation_id.as_str())
                });
                let current = current.map(|conflict| {
                    let mut item = conflict
                        .get("canonical_fields")
                        .cloned()
                        .unwrap_or(Value::Null);
                    if let Some(object) = item.as_object_mut() {
                        if !object.contains_key("id") {
                            if let Some(entity_id) = conflict.get("entity_id") {
                                object.insert("id".into(), entity_id.clone());
                            }
                        }
                        if let Some(revision) = conflict.get("canonical_revision") {
                            object.insert("revision".into(), revision.clone());
                        }
                    }
                    serde_json::json!({ "item": item, "state": null })
                });
                store
                    .set_state(
                        entry.id,
                        "conflict",
                        ack.get("error").and_then(Value::as_str),
                        current.as_ref(),
                        now,
                    )
                    .map(|_| Outcome::Conflict)
            }
            _ => store
                .set_state(
                    entry.id,
                    "failed",
                    ack.get("error").and_then(Value::as_str),
                    None,
                    now,
                )
                .map(|_| Outcome::Failed),
        };
        outcomes.push((entry.id, outcome.unwrap_or(Outcome::Deferred)));
    }
    if outcomes
        .iter()
        .any(|(_, outcome)| matches!(outcome, Outcome::Sent { .. }))
    {
        let refresh = Outgoing {
            method: "GET".into(),
            path: INVENTORY_PATH.into(),
            headers: vec![("accept".into(), "application/json".into())],
            body: None,
        };
        if let Ok(reply) = t.send(&refresh, usize::MAX).await {
            if is_success(reply.status) {
                let _ = store.put_snapshot(INVENTORY_PATH, Kind::Text, &reply, now);
            }
        }
    }
    FlushReport {
        outcomes,
        busy: false,
    }
}

async fn send_entry<T: Transport>(
    store: &LocalStore,
    t: &T,
    entry: &OutboxEntry,
    now: i64,
) -> Outcome {
    let request = Outgoing {
        method: entry.method.clone(),
        path: entry.path.clone(),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("if-match".into(), entry.if_match.clone()),
        ],
        body: Some(entry.body.to_string()),
    };
    let result = t.send(&request, usize::MAX).await;
    record_reach(store, &result, now);
    let reply = match result {
        Ok(reply) => reply,
        Err(e) => {
            let _ = store.note_attempt(entry.id, Some(&e.message()), now);
            return Outcome::Deferred;
        }
    };
    let text = String::from_utf8_lossy(&reply.body).into_owned();
    let parsed: Option<Value> = serde_json::from_str(&text).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "HTTP {}: {}",
                reply.status,
                text.chars().take(300).collect::<String>()
            )
        });
    let written = match reply.status {
        s if is_success(s) => {
            let revision = parsed
                .as_ref()
                .and_then(|v| v.get("revision"))
                .and_then(Value::as_i64);
            store
                .mark_sent(entry, revision, now)
                .map(|_| Outcome::Sent { revision })
        }
        409 => {
            let current = parsed.as_ref().and_then(|v| v.get("current"));
            if already_applied(entry, current) {
                let revision = current
                    .and_then(|c| c.pointer("/item/revision"))
                    .and_then(Value::as_i64);
                store
                    .delete_entry(entry.id)
                    .map(|_| Outcome::Sent { revision })
            } else {
                store
                    .set_state(entry.id, "conflict", Some(&message), current, now)
                    .map(|_| Outcome::Conflict)
            }
        }
        400..=499 => store
            .set_state(entry.id, "failed", Some(&message), None, now)
            .map(|_| Outcome::Failed),
        _ => store
            .note_attempt(entry.id, Some(&message), now)
            .map(|_| Outcome::Deferred),
    };
    written.unwrap_or_else(|e| {
        log::warn!("{e}");
        Outcome::Deferred
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    type Handler = Box<dyn FnMut(&Outgoing) -> Result<Reply, SendError> + Send>;

    struct Fake {
        handler: Mutex<Handler>,
        calls: Mutex<Vec<Outgoing>>,
    }

    impl Fake {
        fn new(
            handler: impl FnMut(&Outgoing) -> Result<Reply, SendError> + Send + 'static,
        ) -> Self {
            Self {
                handler: Mutex::new(Box::new(handler)),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn offline() -> Self {
            Self::new(|_| {
                Err(SendError::Unreachable(
                    "mac-bridge: the Mac did not answer".into(),
                ))
            })
        }
        fn calls(&self) -> Vec<Outgoing> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        async fn send(&self, request: &Outgoing, max_bytes: usize) -> Result<Reply, SendError> {
            self.calls.lock().unwrap().push(request.clone());
            let reply = (self.handler.lock().unwrap())(request)?;
            if reply.body.len() > max_bytes {
                return Err(SendError::Failed("too large".into()));
            }
            Ok(reply)
        }
    }

    fn json(status: u16, body: Value) -> Result<Reply, SendError> {
        Ok(Reply {
            status,
            content_type: Some("application/json".into()),
            body: body.to_string().into_bytes(),
        })
    }

    fn get(path: &str) -> Outgoing {
        Outgoing {
            method: "GET".into(),
            path: path.into(),
            headers: vec![],
            body: None,
        }
    }

    fn write(method: &str, id: &str, body: Value, if_match: Option<&str>) -> Outgoing {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(rev) = if_match {
            headers.push(("If-Match".into(), rev.into()));
        }
        Outgoing {
            method: method.into(),
            path: format!("{ITEM_PREFIX}{id}"),
            headers,
            body: Some(body.to_string()),
        }
    }

    fn store() -> (tempfile_dir::Dir, LocalStore) {
        let dir = tempfile_dir::Dir::new();
        let store = LocalStore::open(&dir.path().join(DB_FILE)).unwrap();
        (dir, store)
    }

    /// A temp directory without a new dependency.
    mod tempfile_dir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};

        pub struct Dir(PathBuf);
        static N: AtomicU64 = AtomicU64::new(0);
        impl Dir {
            pub fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "axon-local-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::SeqCst)
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn block<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    fn inventory_body() -> Value {
        serde_json::json!([
            { "item": { "id": "schrank", "label": "Schrank", "b": 100, "revision": 1 }, "state": "owned" },
            { "item": { "id": "lampe", "label": "Lampe", "b": 20, "revision": 4 }, "state": "wanted" }
        ])
    }

    fn online_inventory() -> Fake {
        Fake::new(|req| {
            assert_eq!(req.method, "GET");
            json(200, inventory_body())
        })
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile_dir::Dir::new();
        let path = dir.path().join(DB_FILE);
        {
            let s = LocalStore::open(&path).unwrap();
            s.enqueue(
                "a",
                "PATCH",
                "/interior/api/items/a",
                Map::new(),
                "\"1\"",
                1,
            )
            .unwrap();
        }
        let s = LocalStore::open(&path).unwrap();
        let version: i64 = s
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(s.entries().unwrap().len(), 1, "a reopen keeps the data");
        drop(s);
        assert!(LocalStore::open(&path).is_ok(), "a third open runs nothing");
    }

    #[test]
    fn snapshot_stored_on_success_and_served_offline_with_marker() {
        let (_d, s) = store();
        let online = block(request_text(
            &s,
            &online_inventory(),
            get(INVENTORY_PATH),
            1_000,
        ))
        .unwrap();
        assert!(!online.stale);
        assert!(!s.reach().offline);

        let offline = block(request_text(
            &s,
            &Fake::offline(),
            get(INVENTORY_PATH),
            9_000,
        ))
        .unwrap();
        assert!(offline.stale);
        assert_eq!(offline.fetched_at, Some(1_000));
        assert_eq!(offline.status, 200);
        assert_eq!(
            serde_json::from_str::<Value>(&offline.body).unwrap(),
            inventory_body()
        );
        let reach = s.reach();
        assert!(reach.offline);
        assert_eq!(reach.offline_since, Some(9_000));
        assert_eq!(reach.showing_from, Some(1_000));
        assert_eq!(s.status().unwrap().showing_from, Some(1_000));

        // The Mac answers again: the offline marker clears.
        block(request_text(
            &s,
            &online_inventory(),
            get(INVENTORY_PATH),
            10_000,
        ))
        .unwrap();
        assert_eq!(s.reach(), Reach::default());
    }

    #[test]
    fn an_http_error_is_not_offline_and_is_not_kept() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &online_inventory(),
            get(INVENTORY_PATH),
            1,
        ))
        .unwrap();
        let err = Fake::new(|_| json(500, serde_json::json!({"error": "boom"})));
        let answer = block(request_text(&s, &err, get(INVENTORY_PATH), 2)).unwrap();
        assert_eq!(answer.status, 500);
        assert!(!answer.stale);
        let snap = s
            .get_snapshot(INVENTORY_PATH, Kind::Text, 3)
            .unwrap()
            .unwrap();
        assert_eq!(snap.fetched_at, 1, "the 500 did not replace the good copy");
        // A non-network failure never falls back to the copy.
        let failed = Fake::new(|_| Err(SendError::Failed("tls".into())));
        assert!(block(request_text(&s, &failed, get(INVENTORY_PATH), 4)).is_err());
    }

    #[test]
    fn non_projected_path_is_not_cached_and_fails_offline() {
        let (_d, s) = store();
        for path in [
            "/comms/feed?limit=5",
            "/finance/api/accounts",
            "/places/api/places",
        ] {
            let ok = Fake::new(|_| json(200, serde_json::json!({"x": 1})));
            block(request_text(&s, &ok, get(path), 1)).unwrap();
            assert!(
                s.get_snapshot(path, Kind::Text, 2).unwrap().is_none(),
                "{path}"
            );
            assert!(
                block(request_text(&s, &Fake::offline(), get(path), 3)).is_err(),
                "{path}"
            );
        }
        assert!(is_c1("/interior/api/items?x=1"));
        assert!(!is_c1("/interior/api/"));
        assert!(!is_c1("/interiorx/api/items"));
    }

    #[test]
    fn media_bytes_are_cached_under_the_cap_and_served_as_203() {
        let (_d, s) = store();
        let path = "/interior/api/media/lamp.jpg";
        let ok = Fake::new(|_| {
            Ok(Reply {
                status: 200,
                content_type: Some("image/jpeg".into()),
                body: vec![1, 2, 3],
            })
        });
        let a = block(request_bytes(&s, &ok, get(path), usize::MAX, 5)).unwrap();
        assert!(!a.stale);
        let b = block(request_bytes(
            &s,
            &Fake::offline(),
            get(path),
            usize::MAX,
            6,
        ))
        .unwrap();
        assert!(b.stale);
        assert_eq!(b.reply.status, STALE_BYTES_STATUS);
        assert_eq!(b.reply.body, vec![1, 2, 3]);
        assert_eq!(b.reply.content_type.as_deref(), Some("image/jpeg"));

        let big = Reply {
            status: 200,
            content_type: None,
            body: vec![0; MAX_SNAPSHOT_ENTRY_BYTES + 1],
        };
        assert!(!s.put_snapshot(path, Kind::Bytes, &big, 7).unwrap());
        assert!(
            s.get_snapshot(path, Kind::Bytes, 8).unwrap().is_none(),
            "an oversized answer also drops the older copy"
        );
    }

    #[test]
    fn media_eviction_drops_the_least_recently_used() {
        let (_d, s) = store();
        let chunk = |n: u8| Reply {
            status: 200,
            content_type: None,
            body: vec![n; MAX_SNAPSHOT_ENTRY_BYTES],
        };
        // 10 × 20 MiB fill the 200 MiB exactly; the 11th pushes the oldest out.
        for n in 0..10u8 {
            s.put_snapshot(
                &format!("/interior/api/media/{n}"),
                Kind::Bytes,
                &chunk(n),
                n as i64,
            )
            .unwrap();
        }
        // Reading 0 makes it recent; 1 is now the least recently used.
        s.get_snapshot("/interior/api/media/0", Kind::Bytes, 100)
            .unwrap();
        s.put_snapshot("/interior/api/media/new", Kind::Bytes, &chunk(99), 101)
            .unwrap();
        assert!(s
            .get_snapshot("/interior/api/media/1", Kind::Bytes, 102)
            .unwrap()
            .is_none());
        assert!(s
            .get_snapshot("/interior/api/media/0", Kind::Bytes, 103)
            .unwrap()
            .is_some());
        assert!(s
            .get_snapshot("/interior/api/media/new", Kind::Bytes, 104)
            .unwrap()
            .is_some());
    }

    #[test]
    fn write_is_queued_only_with_if_match() {
        let (_d, s) = store();
        let without = block(request_text(
            &s,
            &Fake::offline(),
            write("PATCH", "schrank", serde_json::json!({"b": 120}), None),
            1,
        ));
        let message = without.unwrap_err();
        assert!(message.contains("not queued"), "{message}");
        assert!(s.entries().unwrap().is_empty());

        let with = block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            2,
        ))
        .unwrap();
        assert_eq!(with.status, QUEUED_STATUS);
        let body: Value = serde_json::from_str(&with.body).unwrap();
        assert_eq!(body["queued"], true);
        let entries = s.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(body["outbox_id"], entries[0].id);
        assert_eq!(entries[0].if_match, "\"1\"");
        assert_eq!(entries[0].state, "pending");

        // Other writes offline fail as before and queue nothing.
        let post = Outgoing {
            method: "POST".into(),
            path: "/interior/api/items".into(),
            headers: vec![("If-Match".into(), "\"1\"".into())],
            body: Some("{}".into()),
        };
        assert!(block(request_text(&s, &Fake::offline(), post, 3)).is_err());
        let state = write(
            "PATCH",
            "schrank/state",
            serde_json::json!({}),
            Some("\"1\""),
        );
        assert!(block(request_text(&s, &Fake::offline(), state, 4)).is_err());
        assert_eq!(s.entries().unwrap().len(), 1);
    }

    #[test]
    fn coalescing_merges_patches_and_keeps_the_first_if_match() {
        let (_d, s) = store();
        let off = Fake::offline();
        block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120, "label": "A"}),
                Some("\"1\""),
            ),
            1,
        ))
        .unwrap();
        let second = block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"label": "B", "h": 200}),
                Some("\"7\""),
            ),
            2,
        ))
        .unwrap();
        let entries = s.entries().unwrap();
        assert_eq!(entries.len(), 1);
        let second: Value = serde_json::from_str(&second.body).unwrap();
        assert_eq!(second["outbox_id"], entries[0].id);
        assert_eq!(entries[0].method, "PATCH");
        assert_eq!(entries[0].if_match, "\"1\"");
        assert_eq!(
            entries[0].body,
            serde_json::json!({"b": 120, "label": "B", "h": 200})
        );

        // A PUT replaces the pending body and keeps the original If-Match.
        block(request_text(
            &s,
            &off,
            write(
                "PUT",
                "schrank",
                serde_json::json!({"id": "schrank", "label": "C"}),
                Some("\"9\""),
            ),
            3,
        ))
        .unwrap();
        let entries = s.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "PUT");
        assert_eq!(entries[0].if_match, "\"1\"");
        assert_eq!(
            entries[0].body,
            serde_json::json!({"id": "schrank", "label": "C"})
        );

        // A PATCH after a PUT merges into the PUT, which stays a PUT.
        block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 5}),
                Some("\"1\""),
            ),
            4,
        ))
        .unwrap();
        let entries = s.entries().unwrap();
        assert_eq!(entries[0].method, "PUT");
        assert_eq!(
            entries[0].body,
            serde_json::json!({"id": "schrank", "label": "C", "b": 5})
        );
    }

    #[test]
    fn overlay_shows_pending_fields_offline_and_online() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &online_inventory(),
            get(INVENTORY_PATH),
            1,
        ))
        .unwrap();
        block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            2,
        ))
        .unwrap();

        for fake in [Fake::offline(), online_inventory()] {
            let answer = block(request_text(&s, &fake, get(INVENTORY_PATH), 3)).unwrap();
            let rows: Value = serde_json::from_str(&answer.body).unwrap();
            assert_eq!(rows[0]["item"]["b"], 120);
            assert_eq!(rows[0]["item"]["pending"], true);
            assert_eq!(rows[0]["pending"], true);
            assert_eq!(
                rows[0]["item"]["revision"], 1,
                "the revision stays the Mac's"
            );
            assert_eq!(rows[1]["item"]["b"], 20);
            assert!(rows[1].get("pending").is_none());
        }
        // The stored copy stays the Mac's answer; the overlay is applied on read.
        let snap = s
            .get_snapshot(INVENTORY_PATH, Kind::Text, 4)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&snap.body).unwrap(),
            inventory_body()
        );

        // The wishlist shape is overlaid too.
        let wish = serde_json::json!({"items": [{"id": "schrank", "b": 100, "revision": 1}]});
        let out: Value =
            serde_json::from_str(&s.overlay(WISHLIST_PATH, &wish.to_string())).unwrap();
        assert_eq!(out["items"][0]["b"], 120);
        assert_eq!(out["items"][0]["pending"], true);
    }

    #[test]
    fn flush_sends_in_creation_order_and_refreshes_the_inventory() {
        let (_d, s) = store();
        let off = Fake::offline();
        block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "b-second",
                serde_json::json!({"b": 2}),
                Some("\"3\""),
            ),
            1,
        ))
        .unwrap();
        block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "a-third",
                serde_json::json!({"b": 3}),
                Some("\"4\""),
            ),
            2,
        ))
        .unwrap();
        // Coalescing into the first entry does not move it back in the order.
        block(request_text(
            &s,
            &off,
            write(
                "PATCH",
                "b-second",
                serde_json::json!({"h": 9}),
                Some("\"3\""),
            ),
            3,
        ))
        .unwrap();

        let ok = Fake::new(|req| match req.method.as_str() {
            "GET" => json(200, inventory_body()),
            _ => json(200, serde_json::json!({"ok": true, "revision": 10})),
        });
        let report = block(flush(&s, &ok, 10));
        assert_eq!(report.outcomes.len(), 2);
        assert!(report
            .outcomes
            .iter()
            .all(|(_, o)| matches!(o, Outcome::Sent { revision: Some(10) })));
        let calls = ok.calls();
        assert_eq!(calls[0].path, "/interior/api/items/b-second");
        assert_eq!(calls[1].path, "/interior/api/items/a-third");
        assert_eq!(calls[2].method, "GET");
        assert_eq!(calls[2].path, INVENTORY_PATH);
        assert_eq!(header(&calls[0].headers, "if-match"), Some("\"3\""));
        assert_eq!(
            serde_json::from_str::<Value>(calls[0].body.as_deref().unwrap()).unwrap(),
            serde_json::json!({"b": 2, "h": 9})
        );
        assert!(s.entries().unwrap().is_empty());
        assert_eq!(
            s.get_snapshot(INVENTORY_PATH, Kind::Text, 11)
                .unwrap()
                .unwrap()
                .fetched_at,
            10
        );
    }

    #[test]
    fn conflict_keeps_the_macs_item_and_is_never_retried() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            1,
        ))
        .unwrap();
        let current = serde_json::json!({"item": {"id": "schrank", "b": 90, "revision": 2}, "state": "owned"});
        let answer = current.clone();
        let mac = Fake::new(move |_| {
            json(
                409,
                serde_json::json!({"error": "veraltet", "current": answer}),
            )
        });
        let report = block(flush(&s, &mac, 2));
        assert_eq!(report.outcomes[0].1, Outcome::Conflict);
        let entry = &s.entries().unwrap()[0];
        assert_eq!(entry.state, "conflict");
        assert_eq!(entry.current.as_ref(), Some(&current));
        assert_eq!(entry.error.as_deref(), Some("veraltet"));
        assert_eq!(s.status().unwrap().conflicts, 1);

        // Neither a second flush nor the background loop sends it again.
        let report = block(flush(&s, &mac, 3));
        assert!(report.outcomes.is_empty());
        assert_eq!(mac.calls().len(), 1);
        assert!(!s.has_pending());

        // The inventory marks the item; the Mac's values stand.
        let rows: Value =
            serde_json::from_str(&s.overlay(INVENTORY_PATH, &inventory_body().to_string()))
                .unwrap();
        assert_eq!(rows[0]["conflict"], true);
        assert_eq!(rows[0]["item"]["b"], 100);
    }

    #[test]
    fn a_409_that_already_holds_the_patch_counts_as_sent() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            1,
        ))
        .unwrap();
        let mac = Fake::new(|req| match req.method.as_str() {
            "GET" => json(200, inventory_body()),
            _ => json(
                409,
                serde_json::json!({"error": "x", "current": {"item": {"id": "schrank", "b": 120, "revision": 2}, "state": null}}),
            ),
        });
        let report = block(flush(&s, &mac, 2));
        assert_eq!(report.outcomes[0].1, Outcome::Sent { revision: Some(2) });
        assert!(s.entries().unwrap().is_empty());
    }

    #[test]
    fn other_4xx_fails_and_network_error_keeps_pending() {
        let (_d, s) = store();
        let off = Fake::offline();
        block(request_text(
            &s,
            &off,
            write("PATCH", "a", serde_json::json!({"b": 1}), Some("\"1\"")),
            1,
        ))
        .unwrap();
        block(request_text(
            &s,
            &off,
            write("PATCH", "b", serde_json::json!({"b": 1}), Some("\"1\"")),
            2,
        ))
        .unwrap();

        let report = block(flush(&s, &off, 3));
        assert_eq!(
            report.outcomes,
            vec![(1, Outcome::Deferred)],
            "stops at the first unanswered entry"
        );
        assert!(s.entries().unwrap().iter().all(|e| e.state == "pending"));
        assert_eq!(s.entries().unwrap()[0].attempts, 1);
        assert_eq!(s.status().unwrap().pending, 2);

        let bad = Fake::new(|req| {
            if req.path.ends_with("/a") {
                json(400, serde_json::json!({"error": "`x` ist kein Feld"}))
            } else {
                json(503, serde_json::json!({}))
            }
        });
        let report = block(flush(&s, &bad, 4));
        assert_eq!(
            report.outcomes,
            vec![(1, Outcome::Failed), (2, Outcome::Deferred)]
        );
        let entries = s.entries().unwrap();
        assert_eq!(entries[0].state, "failed");
        assert_eq!(entries[0].error.as_deref(), Some("`x` ist kein Feld"));
        assert_eq!(entries[1].state, "pending", "a 5xx is retried later");
    }

    #[test]
    fn keep_mine_resends_with_the_macs_current_revision() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            1,
        ))
        .unwrap();
        let conflict = Fake::new(|_| {
            json(
                409,
                serde_json::json!({"error": "x", "current": {"item": {"id": "schrank", "b": 90, "revision": 5}, "state": "owned"}}),
            )
        });
        block(flush(&s, &conflict, 2));
        let id = s.entries().unwrap()[0].id;

        s.keep_mine(id, 3).unwrap();
        let entry = s.entry(id).unwrap().unwrap();
        assert_eq!(entry.state, "pending");
        assert_eq!(entry.if_match, "\"5\"");
        assert!(entry.current.is_none());

        let mac = Fake::new(|req| match req.method.as_str() {
            "GET" => json(200, inventory_body()),
            _ => json(200, serde_json::json!({"ok": true, "revision": 6})),
        });
        block(flush(&s, &mac, 4));
        assert_eq!(header(&mac.calls()[0].headers, "if-match"), Some("\"5\""));
        assert!(s.entries().unwrap().is_empty());

        // Keep mine refuses anything that is not a conflict.
        let pending = s
            .enqueue(
                "x",
                "PATCH",
                "/interior/api/items/x",
                Map::new(),
                "\"1\"",
                5,
            )
            .unwrap();
        assert!(s.keep_mine(pending, 6).is_err());
        s.discard(pending).unwrap();
        assert!(s.discard(pending).is_err());
    }

    #[test]
    fn a_write_while_an_edit_waits_joins_it_and_flushes() {
        let (_d, s) = store();
        block(request_text(
            &s,
            &Fake::offline(),
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"b": 120}),
                Some("\"1\""),
            ),
            1,
        ))
        .unwrap();
        let mac = Fake::new(|req| match req.method.as_str() {
            "GET" => json(200, inventory_body()),
            _ => json(200, serde_json::json!({"ok": true, "revision": 2})),
        });
        let answer = block(request_text(
            &s,
            &mac,
            write(
                "PATCH",
                "schrank",
                serde_json::json!({"h": 3}),
                Some("\"1\""),
            ),
            2,
        ))
        .unwrap();
        assert_eq!(answer.status, QUEUED_STATUS);
        let body: Value = serde_json::from_str(&answer.body).unwrap();
        assert_eq!(body["state"], "sent");
        assert_eq!(body["revision"], 2);
        let sent = &mac.calls()[0];
        assert_eq!(
            serde_json::from_str::<Value>(sent.body.as_deref().unwrap()).unwrap(),
            serde_json::json!({"b": 120, "h": 3})
        );
        assert!(s.entries().unwrap().is_empty());
    }

    #[test]
    fn an_edit_folded_in_while_sending_is_kept_against_the_new_revision() {
        let (_d, s) = store();
        let off = Fake::offline();
        let first = write(
            "PATCH",
            "schrank",
            serde_json::json!({"b": 120}),
            Some("\"1\""),
        );
        block(request_text(&s, &off, first, 1)).unwrap();
        let sent = s.pending().unwrap().remove(0);
        // The operator edits again while that body is on its way.
        s.enqueue(
            "schrank",
            "PATCH",
            &sent.path,
            serde_json::json!({"h": 7}).as_object().unwrap().clone(),
            "\"1\"",
            2,
        )
        .unwrap();
        s.mark_sent(&sent, Some(2), 3).unwrap();
        let left = s.entries().unwrap();
        assert_eq!(left.len(), 1, "the folded edit is not lost");
        assert_eq!(left[0].if_match, "\"2\"");
        assert_eq!(left[0].state, "pending");

        s.mark_sent(&left[0], Some(3), 4).unwrap();
        assert!(s.entries().unwrap().is_empty());
    }

    #[test]
    fn a_second_flush_while_one_runs_does_nothing() {
        let (_d, s) = store();
        s.flushing.store(true, Ordering::SeqCst);
        assert!(block(flush(&s, &Fake::offline(), 1)).busy);
        s.flushing.store(false, Ordering::SeqCst);
        assert!(!block(flush(&s, &Fake::offline(), 2)).busy);
    }

    #[test]
    fn item_ids_are_read_from_the_item_path_only() {
        assert_eq!(
            item_id_of("/interior/api/items/schrank").as_deref(),
            Some("schrank")
        );
        assert_eq!(
            item_id_of("/interior/api/items/k%C3%BCche").as_deref(),
            Some("küche")
        );
        assert_eq!(item_id_of("/interior/api/items/a/state"), None);
        assert_eq!(item_id_of("/interior/api/items/"), None);
        assert_eq!(item_id_of("/interior/api/items/a?x=1"), None);
        assert_eq!(item_id_of("/interior/api/items/%ZZ"), None);
        assert_eq!(item_id_of("/finance/api/items/a"), None);
    }

    // ─── Trips offline (PRD Q115, 2026-09-25) ───

    /// 2026-09-25 12:00 UTC.
    const TODAY_MS: i64 = 1_790_337_600_000;

    fn plan(id: &str, start: &str, end: &str) -> Value {
        serde_json::json!({ "id": id, "title": id, "date_start": start, "date_end": end })
    }

    fn plans_online() -> Fake {
        Fake::new(|req| {
            let route = route_of(&req.path).to_string();
            if route == TRIPS_LIST {
                return json(
                    200,
                    serde_json::json!([
                        plan("trip:plan:berlin", "2026-10-07", "2026-10-13"),
                        plan("trip:plan:old", "2026-08-16", "2026-08-19"),
                    ]),
                );
            }
            if route == "/trips/api/plans/trip%3Aplan%3Aberlin" {
                let mut detail = plan("trip:plan:berlin", "2026-10-07", "2026-10-13");
                detail["items"] = serde_json::json!([{ "item_type": "booking", "payload": { "order_ref": "4711" } }]);
                return json(200, detail);
            }
            json(404, serde_json::json!({ "error": "no plan" }))
        })
    }

    #[test]
    fn utc_date_matches_the_calendar() {
        assert_eq!(utc_date(TODAY_MS), "2026-09-25");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400_000), "2000-02-29");
    }

    #[test]
    fn bounded_offline_projections_are_explicit() {
        for path in [
            "/vault/api/tasks",
            "/vault/api/people",
            "/calendar/api/entries",
            "/knowledge-graph/api/graph/unit/comms",
            "/trips/api/plans",
        ] {
            assert!(is_offline_projection(path), "{path}");
        }
        for path in [
            "/calendar/api/google/import",
            "/knowledge-graph/api/graph",
            "/knowledge-graph/api/graph/stats",
        ] {
            assert!(!is_offline_projection(path), "{path}");
        }
    }

    #[test]
    fn only_the_list_and_a_plan_detail_are_trip_paths() {
        assert!(is_trip_offline("/trips/api/plans"));
        assert!(is_trip_offline("/trips/api/plans/trip%3Aplan%3A1"));
        assert!(!is_trip_offline("/trips/api/plans/trip%3Aplan%3A1/items"));
        assert!(!is_trip_offline("/trips/api/plans/trip%3Aplan%3A1/cost"));
        assert!(!is_trip_offline("/trips/api/places"));
        assert!(!is_trip_offline("/calendar/api/entries"));
    }

    #[test]
    fn a_plan_ends_after_its_last_day_with_a_day_of_slack() {
        assert!(!plan_ended(
            &plan("p", "2026-10-07", "2026-10-13"),
            TODAY_MS
        ));
        assert!(!plan_ended(
            &plan("p", "2026-09-20", "2026-09-24"),
            TODAY_MS
        ));
        assert!(plan_ended(&plan("p", "2026-09-20", "2026-09-23"), TODAY_MS));
        assert!(!plan_ended(&serde_json::json!({ "id": "p" }), TODAY_MS));
    }

    #[test]
    fn upcoming_plans_are_readable_offline_without_being_opened() {
        let (_dir, store) = store();
        block(request_text(
            &store,
            &plans_online(),
            get(TRIPS_LIST),
            TODAY_MS,
        ))
        .unwrap();

        let offline = Fake::offline();
        let list = block(request_text(&store, &offline, get(TRIPS_LIST), TODAY_MS)).unwrap();
        assert!(list.stale);
        let ids: Vec<String> = serde_json::from_str::<Vec<Value>>(&list.body)
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["trip:plan:berlin"], "an ended plan is not kept");

        let detail = block(request_text(
            &store,
            &offline,
            get("/trips/api/plans/trip%3Aplan%3Aberlin"),
            TODAY_MS,
        ))
        .unwrap();
        assert!(detail.stale);
        assert!(
            detail.body.contains("4711"),
            "the booking reference is readable offline"
        );

        let ended = block(request_text(
            &store,
            &offline,
            get("/trips/api/plans/trip%3Aplan%3Aold"),
            TODAY_MS,
        ));
        assert!(ended.is_err(), "an ended plan has no offline copy");
    }

    #[test]
    fn a_plan_gone_from_the_list_loses_its_offline_copy() {
        let (_dir, store) = store();
        block(request_text(
            &store,
            &plans_online(),
            get(TRIPS_LIST),
            TODAY_MS,
        ))
        .unwrap();
        let path = "/trips/api/plans/trip%3Aplan%3Aberlin";
        assert!(store.snapshot_fetched_at(path, Kind::Text).is_some());

        let empty = Fake::new(|_| json(200, serde_json::json!([])));
        block(request_text(&store, &empty, get(TRIPS_LIST), TODAY_MS)).unwrap();
        assert!(store.snapshot_fetched_at(path, Kind::Text).is_none());
    }

    #[test]
    fn a_fresh_detail_is_not_fetched_again() {
        let (_dir, store) = store();
        let fake = plans_online();
        block(request_text(&store, &fake, get(TRIPS_LIST), TODAY_MS)).unwrap();
        block(request_text(
            &store,
            &fake,
            get(TRIPS_LIST),
            TODAY_MS + 60_000,
        ))
        .unwrap();
        let detail_calls = fake
            .calls()
            .iter()
            .filter(|c| trip_plan_id(&c.path).is_some())
            .count();
        assert_eq!(detail_calls, 1);
    }

    #[test]
    fn trip_writes_are_never_queued() {
        let (_dir, store) = store();
        let request = Outgoing {
            method: "PATCH".into(),
            path: "/trips/api/plans/trip%3Aplan%3Aberlin".into(),
            headers: vec![("If-Match".into(), "1".into())],
            body: Some("{}".into()),
        };
        assert!(block(request_text(&store, &Fake::offline(), request, TODAY_MS)).is_err());
        assert!(store.entries().unwrap().is_empty());
    }
}

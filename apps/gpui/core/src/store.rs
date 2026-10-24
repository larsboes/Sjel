//! Turso / LibSQL local storage and offline replica manager.
//!
//! Provides direct on-device storage for both the desktop app and the standalone iOS client.
//! Reads and writes operate against local storage with microsecond latency, enabling full
//! offline viewing, editing, and task management when disconnected from the Mac host.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::shell::{CalendarEntry, ProjectRecord, ShellError};

/// Specific errors for Turso / LibSQL local storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Libsql(String),
    Json(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Libsql(err) => write!(f, "Turso database error: {err}"),
            Self::Json(err) => write!(f, "Payload serialization error: {err}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<StoreError> for ShellError {
    fn from(err: StoreError) -> Self {
        ShellError::Refused(500, Some(err.to_string()))
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Database table for project and task records matching Sjel schema.
///
/// `synced_content` and `remote_at` describe the Mac's copy of a record: the content it held when
/// this row was last reconciled with it, and the stamp the Mac gave that version. A row whose
/// `synced_content` is NULL has never been on the Mac, which is what tells a sync to create it
/// there instead of replacing it. They are separate columns, not derived, because a local edit
/// moves `content` and must leave the base it will push against alone.
const SCHEMA_PROJECTS: &str = r#"
CREATE TABLE IF NOT EXISTS projects_records (
    path TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    due TEXT,
    priority INTEGER NOT NULL DEFAULT 2,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    synced_content TEXT,
    remote_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_projects_records_project ON projects_records(project);
CREATE INDEX IF NOT EXISTS idx_projects_records_done ON projects_records(done);
"#;

/// The columns an existing database from before delta sync does not have yet.
const ADDED_COLUMNS: &[(&str, &str)] = &[("synced_content", "TEXT"), ("remote_at", "TEXT")];

/// Offline mutation log to replay deltas when reconnected to the host.
///
/// `domain` names which capability the write belongs to, because the two replays are not
/// interchangeable: a record replays against a path the Mac names and may rekey, an entry replays
/// against an id the Mac assigned or the phone minted. One table keeps one order and one badge;
/// [`crate::sync`] pushes per domain anyway, because ordering only has to hold inside a domain.
const SCHEMA_MUTATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS _offline_mutations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    domain TEXT NOT NULL DEFAULT 'projects',
    path TEXT NOT NULL,
    action TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

/// The columns an existing mutation log from before the second domain does not have yet.
const ADDED_MUTATION_COLUMNS: &[(&str, &str)] = &[("domain", "TEXT NOT NULL DEFAULT 'projects'")];

/// The captures typed on this phone, and the Mac's answer once it took one.
///
/// A capture is the one record whose whole purpose is to survive being offline: it is written here
/// first, and the queue carries it to `POST /comms/captures` later. `remote_id` is the Mac's id once
/// it answered, which is also how the page knows a capture has landed without asking the Mac.
const SCHEMA_CAPTURES: &str = r#"
CREATE TABLE IF NOT EXISTS captures (
    id TEXT PRIMARY KEY,
    body TEXT,
    url TEXT,
    tag TEXT,
    remote_id TEXT,
    status TEXT,
    routed_to TEXT,
    client TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

/// The columns a `captures` table written before the pull does not have yet.
const ADDED_CAPTURE_COLUMNS: &[(&str, &str)] = &[
    ("status", "TEXT"),
    ("routed_to", "TEXT"),
    ("client", "TEXT"),
];

/// The writes the Mac refused, one row per record, so the reason outlives the launch that heard it.
///
/// A refusal is not a lost write: the record and its queued mutation are both still here, and this
/// table is what lets the page say which record it is and why, hours after the sync that found it.
/// It is cleared when that record finally goes through ([`crate::sync`]) or when the record is
/// removed, and it is never a decision of its own: nothing here renames, deletes or edits a record
/// to make a refusal go away.
const SCHEMA_REFUSALS: &str = r#"
CREATE TABLE IF NOT EXISTS _sync_refusals (
    path TEXT PRIMARY KEY,
    reason TEXT NOT NULL,
    at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

/// The calendar's own replica: the entries the phone holds, and the cursor it has read to.
///
/// `remote_at` is the Mac's `updated_at` stamp for the entry, which is also the change feed's
/// cursor unit, so a row reconciled at stamp N is skipped by every pull that asks from N.
const SCHEMA_CALENDAR: &str = r#"
CREATE TABLE IF NOT EXISTS calendar_entries (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL DEFAULT 'event',
    commitment TEXT NOT NULL DEFAULT 'possible',
    title TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 1,
    location TEXT,
    notes TEXT,
    source TEXT NOT NULL DEFAULT 'manual',
    remote_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_calendar_entries_starts ON calendar_entries(starts_at);
CREATE TABLE IF NOT EXISTS _replica_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// The replica's name for one domain, used by the queue and by the pull cursors.
pub const DOMAIN_PROJECTS: &str = "projects";
pub const DOMAIN_CALENDAR: &str = "calendar";
/// Captures typed on this phone. Its own domain because its replay is its own: a capture has no
/// path and no record, it is one `POST` to the comms capability.
pub const DOMAIN_CAPTURES: &str = "captures";
/// The id a capture made on this phone carries until the Mac assigns one.
pub const LOCAL_CAPTURE_ID_PREFIX: &str = "capture:local:";
/// The key a change feed's cursor is kept under, per domain.
const CURSOR_PREFIX: &str = "cursor:";
/// The id prefix an entry made on this phone carries until the Mac assigns one. A write to such an
/// id creates; a write to any other id updates.
pub const LOCAL_ENTRY_ID_PREFIX: &str = "calendar:local:";

/// Direct Turso/LibSQL database handle held by the application.
#[derive(Clone)]
pub struct TursoStore {
    path: PathBuf,
    conn: Arc<Mutex<Option<libsql::Connection>>>,
}

impl TursoStore {
    /// Opens or connects to a Turso/LibSQL database at `path`.
    pub async fn open(path: impl Into<PathBuf>) -> StoreResult<Self> {
        let path = path.into();
        let store = Self {
            path,
            conn: Arc::new(Mutex::new(None)),
        };
        store.ensure_schema().await?;
        Ok(store)
    }

    /// Connects to an in-memory database for testing or ephemeral caching.
    pub async fn in_memory() -> StoreResult<Self> {
        let store = Self {
            path: PathBuf::from(":memory:"),
            conn: Arc::new(Mutex::new(None)),
        };
        store.ensure_schema().await?;
        Ok(store)
    }

    /// Acquires or initializes the shared connection.
    pub async fn connection(&self) -> StoreResult<libsql::Connection> {
        let mut slot = self.conn.lock().await;
        if let Some(conn) = slot.as_ref() {
            return Ok(conn.clone());
        }
        let db = if self.path.to_string_lossy() == ":memory:" {
            libsql::Builder::new_local(":memory:")
                .build()
                .await
                .map_err(|e| StoreError::Libsql(format!("libsql in-memory failed: {e}")))?
        } else {
            libsql::Builder::new_local(&self.path)
                .build()
                .await
                .map_err(|e| StoreError::Libsql(format!("libsql local failed: {e}")))?
        };
        let conn = db
            .connect()
            .map_err(|e| StoreError::Libsql(format!("libsql connect failed: {e}")))?;
        conn.execute_batch("PRAGMA busy_timeout = 5000;")
            .await
            .map_err(|e| StoreError::Libsql(format!("libsql pragma failed: {e}")))?;
        *slot = Some(conn.clone());
        Ok(conn)
    }

    /// Ensures required offline tables exist, and adds the sync columns to a database that was
    /// opened by a build that did not have them yet.
    pub async fn ensure_schema(&self) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute_batch(SCHEMA_PROJECTS)
            .await
            .map_err(|e| StoreError::Libsql(format!("projects schema init failed: {e}")))?;
        conn.execute_batch(SCHEMA_MUTATIONS)
            .await
            .map_err(|e| StoreError::Libsql(format!("mutations schema init failed: {e}")))?;
        conn.execute_batch(SCHEMA_CALENDAR)
            .await
            .map_err(|e| StoreError::Libsql(format!("calendar schema init failed: {e}")))?;
        conn.execute_batch(SCHEMA_REFUSALS)
            .await
            .map_err(|e| StoreError::Libsql(format!("refusals schema init failed: {e}")))?;
        conn.execute_batch(SCHEMA_CAPTURES)
            .await
            .map_err(|e| StoreError::Libsql(format!("captures schema init failed: {e}")))?;
        self.add_missing_capture_columns(&conn).await?;
        self.add_missing_columns(&conn).await?;
        self.add_missing_mutation_columns(&conn).await
    }

    /// `ALTER TABLE` for each column the capture table did not have when this database was made:
    /// a build that only captured had no use for the Mac's status, and one that pulls has to keep it.
    async fn add_missing_capture_columns(&self, conn: &libsql::Connection) -> StoreResult<()> {
        let mut rows = conn
            .query("SELECT name FROM pragma_table_info('captures')", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("read capture table info failed: {e}")))?;
        let mut present = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            present.push(row.get::<String>(0).unwrap_or_default());
        }
        for (name, kind) in ADDED_CAPTURE_COLUMNS {
            if present.iter().any(|column| column == name) {
                continue;
            }
            conn.execute_batch(&format!("ALTER TABLE captures ADD COLUMN {name} {kind}"))
                .await
                .map_err(|e| StoreError::Libsql(format!("add captures.{name} failed: {e}")))?;
        }
        Ok(())
    }

    /// `ALTER TABLE` for each column the mutation log did not have when this database was made.
    /// A log written before the calendar is a projects-only log, which the default says it is.
    async fn add_missing_mutation_columns(&self, conn: &libsql::Connection) -> StoreResult<()> {
        let mut rows = conn
            .query(
                "SELECT name FROM pragma_table_info('_offline_mutations')",
                (),
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read mutation table info failed: {e}")))?;
        let mut present = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            if let Ok(name) = row.get::<String>(0) {
                present.push(name);
            }
        }
        for (name, kind) in ADDED_MUTATION_COLUMNS {
            if present.iter().any(|have| have == name) {
                continue;
            }
            conn.execute_batch(&format!(
                "ALTER TABLE _offline_mutations ADD COLUMN {name} {kind}"
            ))
            .await
            .map_err(|e| StoreError::Libsql(format!("add mutations.{name} failed: {e}")))?;
        }
        Ok(())
    }

    /// `ALTER TABLE` for each column [`ADDED_COLUMNS`] names that the table does not have.
    async fn add_missing_columns(&self, conn: &libsql::Connection) -> StoreResult<()> {
        let mut rows = conn
            .query("SELECT name FROM pragma_table_info('projects_records')", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("read table info failed: {e}")))?;
        let mut present = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            if let Ok(name) = row.get::<String>(0) {
                present.push(name);
            }
        }
        for (name, kind) in ADDED_COLUMNS {
            if present.iter().any(|have| have == name) {
                continue;
            }
            conn.execute_batch(&format!(
                "ALTER TABLE projects_records ADD COLUMN {name} {kind}"
            ))
            .await
            .map_err(|e| StoreError::Libsql(format!("add {name} column failed: {e}")))?;
        }
        Ok(())
    }

    /// Reads project records for a specific project.
    pub async fn fetch_project_records(&self, project: &str) -> StoreResult<Vec<ProjectRecord>> {
        let conn = self.connection().await?;
        let query = "SELECT path, kind, title, done, due, priority \
                     FROM projects_records \
                     WHERE project = ?1 \
                     ORDER BY done ASC, priority DESC, path ASC";
        let mut rows = conn
            .query(query, libsql::params![project])
            .await
            .map_err(|e| StoreError::Libsql(format!("query project failed: {e}")))?;

        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let path: String = row.get(0).unwrap_or_default();
            let kind: String = row.get(1).unwrap_or_else(|_| "task".into());
            let title: String = row.get(2).unwrap_or_default();
            let done_int: i64 = row.get(3).unwrap_or(0);
            let due: Option<String> = row.get(4).ok();
            let priority_int: i64 = row.get(5).unwrap_or(2);
            let project: String = row.get(6).unwrap_or_default();
            list.push(ProjectRecord {
                project,
                path,
                kind,
                title,
                content: String::new(),
                done: done_int != 0,
                due,
                priority: priority_int.clamp(1, 3) as u8,
            });
        }
        Ok(list)
    }

    /// Reads all records across projects ordered by done, priority, updated_at.
    pub async fn fetch_all_records(&self) -> StoreResult<Vec<ProjectRecord>> {
        let conn = self.connection().await?;
        let query = "SELECT path, kind, title, done, due, priority, project \
                     FROM projects_records \
                     ORDER BY done ASC, priority DESC, updated_at DESC";
        let mut rows = conn
            .query(query, ())
            .await
            .map_err(|e| StoreError::Libsql(format!("query all records failed: {e}")))?;

        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let path: String = row.get(0).unwrap_or_default();
            let kind: String = row.get(1).unwrap_or_else(|_| "task".into());
            let title: String = row.get(2).unwrap_or_default();
            let done_int: i64 = row.get(3).unwrap_or(0);
            let due: Option<String> = row.get(4).ok();
            let priority_int: i64 = row.get(5).unwrap_or(2);
            let project: String = row.get(6).unwrap_or_default();
            list.push(ProjectRecord {
                project,
                path,
                kind,
                title,
                content: String::new(),
                done: done_int != 0,
                due,
                priority: priority_int.clamp(1, 3) as u8,
            });
        }
        Ok(list)
    }

    /// Reads a single note or task record including markdown content.
    pub async fn fetch_record(&self, path: &str) -> StoreResult<Option<ProjectRecord>> {
        let conn = self.connection().await?;
        let query = "SELECT path, kind, title, content, done, due, priority, project \
                     FROM projects_records \
                     WHERE path = ?1 LIMIT 1";
        let mut rows = conn
            .query(query, libsql::params![path])
            .await
            .map_err(|e| StoreError::Libsql(format!("query record failed: {e}")))?;

        if let Ok(Some(row)) = rows.next().await {
            let path: String = row.get(0).unwrap_or_default();
            let kind: String = row.get(1).unwrap_or_else(|_| "note".into());
            let title: String = row.get(2).unwrap_or_default();
            let content: String = row.get(3).unwrap_or_default();
            let done_int: i64 = row.get(4).unwrap_or(0);
            let due: Option<String> = row.get(5).ok();
            let priority_int: i64 = row.get(6).unwrap_or(2);
            let project: String = row.get(7).unwrap_or_default();
            Ok(Some(ProjectRecord {
                project,
                path,
                kind,
                title,
                content,
                done: done_int != 0,
                due,
                priority: priority_int.clamp(1, 3) as u8,
            }))
        } else {
            Ok(None)
        }
    }

    /// Saves or updates a note or task record locally and queues an offline mutation.
    pub async fn save_record(
        &self,
        project: &str,
        path: &str,
        kind: &str,
        title: &str,
        content: &str,
    ) -> StoreResult<()> {
        let conn = self.connection().await?;
        let sql = r#"
            INSERT INTO projects_records (path, project, kind, title, content, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(path) DO UPDATE SET
                title = excluded.title,
                content = excluded.content,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#;
        conn.execute(sql, libsql::params![path, project, kind, title, content])
            .await
            .map_err(|e| StoreError::Libsql(format!("save record failed: {e}")))?;

        let payload = serde_json::json!({
            "project": project,
            "path": path,
            "kind": kind,
            "title": title,
            "content": content,
        })
        .to_string();
        self.queue_mutation(DOMAIN_PROJECTS, path, "save_record", &payload)
            .await?;
        Ok(())
    }

    /// Toggles or updates task completion locally.
    pub async fn set_task_done(&self, path: &str, done: bool) -> StoreResult<()> {
        let conn = self.connection().await?;
        let done_val: i64 = if done { 1 } else { 0 };
        conn.execute(
            "UPDATE projects_records SET done = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE path = ?2",
            libsql::params![done_val, path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("update task done failed: {e}")))?;

        let payload = serde_json::json!({ "done": done }).to_string();
        self.queue_mutation(DOMAIN_PROJECTS, path, "set_task_done", &payload)
            .await?;
        Ok(())
    }

    /// Updates task priority locally and queues an offline mutation.
    pub async fn set_task_priority(&self, path: &str, priority: u8) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE projects_records SET priority = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE path = ?2",
            libsql::params![priority as i64, path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("update task priority failed: {e}")))?;

        let payload = serde_json::json!({ "priority": priority }).to_string();
        self.queue_mutation(DOMAIN_PROJECTS, path, "set_task_priority", &payload)
            .await?;
        Ok(())
    }

    /// Sets or clears (`""`) a task's due date locally and queues an offline mutation.
    pub async fn set_task_due(&self, path: &str, due: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE projects_records SET due = NULLIF(?1, ''), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE path = ?2",
            libsql::params![due, path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("update task due failed: {e}")))?;

        let payload = serde_json::json!({ "due": due }).to_string();
        self.queue_mutation(DOMAIN_PROJECTS, path, "set_task_due", &payload)
            .await?;
        Ok(())
    }

    /// Deletes a record locally and queues an offline mutation.
    pub async fn delete_record(&self, path: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "DELETE FROM projects_records WHERE path = ?1",
            libsql::params![path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("delete record failed: {e}")))?;

        let payload = serde_json::json!({ "path": path }).to_string();
        self.queue_mutation(DOMAIN_PROJECTS, path, "delete_record", &payload)
            .await?;
        Ok(())
    }

    /// Records an offline mutation to replay once connectivity to host is restored.
    async fn queue_mutation(
        &self,
        domain: &str,
        path: &str,
        action: &str,
        payload: &str,
    ) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO _offline_mutations (domain, path, action, payload) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![domain, path, action, payload],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("queue mutation failed: {e}")))?;
        Ok(())
    }

    /// Returns the number of uncommitted offline mutations waiting to sync.
    pub async fn pending_mutation_count(&self) -> StoreResult<usize> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query("SELECT COUNT(*) FROM _offline_mutations", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("count mutations failed: {e}")))?;
        if let Ok(Some(row)) = rows.next().await {
            let count: i64 = row.get(0).unwrap_or(0);
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }

    /// Every queued write, oldest first, in the order it was made.
    ///
    /// The order is the contract: two edits of the same note replay in the order the person typed
    /// them, so a sync never applies the older one last.
    pub async fn pending_mutations(&self) -> StoreResult<Vec<PendingMutation>> {
        self.pending_in(None).await
    }

    /// The queued writes of one domain, oldest first, or of every domain when `domain` is `None`.
    ///
    /// A sync pushes per domain: ordering only has to hold inside one, and a calendar write that
    /// the Mac refuses must not hold up a note.
    pub async fn pending_in(&self, domain: Option<&str>) -> StoreResult<Vec<PendingMutation>> {
        let conn = self.connection().await?;
        let read = match domain {
            Some(domain) => {
                conn.query(
                    "SELECT id, domain, path, action, payload FROM _offline_mutations \
                     WHERE domain = ?1 ORDER BY id ASC",
                    libsql::params![domain],
                )
                .await
            }
            None => {
                conn.query(
                    "SELECT id, domain, path, action, payload FROM _offline_mutations \
                     ORDER BY id ASC",
                    (),
                )
                .await
            }
        };
        let mut rows =
            read.map_err(|e| StoreError::Libsql(format!("read mutations failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(PendingMutation {
                id: row.get(0).unwrap_or(0),
                domain: row.get(1).unwrap_or_else(|_| DOMAIN_PROJECTS.into()),
                path: row.get(2).unwrap_or_default(),
                action: row.get(3).unwrap_or_default(),
                payload: row.get(4).unwrap_or_default(),
            });
        }
        Ok(list)
    }

    /// Forgets the mutations a sync has already replayed on the Mac.
    pub async fn clear_mutations(&self, ids: &[i64]) -> StoreResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.connection().await?;
        let mut cleared = 0;
        for id in ids {
            cleared += conn
                .execute(
                    "DELETE FROM _offline_mutations WHERE id = ?1",
                    libsql::params![*id],
                )
                .await
                .map_err(|e| StoreError::Libsql(format!("clear mutation failed: {e}")))?;
        }
        Ok(cleared as usize)
    }

    /// Remembers that the Mac refused the queued write for `path`, in the Mac's own words.
    ///
    /// Overwrites an older reason for the same record: a second refusal is what the Mac says now,
    /// and a list that showed both would ask the person to fix a contradiction.
    pub async fn record_refusal(&self, path: &str, reason: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO _sync_refusals (path, reason) VALUES (?1, ?2) \
             ON CONFLICT(path) DO UPDATE SET reason = ?2, at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![path, reason],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("record refusal failed: {e}")))?;
        Ok(())
    }

    /// Forgets a refusal: the record went through, or it is gone.
    pub async fn clear_refusal(&self, path: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "DELETE FROM _sync_refusals WHERE path = ?1",
            libsql::params![path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("clear refusal failed: {e}")))?;
        Ok(())
    }

    /// Every record the Mac refused, oldest first, as `path (reason)` pairs.
    pub async fn refusals(&self) -> StoreResult<Vec<(String, String)>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT path, reason FROM _sync_refusals ORDER BY at ASC",
                (),
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read refusals failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push((
                row.get::<String>(0).unwrap_or_default(),
                row.get::<String>(1).unwrap_or_default(),
            ));
        }
        Ok(list)
    }

    /// The records with a write still waiting for the Mac.
    ///
    /// A pull skips these: the phone's unsent edit is newer than anything the Mac can answer with,
    /// so writing the Mac's version over it would lose the edit.
    pub async fn dirty_paths(&self) -> StoreResult<Vec<String>> {
        self.dirty_keys(DOMAIN_PROJECTS).await
    }

    /// The keys of one domain with a write still waiting for the Mac.
    pub async fn dirty_keys(&self, domain: &str) -> StoreResult<Vec<String>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT DISTINCT path FROM _offline_mutations WHERE domain = ?1",
                libsql::params![domain],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read dirty keys failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(row.get::<String>(0).unwrap_or_default());
        }
        Ok(list)
    }

    /// What this row knows about the Mac's copy, or `None` when there is no such row.
    pub async fn sync_state(&self, path: &str) -> StoreResult<Option<SyncState>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT project, kind, title, content, done, due, priority, synced_content, remote_at \
                 FROM projects_records WHERE path = ?1 LIMIT 1",
                libsql::params![path],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read sync state failed: {e}")))?;
        let Some(row) = rows.next().await.ok().flatten() else {
            return Ok(None);
        };
        Ok(Some(SyncState {
            project: row.get(0).unwrap_or_default(),
            kind: row.get(1).unwrap_or_else(|_| "note".into()),
            title: row.get(2).unwrap_or_default(),
            content: row.get(3).unwrap_or_default(),
            done: row.get::<i64>(4).unwrap_or(0) != 0,
            due: row.get(5).ok(),
            priority: row.get::<i64>(6).unwrap_or(2).clamp(1, 3) as u8,
            synced_content: row.get(7).ok(),
            remote_at: row.get(8).ok(),
        }))
    }

    /// Records that the Mac now holds `content` for this path, so the next local edit pushes
    /// against it as its base.
    ///
    /// The Mac's stamp is left as it was: a pull re-reads this record once and takes the stamp
    /// with the content it confirms, which is one cheap read rather than a wrong `remote_at`.
    pub async fn mark_synced(&self, path: &str, content: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE projects_records SET synced_content = ?1 WHERE path = ?2",
            libsql::params![content, path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("mark synced failed: {e}")))?;
        Ok(())
    }

    /// Writes one record the Mac answered with, without queueing it back at the Mac.
    ///
    /// The Mac's content becomes this row's content and its sync base at once: the two sides agree
    /// from here, so the next local edit is the only difference there is.
    pub async fn upsert_pulled(
        &self,
        project: &str,
        record: &ProjectRecord,
        remote_at: &str,
    ) -> StoreResult<()> {
        let conn = self.connection().await?;
        let priority = i64::from(record.priority.clamp(1, 3));
        let done: i64 = if record.done { 1 } else { 0 };
        conn.execute(
            r#"
            INSERT INTO projects_records
                (path, project, kind, title, content, done, due, priority, synced_content, remote_at,
                 updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5, ?9,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(path) DO UPDATE SET
                project = excluded.project,
                kind = excluded.kind,
                title = excluded.title,
                content = excluded.content,
                done = excluded.done,
                due = excluded.due,
                priority = excluded.priority,
                synced_content = excluded.synced_content,
                remote_at = excluded.remote_at,
                updated_at = excluded.updated_at
            "#,
            libsql::params![
                record.path.clone(),
                project,
                record.kind.clone(),
                record.title.clone(),
                record.content.clone(),
                done,
                record.due.clone(),
                priority,
                remote_at.to_string()
            ],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("upsert pulled record failed: {e}")))?;
        Ok(())
    }

    /// Moves a row, and any write queued for it, to the path the Mac gave the record it created.
    ///
    /// A record made on the phone has a path the phone chose; the Mac names its own when it
    /// creates one (`POST /projects/api/notes`), so the local row follows it.
    pub async fn rekey(&self, from: &str, to: &str) -> StoreResult<()> {
        if from == to {
            return Ok(());
        }
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE projects_records SET path = ?1 WHERE path = ?2",
            libsql::params![to, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey record failed: {e}")))?;
        conn.execute(
            "UPDATE _offline_mutations SET path = ?1 WHERE path = ?2",
            libsql::params![to, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey mutations failed: {e}")))?;
        Ok(())
    }

    /// Drops one record, for a record the Mac no longer has.
    pub async fn remove_local(&self, path: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "DELETE FROM projects_records WHERE path = ?1",
            libsql::params![path],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("remove local record failed: {e}")))?;
        Ok(())
    }

    /// Every record this device has already reconciled with the Mac.
    ///
    /// A record that was never on the Mac is not in this list, so the sweep that follows a pull
    /// cannot delete something the Mac never had.
    pub async fn synced_paths(&self) -> StoreResult<Vec<String>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT path FROM projects_records WHERE synced_content IS NOT NULL",
                (),
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read synced paths failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(row.get::<String>(0).unwrap_or_default());
        }
        Ok(list)
    }

    // ------------------------------------------------------------ calendar

    /// The replica's entries touching `[from, to)`, all-day first then by start.
    ///
    /// The window is the calendar capability's own (`list_entries`): a timed entry ending at
    /// midnight does not reach into the next day, and an all-day entry ending that morning is in
    /// the day before.
    pub async fn calendar_between(&self, from: &str, to: &str) -> StoreResult<Vec<CalendarEntry>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, commitment, title, starts_at, ends_at, all_day, location, notes, \
                 source FROM calendar_entries WHERE starts_at < ?1 AND ends_at > ?2 \
                 ORDER BY all_day DESC, starts_at ASC",
                libsql::params![to.to_owned(), format!("{from}T00:00:00")],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("query calendar failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(CalendarEntry {
                id: row.get(0).unwrap_or_default(),
                kind: row.get(1).unwrap_or_else(|_| "event".into()),
                commitment: row.get(2).unwrap_or_else(|_| "possible".into()),
                title: row.get(3).unwrap_or_default(),
                starts_at: row.get(4).unwrap_or_default(),
                ends_at: row.get(5).unwrap_or_default(),
                all_day: row.get::<i64>(6).unwrap_or(1) != 0,
                location: row.get(7).ok(),
                notes: row.get(8).ok(),
                source: row.get(9).unwrap_or_else(|_| "manual".into()),
            });
        }
        Ok(list)
    }

    /// Writes one entry to the replica and queues it for the Mac.
    pub async fn save_calendar_entry(&self, entry: &CalendarEntry) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            r#"
            INSERT INTO calendar_entries
                (id, kind, commitment, title, starts_at, ends_at, all_day, location, notes, source,
                 updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                commitment = excluded.commitment,
                title = excluded.title,
                starts_at = excluded.starts_at,
                ends_at = excluded.ends_at,
                all_day = excluded.all_day,
                location = excluded.location,
                notes = excluded.notes,
                updated_at = excluded.updated_at
            "#,
            libsql::params![
                entry.id.clone(),
                entry.kind.clone(),
                entry.commitment.clone(),
                entry.title.clone(),
                entry.starts_at.clone(),
                entry.ends_at.clone(),
                if entry.all_day { 1i64 } else { 0i64 },
                entry.location.clone(),
                entry.notes.clone(),
                entry.source.clone(),
            ],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("save calendar entry failed: {e}")))?;
        let payload = serde_json::json!({
            "kind": entry.kind,
            "commitment": entry.commitment,
            "title": entry.title,
            "starts_at": entry.starts_at,
            "ends_at": entry.ends_at,
            "all_day": entry.all_day,
            "location": entry.location,
            "notes": entry.notes,
        })
        .to_string();
        self.queue_mutation(DOMAIN_CALENDAR, &entry.id, "calendar_save", &payload)
            .await?;
        Ok(())
    }

    /// Drops one entry from the replica and queues the deletion for the Mac.
    pub async fn delete_calendar_entry(&self, id: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "DELETE FROM calendar_entries WHERE id = ?1",
            libsql::params![id],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("delete calendar entry failed: {e}")))?;
        let payload = serde_json::json!({ "id": id }).to_string();
        self.queue_mutation(DOMAIN_CALENDAR, id, "calendar_delete", &payload)
            .await?;
        Ok(())
    }

    /// Writes one entry the Mac answered with, without queueing it back at the Mac.
    pub async fn upsert_pulled_calendar(
        &self,
        entry: &CalendarEntry,
        remote_at: &str,
    ) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            r#"
            INSERT INTO calendar_entries
                (id, kind, commitment, title, starts_at, ends_at, all_day, location, notes, source,
                 remote_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                commitment = excluded.commitment,
                title = excluded.title,
                starts_at = excluded.starts_at,
                ends_at = excluded.ends_at,
                all_day = excluded.all_day,
                location = excluded.location,
                notes = excluded.notes,
                source = excluded.source,
                remote_at = excluded.remote_at,
                updated_at = excluded.updated_at
            "#,
            libsql::params![
                entry.id.clone(),
                entry.kind.clone(),
                entry.commitment.clone(),
                entry.title.clone(),
                entry.starts_at.clone(),
                entry.ends_at.clone(),
                if entry.all_day { 1i64 } else { 0i64 },
                entry.location.clone(),
                entry.notes.clone(),
                entry.source.clone(),
                remote_at.to_owned(),
            ],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("upsert pulled entry failed: {e}")))?;
        Ok(())
    }

    /// Drops one entry the Mac no longer has.
    pub async fn remove_local_calendar(&self, id: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "DELETE FROM calendar_entries WHERE id = ?1",
            libsql::params![id],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("remove local entry failed: {e}")))?;
        Ok(())
    }

    /// One entry from the replica, for a caller that needs to compare before it writes.
    pub async fn calendar_entry(&self, id: &str) -> StoreResult<Option<CalendarEntry>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, commitment, title, starts_at, ends_at, all_day, location, notes, \
                 source FROM calendar_entries WHERE id = ?1 LIMIT 1",
                libsql::params![id],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read calendar entry failed: {e}")))?;
        let Ok(Some(row)) = rows.next().await else {
            return Ok(None);
        };
        Ok(Some(CalendarEntry {
            id: row.get(0).unwrap_or_default(),
            kind: row.get(1).unwrap_or_else(|_| "event".into()),
            commitment: row.get(2).unwrap_or_else(|_| "possible".into()),
            title: row.get(3).unwrap_or_default(),
            starts_at: row.get(4).unwrap_or_default(),
            ends_at: row.get(5).unwrap_or_default(),
            all_day: row.get::<i64>(6).unwrap_or(1) != 0,
            location: row.get(7).ok(),
            notes: row.get(8).ok(),
            source: row.get(9).unwrap_or_else(|_| "manual".into()),
        }))
    }

    /// Every entry id this replica holds, so a pull can sweep what the Mac deleted.
    pub async fn calendar_ids(&self) -> StoreResult<Vec<String>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query("SELECT id FROM calendar_entries", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("read entry ids failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(row.get::<String>(0).unwrap_or_default());
        }
        Ok(list)
    }

    /// True once this replica has read a change feed, which is how the Calendar page knows the
    /// replica can answer for calendar at all instead of the Mac's read-only copy.
    pub async fn has_calendar(&self) -> StoreResult<bool> {
        Ok(self.cursor(DOMAIN_CALENDAR).await?.is_some())
    }

    /// Where one domain's change feed was read to.
    pub async fn cursor(&self, domain: &str) -> StoreResult<Option<String>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT value FROM _replica_meta WHERE key = ?1",
                libsql::params![format!("{CURSOR_PREFIX}{domain}")],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read cursor failed: {e}")))?;
        Ok(rows
            .next()
            .await
            .ok()
            .flatten()
            .and_then(|row| row.get::<String>(0).ok()))
    }

    /// Records how far one domain's change feed has been read.
    pub async fn set_cursor(&self, domain: &str, cursor: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO _replica_meta (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            libsql::params![format!("{CURSOR_PREFIX}{domain}"), cursor],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("write cursor failed: {e}")))?;
        Ok(())
    }

    /// Moves an entry, and any write queued for it, to the id the Mac assigned on create.
    /// Writes one typed capture into the replica and queues it for the Mac.
    ///
    /// The row is here before the Mac hears anything, which is the point: a capture is what a person
    /// types on a train, and a queue that lost it would be worse than no field at all. Answers the
    /// row's id, which the page uses to draw it back.
    pub async fn save_capture(
        &self,
        text: Option<&str>,
        url: Option<&str>,
        tag: Option<&str>,
    ) -> StoreResult<String> {
        let id = format!(
            "{LOCAL_CAPTURE_ID_PREFIX}{}",
            self.next_capture_seq().await?
        );
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO captures (id, body, url, tag) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![id.clone(), text, url, tag],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("save capture failed: {e}")))?;
        drop(conn);
        let payload = serde_json::json!({ "text": text, "url": url, "tag": tag }).to_string();
        self.queue_mutation(DOMAIN_CAPTURES, &id, "capture_new", &payload)
            .await?;
        Ok(id)
    }

    /// A counter for the local ids, so two captures typed in the same second cannot collide.
    async fn next_capture_seq(&self) -> StoreResult<i64> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query("SELECT COUNT(*) FROM captures", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("count captures failed: {e}")))?;
        let count: i64 = match rows.next().await {
            Ok(Some(row)) => row.get(0).unwrap_or(0),
            _ => 0,
        };
        Ok(count + 1)
    }

    /// The captures this phone typed, newest first, with the Mac's id once it has one.
    pub async fn captures(&self) -> StoreResult<Vec<LocalCapture>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, body, url, tag, remote_id, created_at, status, routed_to, client \
                 FROM captures ORDER BY created_at DESC, rowid DESC",
                (),
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read captures failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(LocalCapture {
                id: row.get(0).unwrap_or_default(),
                text: row.get(1).ok(),
                url: row.get(2).ok(),
                tag: row.get(3).ok(),
                remote_id: row.get(4).ok(),
                created_at: row.get(5).unwrap_or_default(),
                status: row.get(6).ok(),
                routed_to: row.get(7).ok(),
                client: row.get(8).ok(),
            });
        }
        Ok(list)
    }

    /// Takes one capture the Mac answered with, matched by the Mac's id.
    ///
    /// A capture typed here and then landed is the same row, so the match is on `remote_id` and not
    /// on the primary key: inserting by id would draw the capture twice — once waiting, once on the
    /// Mac — and no reader could tell which was true. `INSERT OR REPLACE` on that match keeps one row.
    /// True when the row is new or changed, so the sync's count is what moved rather than how many
    /// captures the Mac happens to have.
    pub async fn upsert_pulled_capture(
        &self,
        capture: &crate::capture::Capture,
    ) -> StoreResult<bool> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT body, url, tag, status, routed_to, client FROM captures \
                 WHERE id = ?1 OR remote_id = ?1 LIMIT 1",
                libsql::params![capture.id.clone()],
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read pulled capture failed: {e}")))?;
        let same = match rows.next().await {
            Ok(Some(row)) => {
                row.get::<String>(0).ok().as_deref() == capture.text.as_deref()
                    && row.get::<String>(1).ok().as_deref() == capture.url.as_deref()
                    && row.get::<String>(2).ok().as_deref() == capture.tag.as_deref()
                    && row.get::<String>(3).ok().as_deref() == Some(capture.status.as_str())
                    // Where it was captured is provenance, not news: a Mac that echoes a different
                    // client back would otherwise make every sync look like it moved something.
                    && row.get::<String>(4).ok().as_deref() == capture.routed_to.as_deref()
            }
            _ => false,
        };
        if same {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO captures (id, body, url, tag, remote_id, status, routed_to, client, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?1, ?5, ?6, ?7, ?8) \
             ON CONFLICT(id) DO UPDATE SET body = ?2, url = ?3, tag = ?4, remote_id = ?1, \
                 status = ?5, routed_to = ?6, client = ?7",
            libsql::params![
                capture.id.clone(),
                capture.text.clone(),
                capture.url.clone(),
                capture.tag.clone(),
                capture.status.clone(),
                capture.routed_to.clone(),
                capture.client.clone(),
                capture.created_at.clone(),
            ],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("upsert pulled capture failed: {e}")))?;
        // A capture that was typed here under a local id: the row it became is the Mac's id, so the
        // local row goes rather than standing beside its own answer.
        conn.execute(
            "DELETE FROM captures WHERE remote_id = ?1 AND id <> ?1",
            libsql::params![capture.id.clone()],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("drop the local twin failed: {e}")))?;
        Ok(true)
    }

    /// Takes the Mac's whole answer for a capture it just created: the id it chose, and the tag,
    /// status and route it decided.
    ///
    /// The create's answer is the only place the Mac's triage of a phone capture is stated, so
    /// keeping it here is what stops the next pull seeing the same capture as news.
    pub async fn settle_capture(
        &self,
        local_id: &str,
        capture: &crate::capture::Capture,
    ) -> StoreResult<()> {
        self.rekey_capture(local_id, &capture.id).await?;
        // The row keeps the id it was typed under; `rekey_capture` has already put the Mac's id in
        // `remote_id`, which is what the list and the pull match on.
        let conn = self.connection().await?;
        conn.execute(
            // Matched on either id: a retry that already moved the queue to the Mac's id comes back
            // with that id, and the row is still the one that was typed.
            "UPDATE captures SET body = ?2, url = ?3, tag = ?4, status = ?5, routed_to = ?6, \
                 client = ?7 WHERE id = ?8 OR remote_id = ?8",
            libsql::params![
                capture.id.clone(),
                capture.text.clone(),
                capture.url.clone(),
                capture.tag.clone(),
                capture.status.clone(),
                capture.routed_to.clone(),
                capture.client.clone(),
                local_id.to_owned(),
            ],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("settle capture failed: {e}")))?;
        Ok(())
    }

    /// Records the Mac's id for a capture it took, and moves the queue with it: the local id was a
    /// placeholder, and a second write to it would create the capture twice.
    pub async fn rekey_capture(&self, from: &str, to: &str) -> StoreResult<()> {
        if from == to {
            return Ok(());
        }
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE captures SET remote_id = ?1 WHERE id = ?2",
            libsql::params![to, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey capture failed: {e}")))?;
        // The queue moves with it, the way a calendar entry's does: a second write to the local id
        // would create the capture on the Mac twice.
        conn.execute(
            "UPDATE _offline_mutations SET path = ?1 WHERE domain = ?2 AND path = ?3",
            libsql::params![to, DOMAIN_CAPTURES, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey capture mutations failed: {e}")))?;
        Ok(())
    }

    pub async fn rekey_calendar(&self, from: &str, to: &str) -> StoreResult<()> {
        if from == to {
            return Ok(());
        }
        let conn = self.connection().await?;
        conn.execute(
            "UPDATE calendar_entries SET id = ?1 WHERE id = ?2",
            libsql::params![to, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey entry failed: {e}")))?;
        conn.execute(
            "UPDATE _offline_mutations SET path = ?1 WHERE domain = ?2 AND path = ?3",
            libsql::params![to, DOMAIN_CALENDAR, from],
        )
        .await
        .map_err(|e| StoreError::Libsql(format!("rekey entry mutations failed: {e}")))?;
        Ok(())
    }
}

/// One write waiting in `_offline_mutations` for the Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMutation {
    pub id: i64,
    /// Which capability the write belongs to ([`DOMAIN_PROJECTS`], [`DOMAIN_CALENDAR`] or
    /// [`DOMAIN_CAPTURES`]).
    pub domain: String,
    /// The record's path, or for a capture the id it was given here.
    pub path: String,
    /// `save_record`, `set_task_done`, `calendar_save` or `capture_new`.
    pub action: String,
    /// The JSON the write carried, as it was queued.
    pub payload: String,
}

/// One capture this phone typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalCapture {
    /// The id it was written under here.
    pub id: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub tag: Option<String>,
    /// The Mac's id, once it took the capture.
    pub remote_id: Option<String>,
    pub created_at: String,
    /// The Mac's own state for it: `new`, `done` or `dismissed`. `None` until it lands, because a
    /// capture this phone has not sent has no state on the Mac.
    pub status: Option<String>,
    /// Where the Mac routed it: `feed:<id>`, `places:<id>` or `inventory:<id>`.
    pub routed_to: Option<String>,
    /// Who captured it, in the Mac's words: a phone that pulls the Mac's captures shows them too.
    pub client: Option<String>,
}

impl LocalCapture {
    /// True when the Mac has it, so the page can say which of the two it is looking at.
    pub fn on_the_mac(&self) -> bool {
        self.remote_id.is_some()
    }

    /// What the row says: the typed text, else the link's host, else the link.
    pub fn label(&self) -> String {
        self.text
            .clone()
            .filter(|text| !text.is_empty())
            .or_else(|| crate::capture::host(self.url.as_deref()))
            .or_else(|| self.url.clone())
            .unwrap_or_default()
    }
}

/// What one local record knows about its copy on the Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncState {
    pub project: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    /// The Mac's content this row was last reconciled with; `None` until the record is there.
    pub synced_content: Option<String>,
    /// The Mac's stamp for [`Self::synced_content`], compared against a pull's answer.
    pub remote_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two captures typed a moment apart get different ids and read back newest first, because a
    /// field that overwrites the first thought with the second is not an inbox.
    #[test]
    fn captures_are_queued_once_each_and_read_back_newest_first() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let first = store
                .save_capture(Some("call the plumber"), None, None)
                .await
                .expect("first");
            let second = store
                .save_capture(None, Some("https://example.org/x"), Some("look"))
                .await
                .expect("second");
            assert_ne!(first, second);

            let rows = store.captures().await.expect("captures");
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].id, second, "newest first");
            assert_eq!(rows[0].label(), "example.org", "a link shows its host");
            assert_eq!(rows[1].label(), "call the plumber");
            assert!(!rows[0].on_the_mac());
            assert_eq!(store.pending_mutation_count().await.unwrap(), 2);

            store.rekey_capture(&second, "mac-id").await.expect("rekey");
            let rows = store.captures().await.expect("captures");
            assert!(rows[0].on_the_mac(), "the Mac took this one");
            assert_eq!(rows[0].remote_id.as_deref(), Some("mac-id"));
            assert!(!rows[1].on_the_mac());
        });
    }

    #[test]
    fn in_memory_store_lifecycle() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("open in-memory");
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);

            store
                .save_record(
                    "Inbox",
                    "Inbox/Note1.md",
                    "note",
                    "First Offline Note",
                    "# Offline Header\n\nContent created on phone.",
                )
                .await
                .expect("save note");

            let record = store
                .fetch_record("Inbox/Note1.md")
                .await
                .expect("fetch note")
                .expect("record found");
            assert_eq!(record.title, "First Offline Note");
            assert_eq!(
                record.content,
                "# Offline Header\n\nContent created on phone."
            );
            assert_eq!(store.pending_mutation_count().await.unwrap(), 1);

            // Update task done
            store
                .save_record("Inbox", "Inbox/Task1.md", "task", "Buy milk", "")
                .await
                .expect("save task");
            store
                .set_task_done("Inbox/Task1.md", true)
                .await
                .expect("mark done");

            let task = store
                .fetch_record("Inbox/Task1.md")
                .await
                .expect("fetch task")
                .expect("task found");
            assert!(task.done);
            assert_eq!(store.pending_mutation_count().await.unwrap(), 3);
        });
    }

    /// The day window is the calendar capability's own rule (`list_entries`): an entry that ends at
    /// midnight belongs to the day it started in, and an id the phone minted queues in the calendar
    /// domain so a projects push neither waits on it nor sends it.
    #[test]
    fn a_calendar_day_follows_the_capabilitys_own_window() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("open in-memory");
            let entry =
                |id: &str, title: &str, starts: &str, ends: &str, all_day: bool| CalendarEntry {
                    id: id.to_string(),
                    kind: "event".into(),
                    commitment: "planned".into(),
                    title: title.to_string(),
                    starts_at: starts.to_string(),
                    ends_at: ends.to_string(),
                    all_day,
                    location: None,
                    notes: None,
                    source: "manual".into(),
                };

            store
                .save_calendar_entry(&entry(
                    "calendar:all-day",
                    "All day",
                    "2026-10-24",
                    "2026-10-25",
                    true,
                ))
                .await
                .expect("save all-day");
            store
                .save_calendar_entry(&entry(
                    "calendar:late",
                    "Late",
                    "2026-10-23T23:00:00",
                    "2026-10-24T00:00:00",
                    false,
                ))
                .await
                .expect("save late");
            store
                .save_calendar_entry(&entry(
                    "calendar:morning",
                    "Morning",
                    "2026-10-24T09:00:00",
                    "2026-10-24T10:00:00",
                    false,
                ))
                .await
                .expect("save morning");

            // All-day first, then by start. The entry ending at midnight is the day before.
            let day = store
                .calendar_between("2026-10-24", "2026-10-25")
                .await
                .expect("day");
            let titles: Vec<&str> = day.iter().map(|e| e.title.as_str()).collect();
            assert_eq!(titles, ["All day", "Morning"], "{day:?}");

            let previous = store
                .calendar_between("2026-10-23", "2026-10-24")
                .await
                .expect("previous day");
            let titles: Vec<&str> = previous.iter().map(|e| e.title.as_str()).collect();
            assert_eq!(titles, ["Late"], "{previous:?}");

            // Calendar writes queue in the calendar domain, not in the projects one.
            assert_eq!(
                store.pending_in(Some(DOMAIN_PROJECTS)).await.unwrap().len(),
                0
            );
            assert_eq!(
                store.pending_in(Some(DOMAIN_CALENDAR)).await.unwrap().len(),
                3
            );
            assert!(store.dirty_keys(DOMAIN_CALENDAR).await.unwrap().len() == 3);
            assert!(store.dirty_keys(DOMAIN_PROJECTS).await.unwrap().is_empty());
        });
    }
}

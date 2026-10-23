//! Turso / LibSQL local storage and offline replica manager.
//!
//! Provides direct on-device storage for both the desktop app and the standalone iOS client.
//! Reads and writes operate against local storage with microsecond latency, enabling full
//! offline viewing, editing, and task management when disconnected from the Mac host.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::shell::{ProjectRecord, ShellError};

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
const SCHEMA_MUTATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS _offline_mutations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    action TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

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
        self.add_missing_columns(&conn).await
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
            list.push(ProjectRecord {
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
        let query = "SELECT path, kind, title, done, due, priority \
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
            list.push(ProjectRecord {
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
        let query = "SELECT path, kind, title, content, done, due, priority \
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
            Ok(Some(ProjectRecord {
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
        self.queue_mutation(path, "save_record", &payload).await?;
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
        self.queue_mutation(path, "set_task_done", &payload).await?;
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
        self.queue_mutation(path, "set_task_priority", &payload)
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
        self.queue_mutation(path, "set_task_due", &payload).await?;
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
        self.queue_mutation(path, "delete_record", &payload).await?;
        Ok(())
    }

    /// Records an offline mutation to replay once connectivity to host is restored.
    async fn queue_mutation(&self, path: &str, action: &str, payload: &str) -> StoreResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO _offline_mutations (path, action, payload) VALUES (?1, ?2, ?3)",
            libsql::params![path, action, payload],
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
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, path, action, payload FROM _offline_mutations ORDER BY id ASC",
                (),
            )
            .await
            .map_err(|e| StoreError::Libsql(format!("read mutations failed: {e}")))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(PendingMutation {
                id: row.get(0).unwrap_or(0),
                path: row.get(1).unwrap_or_default(),
                action: row.get(2).unwrap_or_default(),
                payload: row.get(3).unwrap_or_default(),
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

    /// The records with a write still waiting for the Mac.
    ///
    /// A pull skips these: the phone's unsent edit is newer than anything the Mac can answer with,
    /// so writing the Mac's version over it would lose the edit.
    pub async fn dirty_paths(&self) -> StoreResult<Vec<String>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query("SELECT DISTINCT path FROM _offline_mutations", ())
            .await
            .map_err(|e| StoreError::Libsql(format!("read dirty paths failed: {e}")))?;
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
}

/// One write waiting in `_offline_mutations` for the Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMutation {
    pub id: i64,
    pub path: String,
    /// `save_record` or `set_task_done`, the two writes a phone makes.
    pub action: String,
    /// The JSON the write carried, as it was queued.
    pub payload: String,
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
}

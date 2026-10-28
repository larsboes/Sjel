//! The local-first read path: the Mac's own database when it is readable, the shell otherwise.
//!
//! `Repo` is the adapter the Projects page reads and writes through on both platforms. On the Mac
//! it reads this machine's database and writes through the shell. On the phone it holds the
//! replica ([`Repo::replica`]): every read answers from it, and every write lands there first and
//! is logged in `_offline_mutations`, which `crate::sync` replays at the Mac.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use crate::shell::{
    CalendarEntry, GlobalNote, GlobalTask, ProjectRecord, ProjectSummary, Shell, ShellError,
    ShellResult, SoundscapeState,
};

use crate::store::{StoreError, TursoStore, LOCAL_ENTRY_ID_PREFIX};

use super::{board::task_path, Notes};

/// The shell client, or the reason there is none, the local handle, and the notes read recently.
#[derive(Clone)]
pub struct Repo {
    shell: Result<Arc<Shell>, ShellError>,
    local: Arc<LocalDb>,
    notes: Notes,
    replica: Option<Replica>,
}

/// The phone's own store, and what to run after each write to it.
#[derive(Clone)]
struct Replica {
    store: Arc<TursoStore>,
    /// The phone refreshes its queued count and starts a sync here.
    wrote: Arc<dyn Fn() + Send + Sync>,
}

impl Replica {
    /// One record of the replica, with its content.
    async fn record(&self, path: &str) -> ShellResult<ProjectRecord> {
        self.store
            .fetch_record(path)
            .await
            .map_err(store_error)?
            .ok_or_else(|| ShellError::Store(format!("{path} is not on this device")))
    }

    /// A task as a task write answers it, read back after the write.
    async fn task(&self, path: &str) -> ShellResult<GlobalTask> {
        (self.wrote)();
        let record = self.record(path).await?;
        Ok(GlobalTask {
            project: project_of(&record.path).to_owned(),
            id: record.path,
            kind: record.kind,
            title: record.title,
            done: record.done,
            due: record.due,
            priority: record.priority,
            summary: None,
        })
    }
}

fn store_error(error: StoreError) -> ShellError {
    ShellError::Store(error.to_string())
}

/// The project a record path lives in: its first segment.
fn project_of(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// `<project>/Note-<seconds>.md`, the path a note made on this device has until the Mac names it.
fn note_path(project: &str, at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    format!("{}/Note-{secs}.md", project.trim_end_matches('/'))
}

/// `calendar:local:<nanos>`, the id an entry made on this device has until the Mac names it.
///
/// Nanoseconds because two entries made in the same second are ordinary on a phone, and a
/// duplicate id would make the second overwrite the first in the replica and in its queue.
fn local_entry_id(at: SystemTime) -> String {
    let nanos = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    format!("{LOCAL_ENTRY_ID_PREFIX}{nanos:x}")
}

/// One entry as `POST /calendar/api/entries` takes it (`NewEntry` in the calendar capability).
///
/// The Mac's own page builds this from a form's [`crate::calendar::Draft`]; a caller holding an
/// entry it already made, here or on the phone, sends the entry back.
fn entry_body(entry: &CalendarEntry) -> serde_json::Value {
    let mut body = serde_json::json!({
        "kind": if entry.kind.is_empty() { "event" } else { entry.kind.as_str() },
        "commitment": if entry.commitment.is_empty() {
            "possible"
        } else {
            entry.commitment.as_str()
        },
        "title": entry.title,
        "starts_at": entry.starts_at,
        "ends_at": entry.ends_at,
        "all_day": entry.all_day,
        "source": if entry.source.is_empty() {
            "manual"
        } else {
            entry.source.as_str()
        },
    });
    if let Some(location) = &entry.location {
        body["location"] = serde_json::json!(location);
    }
    if let Some(notes) = &entry.notes {
        body["notes"] = serde_json::json!(notes);
    }
    body
}

/// The one direct handle on the local database file, shared by every `Repo` clone.
///
/// The connection is what is worth keeping, not the `Database` it came from. Opening one costs
/// 0.05 ms, but a fresh connection starts on an empty SQLite page cache and pays to fault the rows
/// back in, so opening and reading once costs 0.68 ms where the same read costs 0.02 ms on a handle
/// that already ran one. A poll is a handful of queries every 10 s, so this is the difference
/// between paying that fault on every poll and paying it once per process. The numbers come from
/// `holding_one_connection_beats_opening_one_per_read` in this module's tests.
///
/// The cache holds a read view, never a transaction: every read here drains its own `Rows`, and a
/// WAL reader between queries sees what the capability processes committed after the handle was
/// opened. An open that fails is retried on the next read, because the file can appear after the
/// app starts.
///
/// The guard is a `tokio::sync::Mutex` because opening awaits, as in `sjel_turso::database_for`.
#[derive(Default)]
struct LocalDb {
    conn: Mutex<Option<libsql::Connection>>,
}

impl LocalDb {
    /// The shared connection to the configured database, opened on first use.
    async fn connection(&self) -> Option<libsql::Connection> {
        let path = Repo::local_db_path()?;
        self.connection_at(&path).await
    }

    /// The shared connection to `path`, opened on first use and reused afterwards.
    async fn connection_at(&self, path: &Path) -> Option<libsql::Connection> {
        let mut slot = self.conn.lock().await;
        if let Some(conn) = slot.as_ref() {
            return Some(conn.clone());
        }
        let conn = open_local(path).await?;
        *slot = Some(conn.clone());
        Some(conn)
    }
}

/// Open one read connection to `path`, with the PRAGMAs every read of this database expects.
async fn open_local(path: &Path) -> Option<libsql::Connection> {
    let db = libsql::Builder::new_local(path).build().await.ok()?;
    let conn = db.connect().ok()?;
    conn.execute_batch("PRAGMA busy_timeout = 5000;")
        .await
        .ok()?;
    Some(conn)
}

/// The database path this machine's environment names, whether or not a test may open it.
///
/// `Repo::local_db_path` adds the test gate; the probe in this module's tests asks this instead,
/// because measuring the held connection needs a database that is not in memory.
#[allow(dead_code)]
fn configured_db_path() -> Option<PathBuf> {
    let configured = std::env::var("SJEL_DB_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let path = if configured.is_some() || sjel_config::overlay_root().is_some() {
        sjel_config::database_path()
    } else {
        PathBuf::from(std::env::var("SJEL_OVERLAY_ROOT").ok()?).join("data/axon/axon.db")
    };
    path.is_file().then_some(path)
}

impl Repo {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self {
            shell,
            local: Arc::new(LocalDb::default()),
            notes: Notes::default(),
            replica: None,
        }
    }

    /// The phone's `Repo`: it reads and writes `store` and never asks a shell. `wrote` runs after
    /// every write, so the app can show the queued count and start a sync.
    pub fn replica(store: Arc<TursoStore>, wrote: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            shell: Err(ShellError::Store("this device has no shell".to_owned())),
            local: Arc::new(LocalDb::default()),
            notes: Notes::default(),
            replica: Some(Replica {
                store,
                wrote: Arc::new(wrote),
            }),
        }
    }

    /// True on the phone, where the replica is the record and the Mac is optional.
    pub fn is_replica(&self) -> bool {
        self.replica.is_some()
    }

    /// The connection reads go to: the replica's, otherwise this Mac's database file.
    async fn connection(&self) -> Option<libsql::Connection> {
        match &self.replica {
            Some(replica) => replica.store.connection().await.ok(),
            None => self.local.connection().await,
        }
    }

    /// The shell a read falls back to. The replica's answer is final, empty or not: a phone has
    /// no shell to ask instead.
    fn fallback(&self) -> ShellResult<Arc<Shell>> {
        match &self.replica {
            Some(_) => Err(ShellError::Store("the store did not answer".to_owned())),
            None => self.shell(),
        }
    }

    pub fn shell(&self) -> ShellResult<Arc<Shell>> {
        self.shell.clone()
    }

    /// The notes opened recently, so a note someone tabs back to opens without a read.
    pub fn notes(&self) -> &Notes {
        &self.notes
    }

    /// The endpoint the shell answers on, or why there is no shell. A `String` rather than the
    /// widget type the page draws, because this crate has no widgets.
    pub fn endpoint(&self) -> String {
        match &self.shell {
            Ok(shell) => shell.endpoint().to_owned(),
            Err(error) => error.to_string(),
        }
    }

    /// True if direct non-blocking LibSQL local access is active.
    pub fn has_direct_db(&self) -> bool {
        self.replica.is_some() || Self::local_db_path().is_some()
    }

    /// Return the resolved local database path from `SJEL_DB_PATH` or the overlay default.
    ///
    /// Tests get `None`, so no test reads this machine's database by accident; the probe that
    /// measures the held connection asks [`configured_db_path`] instead.
    ///
    /// `cfg(test)` only marks the crate that declares the test, so the app crate's window tests
    /// would otherwise read this machine's database through the shared `Repo`. It enables
    /// `test-support` from its dev-dependencies for exactly this line.
    pub fn local_db_path() -> Option<PathBuf> {
        #[cfg(any(test, feature = "test-support"))]
        {
            None
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            configured_db_path()
        }
    }

    /// Read projects directly from the local LibSQL database if accessible, otherwise the shell.
    pub async fn fetch_projects(&self) -> ShellResult<Vec<ProjectSummary>> {
        if let Some(conn) = self.connection().await {
            if let Ok(mut rows) = conn
                .query(
                    // The replica keeps no archived flag: the pull leaves archived projects out.
                    &format!(
                        "SELECT project, {} as archived, \
                         SUM(CASE WHEN kind = 'task' AND done = 0 THEN 1 ELSE 0 END) as open, \
                         SUM(CASE WHEN kind = 'task' AND done != 0 THEN 1 ELSE 0 END) as done, \
                         SUM(CASE WHEN kind = 'note' THEN 1 ELSE 0 END) as notes \
                         FROM projects_records GROUP BY project ORDER BY project",
                        if self.replica.is_some() {
                            "0"
                        } else {
                            "MAX(archived)"
                        }
                    ),
                    (),
                )
                .await
            {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    let name: String = row.get(0).unwrap_or_default();
                    let archived_int: i64 = row.get(1).unwrap_or(0);
                    let open: i64 = row.get(2).unwrap_or(0);
                    let done: i64 = row.get(3).unwrap_or(0);
                    let notes: i64 = row.get(4).unwrap_or(0);
                    list.push(ProjectSummary {
                        name,
                        archived: archived_int != 0,
                        open: open.max(0) as usize,
                        done: done.max(0) as usize,
                        notes: notes.max(0) as usize,
                    });
                }
                if !list.is_empty() || self.replica.is_some() {
                    return Ok(list);
                }
            }
        }
        let shell = self.fallback()?;
        shell.projects().await
    }

    /// Read tasks directly from the local LibSQL database if accessible, otherwise the shell.
    pub async fn fetch_tasks(&self, status: Option<&str>) -> ShellResult<Vec<GlobalTask>> {
        if let Some(conn) = self.connection().await {
            let filter = match status {
                Some("open") => " AND done = 0",
                Some("done") => " AND done != 0",
                _ => "",
            };
            let query = format!(
                "SELECT path, kind, title, done, due, priority, project \
                 FROM projects_records WHERE kind = 'task'{filter} \
                 ORDER BY done ASC, priority DESC, due ASC"
            );
            if let Ok(mut rows) = conn.query(&query, ()).await {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    let id: String = row.get(0).unwrap_or_default();
                    let kind: String = row.get(1).unwrap_or_else(|_| "task".into());
                    let title: String = row.get(2).unwrap_or_default();
                    let done_int: i64 = row.get(3).unwrap_or(0);
                    let due: Option<String> = row.get(4).ok();
                    let priority_int: i64 = row.get(5).unwrap_or(2);
                    let project: String = row.get(6).unwrap_or_default();
                    list.push(GlobalTask {
                        id,
                        kind,
                        title,
                        done: done_int != 0,
                        due,
                        priority: priority_int.clamp(1, 3) as u8,
                        summary: None,
                        project,
                    });
                }
                if !list.is_empty() || self.replica.is_some() {
                    return Ok(list);
                }
            }
        }
        let shell = self.fallback()?;
        shell.tasks(status).await
    }

    /// Read notes directly from the local LibSQL database if accessible, otherwise the shell.
    pub async fn fetch_all_notes(&self) -> ShellResult<Vec<GlobalNote>> {
        if let Some(conn) = self.connection().await {
            let query = "SELECT path, kind, title, project \
                         FROM projects_records WHERE kind != 'task' \
                         ORDER BY project ASC, path ASC";
            if let Ok(mut rows) = conn.query(query, ()).await {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    let path: String = row.get(0).unwrap_or_default();
                    let kind: String = row.get(1).unwrap_or_else(|_| "note".into());
                    let title: String = row.get(2).unwrap_or_default();
                    let project: String = row.get(3).unwrap_or_default();
                    list.push(GlobalNote {
                        path,
                        kind,
                        title,
                        project,
                    });
                }
                if !list.is_empty() || self.replica.is_some() {
                    return Ok(list);
                }
            }
        }
        let shell = self.fallback()?;
        shell.notes().await
    }

    /// Read calendar entries from wherever this device keeps them.
    ///
    /// On the phone that is the replica, read through [`TursoStore::calendar_between`], so the day
    /// window has one owner and the id, kind, place and notes come with every row: a page that can
    /// change an entry needs to name it. On the Mac the local database is read with the same window
    /// and the same columns. Either way the shell answers when the database does not.
    pub async fn fetch_calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        if let Some(replica) = &self.replica {
            return replica
                .store
                .calendar_between(from, to)
                .await
                .map_err(store_error);
        }
        if let Some(conn) = self.connection().await {
            // `[from, to)`, with the bare `to` date so all-day and timed entries end alike, and
            // all-day first, which is the order the list reads in.
            let from_bound = format!("{from}T00:00:00");
            let query =
                "SELECT id, kind, commitment, title, starts_at, ends_at, all_day, location, \
                         notes, source FROM calendar_entries \
                         WHERE starts_at < ?1 AND ends_at > ?2 \
                         ORDER BY all_day DESC, starts_at ASC";
            if let Ok(mut rows) = conn
                .query(query, libsql::params![to.to_owned(), from_bound])
                .await
            {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    list.push(CalendarEntry {
                        id: row.get(0).unwrap_or_default(),
                        kind: row.get(1).unwrap_or_else(|_| "event".into()),
                        commitment: row.get(2).unwrap_or_else(|_| "possible".into()),
                        title: row.get(3).unwrap_or_default(),
                        starts_at: row.get(4).unwrap_or_default(),
                        ends_at: row.get(5).unwrap_or_default(),
                        all_day: row.get::<i64>(6).unwrap_or(0) != 0,
                        location: row.get(7).ok(),
                        notes: row.get(8).ok(),
                        source: row.get(9).unwrap_or_else(|_| "manual".into()),
                    });
                }
                return Ok(list);
            }
        }
        let shell = self.fallback()?;
        shell.calendar(from, to).await
    }

    /// True once the calendar can be answered from here: the replica, once its change feed has
    /// been read ([`TursoStore::has_calendar`]), or this Mac's own database, which is always there.
    ///
    /// A page reads the Mac's read-only copy until this answers true, because a replica that has
    /// never pulled holds no entries and would draw an empty month beside a readable one.
    pub async fn has_calendar(&self) -> bool {
        match &self.replica {
            Some(replica) => replica.store.has_calendar().await.unwrap_or(false),
            None => self.has_direct_db(),
        }
    }

    /// Writes one entry where this device writes.
    ///
    /// On the phone that is the replica, and the write is queued for the Mac like any other. An
    /// entry with no id is one this device just made, and gets a local id
    /// ([`LOCAL_ENTRY_ID_PREFIX`]) that the sync replaces with the Mac's, moving the row and the
    /// queued write together (`crate::sync`). On the Mac it is the shell, and the answer is the
    /// entry the capability made.
    pub async fn save_calendar_entry(&self, entry: CalendarEntry) -> ShellResult<CalendarEntry> {
        let mut entry = entry;
        // What a blank field means is decided here rather than per store, so the phone's replica and
        // the Mac's shell cannot disagree about the entry that was sent.
        if entry.kind.is_empty() {
            entry.kind = "event".to_owned();
        }
        if entry.commitment.is_empty() {
            entry.commitment = "possible".to_owned();
        }
        if entry.source.is_empty() {
            entry.source = "manual".to_owned();
        }
        match &self.replica {
            Some(replica) => {
                if entry.id.is_empty() {
                    entry.id = local_entry_id(SystemTime::now());
                }
                replica
                    .store
                    .save_calendar_entry(&entry)
                    .await
                    .map_err(store_error)?;
                (replica.wrote)();
                Ok(entry)
            }
            None => {
                self.shell()?
                    .create_calendar_entry(entry_body(&entry))
                    .await
            }
        }
    }

    /// Drops one entry here and queues the deletion for the Mac.
    pub async fn delete_calendar_entry(&self, id: &str) -> ShellResult<()> {
        match &self.replica {
            Some(replica) => {
                replica
                    .store
                    .delete_calendar_entry(id)
                    .await
                    .map_err(store_error)?;
                (replica.wrote)();
                Ok(())
            }
            None => self.shell()?.delete_calendar_entry(id).await,
        }
    }

    /// Read project records directly from the local LibSQL database if accessible, otherwise the
    /// shell.
    pub async fn fetch_project_records(&self, project: &str) -> ShellResult<Vec<ProjectRecord>> {
        if let Some(conn) = self.connection().await {
            let query = "SELECT path, kind, title, done, due, priority \
                         FROM projects_records \
                         WHERE project = ?1 \
                         ORDER BY done ASC, priority DESC, path ASC";
            if let Ok(mut rows) = conn.query(query, libsql::params![project]).await {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    let path: String = row.get(0).unwrap_or_default();
                    let kind: String = row.get(1).unwrap_or_else(|_| "task".into());
                    let title: String = row.get(2).unwrap_or_default();
                    let done_int: i64 = row.get(3).unwrap_or(0);
                    let due: Option<String> = row.get(4).ok();
                    let priority_int: i64 = row.get(5).unwrap_or(2);
                    list.push(ProjectRecord {
                        project: String::new(),
                        path,
                        kind,
                        title,
                        content: String::new(),
                        done: done_int != 0,
                        due,
                        priority: priority_int.clamp(1, 3) as u8,
                    });
                }
                if !list.is_empty() || self.replica.is_some() {
                    return Ok(list);
                }
            }
        }
        let shell = self.fallback()?;
        shell.project(project).await
    }

    /// Read a single record with its content: from memory when it was opened recently, otherwise
    /// from the local LibSQL database, otherwise from the shell. Every answer is kept, so opening
    /// the same note again costs no read, no disk and no network.
    pub async fn fetch_record(&self, path: &str) -> ShellResult<ProjectRecord> {
        if let Some(record) = self.notes.get(path) {
            return Ok(record);
        }
        let record = self.read_record(path).await?;
        self.notes.put(&record);
        Ok(record)
    }

    /// The slow half of [`Repo::fetch_record`]: one record with its content, from the local LibSQL
    /// database otherwise from the shell.
    async fn read_record(&self, path: &str) -> ShellResult<ProjectRecord> {
        if let Some(conn) = self.connection().await {
            let query = "SELECT path, kind, title, content, done, due, priority \
                         FROM projects_records \
                         WHERE path = ?1 LIMIT 1";
            if let Ok(mut rows) = conn.query(query, libsql::params![path]).await {
                if let Ok(Some(row)) = rows.next().await {
                    let path: String = row.get(0).unwrap_or_default();
                    let kind: String = row.get(1).unwrap_or_else(|_| "note".into());
                    let title: String = row.get(2).unwrap_or_default();
                    let content: String = row.get(3).unwrap_or_default();
                    let done_int: i64 = row.get(4).unwrap_or(0);
                    let due: Option<String> = row.get(5).ok();
                    let priority_int: i64 = row.get(6).unwrap_or(2);
                    return Ok(ProjectRecord {
                        project: String::new(),
                        path,
                        kind,
                        title,
                        content,
                        done: done_int != 0,
                        due,
                        priority: priority_int.clamp(1, 3) as u8,
                    });
                }
            }
        }
        let shell = self.fallback()?;
        shell.record(path).await
    }

    /// Writes a note's body. The Mac sends the base, so the capability refuses (409) an edit made
    /// over a newer version. The phone writes its replica; the sync settles a conflict there.
    /// Either way the kept copy becomes what was written, so opening the note again shows it.
    pub async fn save_record(&self, path: &str, content: &str, base: &str) -> ShellResult<()> {
        match &self.replica {
            None => self.shell()?.save_record(path, content, base).await?,
            Some(replica) => {
                let record = replica.record(path).await?;
                replica
                    .store
                    .save_record(project_of(path), path, &record.kind, &record.title, content)
                    .await
                    .map_err(store_error)?;
                (replica.wrote)();
            }
        }
        self.notes.set_content(path, content);
        Ok(())
    }

    /// Ticks or reopens a task, answering with its new frontmatter.
    pub async fn set_task_done(&self, path: &str, done: bool) -> ShellResult<GlobalTask> {
        let Some(replica) = &self.replica else {
            return self.shell()?.set_task_done(path, done).await;
        };
        replica
            .store
            .set_task_done(path, done)
            .await
            .map_err(store_error)?;
        replica.task(path).await
    }

    /// Writes one frontmatter field of a task (`due` or `priority`), answering with its new
    /// frontmatter.
    pub async fn set_property(
        &self,
        path: &str,
        key: &str,
        value: &str,
    ) -> ShellResult<GlobalTask> {
        let Some(replica) = &self.replica else {
            return self.shell()?.set_property(path, key, value).await;
        };
        let written = match key {
            "due" => replica.store.set_task_due(path, value).await,
            "priority" => {
                let priority = value
                    .parse::<u8>()
                    .map_err(|_| ShellError::Store(format!("{value} is not a priority")))?;
                replica.store.set_task_priority(path, priority).await
            }
            other => return Err(ShellError::Store(format!("this device cannot set {other}"))),
        };
        written.map_err(store_error)?;
        replica.task(path).await
    }

    /// Adds a task to a project.
    pub async fn add_task(&self, project: &str, title: &str) -> ShellResult<()> {
        let Some(replica) = &self.replica else {
            return self.shell()?.add_task(project, title).await;
        };
        let path = task_path(project, SystemTime::now());
        replica
            .store
            .save_record(project, &path, "task", title, "")
            .await
            .map_err(store_error)?;
        (replica.wrote)();
        Ok(())
    }

    /// Adds a note to a project.
    pub async fn add_note(&self, project: &str, title: &str) -> ShellResult<()> {
        let Some(replica) = &self.replica else {
            return self.shell()?.add_note(project, title).await;
        };
        let path = note_path(project, SystemTime::now());
        replica
            .store
            .save_record(project, &path, "note", title, &format!("# {title}\n"))
            .await
            .map_err(store_error)?;
        (replica.wrote)();
        Ok(())
    }

    /// Adds a project. On the phone that is its hub note, which the sync creates the project for.
    pub async fn add_project(&self, name: &str, summary: &str) -> ShellResult<()> {
        let Some(replica) = &self.replica else {
            return self.shell()?.add_project(name, summary).await;
        };
        let hub = format!("{name}/{name}.md");
        replica
            .store
            // Empty, so the first sync keeps the hub the Mac's template writes (`sync::create`).
            .save_record(name, &hub, "project", name, "")
            .await
            .map_err(store_error)?;
        (replica.wrote)();
        Ok(())
    }

    /// Removes a record.
    pub async fn remove_record(&self, path: &str) -> ShellResult<()> {
        self.notes.forget(path);
        let Some(replica) = &self.replica else {
            return self.shell()?.remove_record(path).await;
        };
        replica
            .store
            .delete_record(path)
            .await
            .map_err(store_error)?;
        (replica.wrote)();
        Ok(())
    }

    /// Read soundscape state (`GET /soundscape/api/soundscape/state`).
    pub async fn soundscape_state(&self) -> ShellResult<SoundscapeState> {
        let shell = self.fallback()?;
        shell.soundscape_state().await
    }

    /// Update soundscape playback (`POST /soundscape/api/soundscape/state`).
    #[allow(dead_code)]
    pub async fn set_soundscape_playing(&self, playing: bool) -> ShellResult<SoundscapeState> {
        let shell = self.fallback()?;
        shell.set_soundscape_playing(playing).await
    }

    /// Update soundscape preset (`POST /soundscape/api/soundscape/state`).
    #[allow(dead_code)]
    pub async fn set_soundscape_preset(&self, preset: &str) -> ShellResult<SoundscapeState> {
        let shell = self.fallback()?;
        shell.set_soundscape_preset(preset).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DOMAIN_CALENDAR;

    /// A fresh store and the phone's repo over it, for the calendar tests below.
    async fn replica() -> (Arc<TursoStore>, Repo) {
        let store = Arc::new(TursoStore::in_memory().await.expect("store"));
        let repo = Repo::replica(store.clone(), || {});
        (store, repo)
    }

    fn an_entry(title: &str, starts_at: &str, ends_at: &str) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts_at.to_owned(),
            ends_at: ends_at.to_owned(),
            all_day: false,
            commitment: "planned".to_owned(),
            kind: "event".to_owned(),
            ..CalendarEntry::default()
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    /// An entry made on the phone gets an id this device can name, and the page needs that id back:
    /// it is what a later edit and a delete are addressed to.
    #[test]
    fn an_entry_made_here_is_stored_with_a_local_id_and_queued_for_the_mac() {
        runtime().block_on(async {
            let (store, repo) = replica().await;
            let saved = repo
                .save_calendar_entry(an_entry(
                    "Dentist",
                    "2026-10-24T09:00:00",
                    "2026-10-24T10:00:00",
                ))
                .await
                .expect("save");
            assert!(
                saved.id.starts_with(LOCAL_ENTRY_ID_PREFIX),
                "a local id until the Mac names one: {saved:?}"
            );

            let day = repo
                .fetch_calendar("2026-10-24", "2026-10-25")
                .await
                .expect("day");
            assert_eq!(day.len(), 1, "{day:?}");
            assert_eq!(day[0].id, saved.id, "the id survives the read");
            assert_eq!(day[0].title, "Dentist");
            assert_eq!(day[0].kind, "event");
            assert_eq!(day[0].source, "manual");
            assert_eq!(day[0].commitment, "planned");
            assert_eq!(
                store.pending_in(Some(DOMAIN_CALENDAR)).await.unwrap().len(),
                1,
                "the write is queued for the next sync"
            );
        });
    }

    /// Two entries made in the same second are ordinary on a phone, and a shared id would make the
    /// second overwrite the first in the replica and in its queue.
    #[test]
    fn two_entries_made_here_carry_different_ids() {
        runtime().block_on(async {
            let (_, repo) = replica().await;
            let one = repo
                .save_calendar_entry(an_entry("One", "2026-10-24", "2026-10-25"))
                .await
                .expect("save one");
            let two = repo
                .save_calendar_entry(an_entry("Two", "2026-10-24", "2026-10-25"))
                .await
                .expect("save two");
            assert_ne!(one.id, two.id);
            let day = repo
                .fetch_calendar("2026-10-24", "2026-10-25")
                .await
                .expect("day");
            assert_eq!(day.len(), 2, "{day:?}");
        });
    }

    #[test]
    fn a_deleted_entry_leaves_the_day_and_queues_the_deletion() {
        runtime().block_on(async {
            let (store, repo) = replica().await;
            let saved = repo
                .save_calendar_entry(an_entry("Dentist", "2026-10-24", "2026-10-25"))
                .await
                .expect("save");
            repo.delete_calendar_entry(&saved.id).await.expect("delete");

            assert!(repo
                .fetch_calendar("2026-10-24", "2026-10-25")
                .await
                .expect("day")
                .is_empty());
            // The create replays first and takes the Mac's id, and the rekey moves the deletion
            // with it, so the entry the Mac made is the one the deletion names.
            assert_eq!(
                store.pending_in(Some(DOMAIN_CALENDAR)).await.unwrap().len(),
                2
            );
        });
    }

    /// A replica that has never pulled holds no entries, and the page must keep drawing the Mac's
    /// copy until this answers true.
    #[test]
    fn the_calendar_is_read_off_the_replica_once_its_feed_has_been_read() {
        runtime().block_on(async {
            let (store, repo) = replica().await;
            assert!(!repo.has_calendar().await);
            store
                .set_cursor(DOMAIN_CALENDAR, "1761000000000")
                .await
                .expect("cursor");
            assert!(repo.has_calendar().await);
        });
    }

    /// A device with no store and no shell has nowhere to write an entry, and says so rather than
    /// losing it.
    #[test]
    fn an_entry_with_no_store_to_write_to_is_refused() {
        runtime().block_on(async {
            let repo = Repo::new(Err(ShellError::NotLoopback));
            assert!(!repo.has_calendar().await);
            assert!(repo
                .save_calendar_entry(an_entry("Dentist", "2026-10-24", "2026-10-25"))
                .await
                .is_err());
            assert!(repo.delete_calendar_entry("calendar:1").await.is_err());
        });
    }

    fn note(path: &str) -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: path.into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: "# Plan\n".into(),
            done: false,
            due: None,
            priority: 2,
        }
    }

    #[test]
    fn a_kept_note_opens_with_no_database_and_no_shell() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                // Neither a database path (there is none under test) nor a shell to ask: the only
                // answer this repo can give is one it already holds.
                let repo = Repo::new(Err(ShellError::NotLoopback));
                let kept = note("P/Plan.md");
                repo.notes().put(&kept);

                let read = repo.fetch_record("P/Plan.md").await.expect("from memory");
                assert_eq!(read.content, kept.content);
                assert_eq!(read.title, kept.title);

                // The same read for a note that is not kept fails, which is what makes the line
                // above proof that nothing was read.
                assert!(repo.fetch_record("P/Other.md").await.is_err());
            });
    }

    #[test]
    fn test_direct_libsql_queries() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
            let conn = db.connect().unwrap();

            conn.execute_batch(
                "
                CREATE TABLE projects_records (
                    path TEXT PRIMARY KEY,
                    project TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    done INTEGER NOT NULL,
                    due TEXT,
                    priority INTEGER NOT NULL,
                    archived INTEGER NOT NULL,
                    content TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE calendar_entries (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    starts_at TEXT NOT NULL,
                    ends_at TEXT NOT NULL,
                    all_day INTEGER NOT NULL,
                    commitment TEXT NOT NULL
                );
                INSERT INTO projects_records VALUES (
                    'p1/note.md', 'p1', 'note', 'Note Title', 0, NULL, 2, 0, '# Note Body', '2026-10-21'
                );
                INSERT INTO projects_records VALUES (
                    'p1/task.md', 'p1', 'task', 'Task Title', 1, '2026-10-22', 3, 0, '', '2026-10-21'
                );
                INSERT INTO calendar_entries VALUES (
                    'cal:1', 'Team Sync', '2026-10-21T10:00:00', '2026-10-21T11:00:00', 0, 'committed'
                );
                ",
            )
            .await
            .unwrap();

            // Query projects
            let mut rows = conn
                .query(
                    "SELECT project, MAX(archived) as archived, \
                     SUM(CASE WHEN kind = 'task' AND done = 0 THEN 1 ELSE 0 END) as open, \
                     SUM(CASE WHEN kind = 'task' AND done != 0 THEN 1 ELSE 0 END) as done, \
                     SUM(CASE WHEN kind = 'note' THEN 1 ELSE 0 END) as notes \
                     FROM projects_records GROUP BY project ORDER BY project",
                    (),
                )
                .await
                .unwrap();
            let row = rows.next().await.unwrap().unwrap();
            let name: String = row.get(0).unwrap();
            assert_eq!(name, "p1");
            let open: i64 = row.get(2).unwrap();
            let done: i64 = row.get(3).unwrap();
            let notes: i64 = row.get(4).unwrap();
            assert_eq!(open, 0);
            assert_eq!(done, 1);
            assert_eq!(notes, 1);

            // Query calendar
            let from_bound = "2026-10-21T00:00:00";
            let to_bound = "2026-10-21T23:59:59";
            let mut cal_rows = conn
                .query(
                    "SELECT title, starts_at, ends_at, all_day, commitment \
                     FROM calendar_entries \
                     WHERE starts_at <= ?1 AND ends_at >= ?2 \
                     ORDER BY starts_at ASC",
                    libsql::params![to_bound, from_bound],
                )
                .await
                .unwrap();
            let cal_row = cal_rows.next().await.unwrap().unwrap();
            let title: String = cal_row.get(0).unwrap();
            assert_eq!(title, "Team Sync");
        });
    }

    /// A cached handle must see writes committed after it opened, because the file it reads is
    /// written by the capability processes while the app is running.
    #[test]
    fn a_reused_connection_sees_writes_committed_after_it_opened() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let dir = std::env::temp_dir().join(format!(
                    "sjel-gpui-reused-connection-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let path = dir.join("axon.db");

                let writer = open_local(&path).await.unwrap();
                writer
                    .execute_batch(
                        "CREATE TABLE IF NOT EXISTS t (x INTEGER); INSERT INTO t VALUES (1);",
                    )
                    .await
                    .unwrap();

                let local = LocalDb::default();
                assert_eq!(count(&local.connection_at(&path).await.unwrap()).await, 1);

                writer
                    .execute("INSERT INTO t VALUES (2)", ())
                    .await
                    .unwrap();

                assert_eq!(count(&local.connection_at(&path).await.unwrap()).await, 2);

                std::fs::remove_dir_all(&dir).unwrap();
            });
    }

    async fn count(conn: &libsql::Connection) -> i64 {
        let mut rows = conn.query("SELECT COUNT(*) FROM t", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        row.get(0).unwrap()
    }

    /// What holding one connection buys, measured against a database this machine already has.
    ///
    /// Ignored by default: it reads a file outside the checkout and prints timings instead of
    /// asserting one, which a shared machine cannot promise. Run it with
    ///
    ///     cargo test --release --manifest-path apps/gpui/core/Cargo.toml \
    ///         holding_one_connection -- --ignored --nocapture
    ///
    /// The release profile is the one the quotes in this module and in the README come from; the
    /// test profile prints the same ratio at larger milliseconds. `SJEL_DB_PATH` or an overlay's
    /// `data/axon/axon.db` has to exist; without one it says so and passes. Every path reads the
    /// same rows, so a change that made one of them wrong fails here.
    #[test]
    #[ignore = "needs this machine's database; prints timings"]
    fn holding_one_connection_beats_opening_one_per_read() {
        let Some(path) = configured_db_path() else {
            println!("no local database: set SJEL_DB_PATH or SJEL_PERSONAL_ROOT");
            return;
        };
        let query = "SELECT path, kind, title, done, due, priority, project \
                     FROM projects_records WHERE kind = 'task'";

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            // One open per read, which is what every fetch did before the handle was kept, and the
            // open on its own, which is the part that was never the cost.
            let mut per_read = Vec::new();
            let mut open_only = Vec::new();
            let mut per_read_rows = 0;
            for _ in 0..10 {
                let started = std::time::Instant::now();
                let conn = open_local(&path).await.expect("the database opens");
                open_only.push(started.elapsed());
                per_read_rows = drain(&conn, query).await;
                per_read.push(started.elapsed());
            }

            // One open, reused, which is what a poll does now.
            let local = LocalDb::default();
            let mut held = Vec::new();
            let mut held_rows = 0;
            for _ in 0..10 {
                let started = std::time::Instant::now();
                let conn = local
                    .connection_at(&path)
                    .await
                    .expect("the database opens");
                held_rows = drain(&conn, query).await;
                held.push(started.elapsed());
            }

            assert_eq!(per_read_rows, held_rows, "both paths read the same rows");
            let mib =
                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / (1024.0 * 1024.0);
            println!("{} ({mib:.0} MiB): {held_rows} task rows", path.display());
            println!("open, no read:             {:?} median", median(open_only));
            println!("open + read, one per read: {:?} median", median(per_read));
            println!("held connection, read:     {:?} median", median(held));
        });
    }

    /// How many rows `sql` returns on `conn`.
    async fn drain(conn: &libsql::Connection, sql: &str) -> usize {
        let mut rows = conn.query(sql, ()).await.expect("query runs");
        let mut read = 0;
        while rows.next().await.expect("row").is_some() {
            read += 1;
        }
        read
    }

    /// The middle timing, which one scheduler hiccup cannot move.
    fn median(mut timings: Vec<std::time::Duration>) -> std::time::Duration {
        timings.sort_unstable();
        timings[timings.len() / 2]
    }
}

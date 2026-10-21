//! Every read and write against the shell, in one place.
//!
//! A read keeps its last good value while it retries, never runs twice at once for the same
//! slot, and settles into the slot it came from. A write toasts its outcome and then lets the
//! caller re-read only what it changed, so a task edit does not pull the project again.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::{Context, SharedString, Window};

use sjel_gpui_core::shell::{self, Shell, ShellError, ShellResult};

/// Data older than this shows its age beside the page title.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// The shell client, or the reason there is none.
#[derive(Clone)]
pub struct Repo {
    shell: Result<Arc<Shell>, ShellError>,
}

impl Repo {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self { shell }
    }

    pub fn shell(&self) -> ShellResult<Arc<Shell>> {
        self.shell.clone()
    }

    pub fn endpoint(&self) -> SharedString {
        match &self.shell {
            Ok(shell) => shell.endpoint().to_owned().into(),
            Err(error) => error.to_string().into(),
        }
    }

    /// Return the resolved local database path from `SJEL_DB_PATH` or the overlay default.
    #[allow(dead_code)]
    pub fn local_db_path() -> Option<std::path::PathBuf> {
        #[cfg(test)]
        {
            return None;
        }
        #[allow(unreachable_code)]
        if let Ok(p) = std::env::var("SJEL_DB_PATH") {
            let path = std::path::PathBuf::from(p);
            if path.exists() {
                return Some(path);
            }
        }
        if let Ok(overlay) = std::env::var("SJEL_OVERLAY_ROOT") {
            let candidate = std::path::PathBuf::from(overlay).join("data/axon/axon.db");
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Ok(overlay) = std::env::var("SJEL_PERSONAL_ROOT") {
            let candidate = std::path::PathBuf::from(overlay).join("data/axon/axon.db");
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    /// Open a direct non-blocking read connection to the local LibSQL database for zero-latency queries.
    #[allow(dead_code)]
    pub async fn connect_local_db() -> Result<libsql::Connection, Box<dyn std::error::Error + Send + Sync>> {
        let path = Self::local_db_path().ok_or("no local Sjel database file found")?;
        let path_str = path.to_str().ok_or("invalid UTF-8 database path")?;
        let db = libsql::Builder::new_local(path_str).build().await?;
        let conn = db.connect()?;
        conn.execute_batch("PRAGMA busy_timeout = 5000;").await?;
        Ok(conn)
    }

    /// Read projects directly from local LibSQL database if accessible, otherwise fallback to shell.
    pub async fn fetch_projects(&self) -> ShellResult<Vec<sjel_gpui_core::shell::ProjectSummary>> {
        if let Ok(conn) = Self::connect_local_db().await {
            if let Ok(mut rows) = conn
                .query(
                    "SELECT project, MAX(archived) as archived, \
                     SUM(CASE WHEN kind = 'task' AND done = 0 THEN 1 ELSE 0 END) as open, \
                     SUM(CASE WHEN kind = 'task' AND done != 0 THEN 1 ELSE 0 END) as done, \
                     SUM(CASE WHEN kind = 'note' THEN 1 ELSE 0 END) as notes \
                     FROM projects_records GROUP BY project ORDER BY project",
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
                    list.push(sjel_gpui_core::shell::ProjectSummary {
                        name,
                        archived: archived_int != 0,
                        open: open.max(0) as usize,
                        done: done.max(0) as usize,
                        notes: notes.max(0) as usize,
                    });
                }
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
        let shell = self.shell()?;
        shell.projects().await
    }

    /// Read tasks directly from local LibSQL database if accessible, otherwise fallback to shell.
    pub async fn fetch_tasks(&self, status: Option<&str>) -> ShellResult<Vec<sjel_gpui_core::shell::GlobalTask>> {
        if let Ok(conn) = Self::connect_local_db().await {
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
                    list.push(sjel_gpui_core::shell::GlobalTask {
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
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
        let shell = self.shell()?;
        shell.tasks(status).await
    }

    /// Read calendar entries directly from local LibSQL database if accessible, otherwise fallback to shell.
    pub async fn fetch_calendar(
        &self,
        from: &str,
        to: &str,
    ) -> ShellResult<Vec<sjel_gpui_core::shell::CalendarEntry>> {
        if let Ok(conn) = Self::connect_local_db().await {
            let from_bound = format!("{from}T00:00:00");
            let to_bound = format!("{to}T23:59:59");
            let query = "SELECT title, starts_at, ends_at, all_day, commitment \
                         FROM calendar_entries \
                         WHERE starts_at <= ?1 AND ends_at >= ?2 \
                         ORDER BY starts_at ASC";
            if let Ok(mut rows) = conn.query(query, libsql::params![to_bound, from_bound]).await {
                let mut list = Vec::new();
                while let Ok(Some(row)) = rows.next().await {
                    let title: String = row.get(0).unwrap_or_default();
                    let starts_at: String = row.get(1).unwrap_or_default();
                    let ends_at: String = row.get(2).unwrap_or_default();
                    let all_day_int: i64 = row.get(3).unwrap_or(0);
                    let commitment: String = row.get(4).unwrap_or_else(|_| "possible".into());
                    list.push(sjel_gpui_core::shell::CalendarEntry {
                        title,
                        starts_at,
                        ends_at,
                        all_day: all_day_int != 0,
                        commitment,
                    });
                }
                return Ok(list);
            }
        }
        let shell = self.shell()?;
        shell.calendar(from, to).await
    }

    /// Read project records directly from local LibSQL database if accessible, otherwise fallback to shell.
    pub async fn fetch_project_records(
        &self,
        project: &str,
    ) -> ShellResult<Vec<sjel_gpui_core::shell::ProjectRecord>> {
        if let Ok(conn) = Self::connect_local_db().await {
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
                    list.push(sjel_gpui_core::shell::ProjectRecord {
                        path,
                        kind,
                        title,
                        content: String::new(),
                        done: done_int != 0,
                        due,
                        priority: priority_int.clamp(1, 3) as u8,
                    });
                }
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
        let shell = self.shell()?;
        shell.project(project).await
    }

    /// Read a single record with its content directly from local LibSQL database, otherwise fallback to shell.
    pub async fn fetch_record(&self, path: &str) -> ShellResult<sjel_gpui_core::shell::ProjectRecord> {
        if let Ok(conn) = Self::connect_local_db().await {
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
                    return Ok(sjel_gpui_core::shell::ProjectRecord {
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
        let shell = self.shell()?;
        shell.record(path).await
    }
}

/// The last answer for one slot. A failed read keeps the value on screen and adds the error.
pub struct Remote<T> {
    pub value: Option<T>,
    pub error: Option<ShellError>,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
            loaded_at: None,
            loading: false,
        }
    }
}

impl<T> Remote<T> {
    pub fn new_loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }

    pub fn settle(&mut self, result: ShellResult<T>) {
        self.loading = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.loaded_at = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Neither loaded nor failed yet: the page reserves space and shows no error.
    pub fn first_load(&self) -> bool {
        self.value.is_none() && self.error.is_none()
    }

    /// Seconds since this value arrived, for a value that counts up on its own.
    pub fn age_of_value(&self) -> u64 {
        self.loaded_at.map(|at| at.elapsed().as_secs()).unwrap_or(0)
    }

    /// How long ago this value arrived, once it is past [`STALE_AFTER`].
    pub fn age(&self) -> Option<Duration> {
        self.loaded_at
            .map(|at| at.elapsed())
            .filter(|elapsed| *elapsed > STALE_AFTER)
    }

    /// Marks the slot in flight, or reports that a read is already running.
    fn begin(&mut self) -> bool {
        if self.loading {
            return false;
        }
        self.loading = true;
        true
    }
}

/// The button one write belongs to: the row it marks busy while it runs, and the sentence a
/// success shows.
pub struct Press<C> {
    slot: Option<fn(&mut C) -> &mut Busy>,
    key: String,
    done: &'static str,
}

impl<C> Press<C> {
    /// A press on a row: it cannot fire again until the answer arrives.
    pub fn once(slot: fn(&mut C) -> &mut Busy, key: impl Into<String>, done: &'static str) -> Self {
        Self {
            slot: Some(slot),
            key: key.into(),
            done,
        }
    }

    /// A write with no row of its own.
    pub fn quiet(done: &'static str) -> Self {
        Self {
            slot: None,
            key: String::new(),
            done,
        }
    }
}

/// The writes in flight, by key, so one button cannot be pressed twice.
#[derive(Default)]
pub struct Busy(HashSet<String>);

impl Busy {
    /// Marks `key` in flight, or reports that it already is.
    pub fn start(&mut self, key: &str) -> bool {
        self.0.insert(key.to_owned())
    }

    pub fn finish(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }
}

/// Reads into `slot` from the shell, unless that slot is already loading.
pub fn read<C, T, F, Fut>(
    cx: &mut Context<C>,
    this: &mut C,
    slot: fn(&mut C) -> &mut Remote<T>,
    repo: &Repo,
    request: F,
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Arc<Shell>) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    let shell = match repo.shell() {
        Ok(shell) => shell,
        Err(error) => {
            slot(this).settle(Err(error));
            cx.notify();
            return;
        }
    };
    if !slot(this).begin() {
        return;
    }
    cx.notify();
    let work = shell::run(request(shell));
    cx.spawn(async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update(cx, |this, cx| {
            slot(this).settle(result);
            cx.notify();
        });
    })
    .detach();
}

/// Reads into `slot` via `Repo` (which can use direct local DB or Shell fallback),
/// unless that slot is already loading.
pub fn read_repo<C, T, F, Fut>(
    cx: &mut Context<C>,
    this: &mut C,
    slot: fn(&mut C) -> &mut Remote<T>,
    repo: &Repo,
    request: F,
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Repo) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    if !slot(this).begin() {
        return;
    }
    cx.notify();
    let repo_clone = repo.clone();
    let work = shell::run(request(repo_clone));
    cx.spawn(async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update(cx, |this, cx| {
            slot(this).settle(result);
            cx.notify();
        });
    })
    .detach();
}

/// Runs one write, toasts its outcome, and then lets the caller apply what came back.
///
/// `then` receives the shell's answer, or none when the write failed, so a caller can patch one
/// row instead of reading again.
pub fn write<C, T, F, Fut>(
    cx: &mut Context<C>,
    window: &mut Window,
    this: &mut C,
    repo: &Repo,
    press: Press<C>,
    request: F,
    then: fn(&mut C, Option<T>, &mut Context<C>),
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Arc<Shell>) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    let shell = match repo.shell() {
        Ok(shell) => shell,
        Err(error) => {
            window.push_notification(Notification::error(error.to_string()), cx);
            return;
        }
    };
    let Press { slot, key, done } = press;
    if let Some(slot) = slot {
        if !slot(this).start(&key) {
            return;
        }
    }
    cx.notify();
    let work = shell::run(request(shell));
    cx.spawn_in(window, async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update_in(cx, |this, window, cx| {
            if let Some(slot) = slot {
                slot(this).finish(&key);
            }
            match result {
                Ok(value) => {
                    window.push_notification(Notification::success(done), cx);
                    then(this, Some(value), cx);
                }
                Err(error) => {
                    window.push_notification(Notification::error(error.to_string()), cx);
                    then(this, None, cx);
                }
            }
            cx.notify();
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
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
}

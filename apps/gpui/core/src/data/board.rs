//! The records list both apps draw, over a local replica of the Mac's projects.
//!
//! A phone has no database to read and no shell to ask, so it keeps its own copy: rows live in
//! the replica, every write is applied there first and logged in `_offline_mutations`, and the
//! list stays usable whether or not the Mac is reachable. That log is what a later slice replays
//! when the Mac answers again; nothing here sends anything.
//!
//! The view model is deliberately small and cloneable: a view holds one `Board`, replaces it
//! with the copy a mutation returns, and draws [`Board::tasks`] and [`Board::notes`]. No GPUI
//! type appears here, so the same list feeds a phone. The Mac app reads the host's live database
//! through [`Repo`](super::Repo) instead and does not queue writes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::shell::ProjectRecord;
use crate::store::{StoreResult, TursoStore};

/// The app's own copy of projects, tasks and notes, plus the count of writes waiting for the Mac.
#[derive(Clone)]
pub struct Board {
    store: Option<Arc<TursoStore>>,
    records: Vec<ProjectRecord>,
    pending: usize,
}

impl Board {
    /// Opens the replica at `path` and reads it.
    pub async fn open(path: impl Into<PathBuf>) -> StoreResult<Self> {
        Self::from_store(Arc::new(TursoStore::open(path).await?)).await
    }

    /// Opens a replica that exists only in this process, which is what a test wants.
    pub async fn in_memory() -> StoreResult<Self> {
        Self::from_store(Arc::new(TursoStore::in_memory().await?)).await
    }

    /// Wraps a store another owner already opened, and reads it.
    pub async fn from_store(store: Arc<TursoStore>) -> StoreResult<Self> {
        let mut board = Self {
            store: Some(store),
            records: Vec::new(),
            pending: 0,
        };
        board.reload().await?;
        Ok(board)
    }

    /// Constructs a Board directly from existing records with no backing store.
    /// Useful for desktop viewing where records come directly from [`Repo`](super::Repo).
    pub fn from_records(records: Vec<ProjectRecord>) -> Self {
        Self {
            store: None,
            records,
            pending: 0,
        }
    }

    /// The file this platform keeps the replica in, or `None` where it keeps none.
    ///
    /// A phone writes into its app container. The Mac app does not keep a replica at all — it
    /// reads the host's database through [`Repo`](super::Repo), so queueing a write into a second
    /// copy of the same records would be a second source of truth. `SJEL_REPLICA_PATH` names one
    /// anyway, which is how the Mac can try the phone's view.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("SJEL_REPLICA_PATH") {
            return Some(PathBuf::from(path));
        }
        replica_path()
    }

    /// The replica itself, for the sync routine that replays and pulls.
    pub fn store(&self) -> Option<&Arc<TursoStore>> {
        self.store.as_ref()
    }

    pub fn records(&self) -> &[ProjectRecord] {
        &self.records
    }

    /// How many writes are waiting to reach the Mac.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Every task, done ones included, in the store's order: open first, then priority, then path.
    pub fn tasks(&self) -> impl Iterator<Item = &ProjectRecord> {
        self.records.iter().filter(|record| record.kind == "task")
    }

    /// Tasks still to do, for a list that asks what is next.
    pub fn open_tasks(&self) -> impl Iterator<Item = &ProjectRecord> {
        self.tasks().filter(|record| !record.done)
    }

    /// Every record that is not a task.
    pub fn notes(&self) -> impl Iterator<Item = &ProjectRecord> {
        self.records.iter().filter(|record| record.kind != "task")
    }

    /// Ticks a task in the list before the store answers, so the tap lands in the frame it was
    /// made in. The write below settles the truth.
    pub fn mark_done(&mut self, path: &str, done: bool) {
        if let Some(record) = self.records.iter_mut().find(|record| record.path == path) {
            record.done = done;
        }
    }

    /// Shows a note's new body before the store answers, the way [`Board::mark_done`] shows a
    /// tick. The save below settles the truth.
    pub fn set_content(&mut self, path: &str, content: &str) {
        if let Some(record) = self.records.iter_mut().find(|record| record.path == path) {
            content.clone_into(&mut record.content);
        }
    }

    /// Re-reads the rows and the queued count.
    pub async fn reload(&mut self) -> StoreResult<()> {
        if let Some(store) = &self.store {
            self.records = store.fetch_all_records().await?;
            self.pending = store.pending_mutation_count().await?;
        }
        Ok(())
    }

    /// Changes a task's priority in memory immediately for snappy UI response.
    pub fn mark_priority(&mut self, path: &str, priority: u8) {
        if let Some(record) = self.records.iter_mut().find(|record| record.path == path) {
            record.priority = priority;
        }
    }

    /// Cycles priority: P1 -> P2 -> P3 -> P1.
    pub fn cycle_priority(&mut self, path: &str) -> u8 {
        let mut next = 1;
        if let Some(record) = self.records.iter_mut().find(|record| record.path == path) {
            next = match record.priority {
                1 => 2,
                2 => 3,
                _ => 1,
            };
            record.priority = next;
        }
        next
    }

    /// Persists the new priority to the replica and queues for sync, or updates in-memory.
    pub async fn set_priority(&mut self, path: &str, priority: u8) -> StoreResult<()> {
        if let Some(store) = &self.store {
            store.set_task_priority(path, priority).await?;
            self.reload().await
        } else {
            self.mark_priority(path, priority);
            Ok(())
        }
    }

    /// Deletes a record from the replica and queues removal for sync, or deletes from in-memory.
    pub async fn delete_record(&mut self, path: &str) -> StoreResult<()> {
        if let Some(store) = &self.store {
            store.delete_record(path).await?;
            self.reload().await
        } else {
            self.records.retain(|r| r.path != path);
            Ok(())
        }
    }

    /// Ticks a task in the replica and logs the write for the Mac, or toggles in-memory.
    pub async fn toggle_task(&mut self, path: &str, done: bool) -> StoreResult<()> {
        if let Some(store) = &self.store {
            store.set_task_done(path, done).await?;
            self.reload().await
        } else {
            self.mark_done(path, done);
            Ok(())
        }
    }

    /// Adds a task to `project` and returns its path, which is [`task_path`] of now.
    pub async fn add_task(&mut self, project: &str, title: &str) -> StoreResult<String> {
        let path = task_path(project, SystemTime::now());
        if let Some(store) = &self.store {
            store.save_record(project, &path, "task", title, "").await?;
            self.reload().await?;
        } else {
            self.records.push(ProjectRecord {
                project: String::new(),
                path: path.clone(),
                title: title.to_string(),
                kind: "task".to_string(),
                content: String::new(),
                done: false,
                due: None,
                priority: 2,
            });
        }
        Ok(path)
    }

    /// Saves a note's body, creating the note when the replica has never seen it.
    pub async fn save_note(
        &mut self,
        project: &str,
        path: &str,
        title: &str,
        content: &str,
    ) -> StoreResult<()> {
        if let Some(store) = &self.store {
            store
                .save_record(project, path, "note", title, content)
                .await?;
            self.reload().await
        } else {
            if let Some(record) = self.records.iter_mut().find(|r| r.path == path) {
                record.title = title.to_string();
                record.content = content.to_string();
            } else {
                self.records.push(ProjectRecord {
                    project: String::new(),
                    path: path.to_string(),
                    title: title.to_string(),
                    kind: "note".to_string(),
                    content: content.to_string(),
                    done: false,
                    due: None,
                    priority: 2,
                });
            }
            Ok(())
        }
    }
}

/// `<project>/Task-<seconds>.md`: a path two captures a second apart cannot share.
pub fn task_path(project: &str, at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    format!("{}/Task-{secs}.md", project.trim_end_matches('/'))
}

/// Where this platform keeps an app-owned replica.
#[cfg(target_os = "ios")]
fn replica_path() -> Option<PathBuf> {
    // The app container's Application Support, beside the saved pairing. The directory can be
    // missing on a first run, and `TursoStore::open` will not create it.
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join("Library/Application Support");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("sjel.db"))
}

/// Nowhere: the Mac app reads the host's database, so it keeps no replica of its own.
#[cfg(not(target_os = "ios"))]
fn replica_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// The crate's tests build their own runtime, because `core` does not enable tokio's macros.
    fn blocking<T>(work: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(work)
    }

    #[test]
    fn a_board_reads_back_what_it_wrote() {
        blocking(async {
            let mut board = Board::in_memory().await.expect("an empty replica");
            assert_eq!(board.records().len(), 0);
            assert_eq!(board.pending(), 0);

            let path = board
                .add_task("Inbox", "Buy milk")
                .await
                .expect("a task is added");
            assert_eq!(board.tasks().count(), 1);
            assert_eq!(board.open_tasks().count(), 1);
            assert_eq!(board.pending(), 1, "the save is queued for the Mac");

            board
                .toggle_task(&path, true)
                .await
                .expect("the task is ticked");
            assert_eq!(board.tasks().count(), 1, "a done task is still a task");
            assert_eq!(board.open_tasks().count(), 0);
            assert_eq!(board.pending(), 2, "the tick is queued too");
        });
    }

    #[test]
    fn a_note_is_not_a_task() {
        blocking(async {
            let mut board = Board::in_memory().await.expect("an empty replica");
            board
                .save_note("Inbox", "Inbox/Plan.md", "Plan", "# Plan\n")
                .await
                .expect("a note is saved");
            assert_eq!(board.notes().count(), 1);
            assert_eq!(board.tasks().count(), 0);
            let kept = board
                .store()
                .expect("store is present")
                .fetch_record("Inbox/Plan.md")
                .await
                .expect("the replica answers")
                .expect("the note is there");
            assert_eq!(kept.content, "# Plan\n");
        });
    }

    #[test]
    fn a_tick_shows_before_the_store_answers() {
        blocking(async {
            let mut board = Board::in_memory().await.expect("an empty replica");
            let path = board.add_task("Inbox", "Buy milk").await.expect("a task");
            board.mark_done(&path, true);
            assert_eq!(board.open_tasks().count(), 0, "the row is ticked at once");
            assert_eq!(board.pending(), 1, "and nothing was written yet");

            board
                .toggle_task(&path, true)
                .await
                .expect("the write lands");
            assert_eq!(board.open_tasks().count(), 0);
        });
    }

    #[test]
    fn a_typed_body_shows_before_the_store_answers() {
        blocking(async {
            let mut board = Board::in_memory().await.expect("an empty replica");
            board
                .save_note("Inbox", "Inbox/Plan.md", "Plan", "# Plan\n")
                .await
                .expect("a note");
            board.set_content("Inbox/Plan.md", "# Plan\n\n- [x] done\n");
            let shown = board.notes().next().expect("the note is in the list");
            assert_eq!(shown.content, "# Plan\n\n- [x] done\n");
            assert_eq!(board.pending(), 1, "the tick in the body is not saved yet");

            // A path the list does not hold is nothing, not a panic.
            board.set_content("Inbox/Gone.md", "# Gone\n");
        });
    }

    #[test]
    fn a_board_cycles_and_persists_task_priority_and_deletes_record() {
        blocking(async {
            let mut board = Board::in_memory().await.expect("an empty replica");
            let path = board.add_task("Inbox", "Priority test").await.unwrap();
            assert_eq!(board.tasks().next().unwrap().priority, 2);

            let next = board.cycle_priority(&path);
            assert_eq!(next, 3);
            assert_eq!(board.tasks().next().unwrap().priority, 3);

            board.set_priority(&path, 1).await.unwrap();
            assert_eq!(board.tasks().next().unwrap().priority, 1);
            assert_eq!(board.pending(), 2);

            board.delete_record(&path).await.unwrap();
            assert_eq!(board.tasks().count(), 0);
            assert_eq!(board.pending(), 3);
        });
    }

    #[test]
    fn two_tasks_a_second_apart_get_different_paths() {
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let first = task_path("Inbox", at);
        let second = task_path("Inbox", at + Duration::from_secs(1));
        assert_eq!(first, "Inbox/Task-1700000000.md");
        assert_ne!(first, second);
        assert_eq!(
            task_path("Inbox/", at),
            first,
            "a trailing slash is not a path"
        );
    }

    #[test]
    fn board_from_records_queries_and_mutates_in_memory() {
        let records = vec![
            ProjectRecord {
                project: String::new(),
                path: "Project/Task-1.md".to_string(),
                title: "Task 1".to_string(),
                kind: "task".to_string(),
                content: String::new(),
                done: false,
                due: None,
                priority: 1,
            },
            ProjectRecord {
                project: String::new(),
                path: "Project/Note-1.md".to_string(),
                title: "Note 1".to_string(),
                kind: "note".to_string(),
                content: "# Hello".to_string(),
                done: false,
                due: None,
                priority: 2,
            },
        ];

        let mut board = Board::from_records(records);
        assert_eq!(board.tasks().count(), 1);
        assert_eq!(board.open_tasks().count(), 1);
        assert_eq!(board.notes().count(), 1);
        assert_eq!(board.pending(), 0);
        assert!(board.store().is_none());

        board.mark_done("Project/Task-1.md", true);
        assert_eq!(board.open_tasks().count(), 0);
        assert_eq!(board.tasks().count(), 1);

        let next_p = board.cycle_priority("Project/Task-1.md");
        assert_eq!(next_p, 2);
    }
}

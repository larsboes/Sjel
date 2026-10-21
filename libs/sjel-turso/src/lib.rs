//! Turso / LibSQL storage engine for Sjel.
//!
//! Provides async multi-writer concurrency, MVCC, embedded replicas, and
//! vector search over Sjel's shared SQLite-compatible database (`axon.db`).

use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Re-export the underlying LibSQL crate so capabilities do not depend on it directly.
pub use libsql;

#[cfg(feature = "replica")]
mod replica;

/// JWT minting for a primary that authenticates its clients, re-exported for the `replica`
/// feature's callers: an embedded replica cannot write to a `sqld` that demands a credential.
///
/// Implemented in `libsql-auth`, which carries no libsql at all — `tools/sjel-cli` needs to mint
/// and must not link a second SQLite into the same binary as `sjel-store`.
#[cfg(feature = "replica")]
pub use libsql_auth::{Access, PrimaryKey};

/// Embedded-replica configuration, re-exported for the `replica` feature's callers.
#[cfg(feature = "replica")]
pub use replica::{ReplicaConfig, SyncStats};

/// What every fallible operation in this crate returns.
pub type Error = Box<dyn StdError + Send + Sync>;

/// The result of a fallible operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The canonical stored timestamp, as a SQL expression.
///
/// Renders `2026-08-27 21:23:35.871+00:00`: UTC, millisecond resolution,
/// ISO-8601 with a space separator. Interpolate where needed:
///
/// ```text
/// format!("UPDATE {p}_tasks SET updated_at = {now}", p = self.prefix, now = sjel_turso::NOW)
/// ```
pub const NOW: &str = "strftime('%Y-%m-%d %H:%M:%f+00:00','now')";

/// The canonical stamp's `strftime` format.
pub const STAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%f+00:00";

/// [`NOW`] shifted by a SQLite/LibSQL date modifier, in canonical format.
///
/// ```text
/// now_offset("'+30 days'")
/// now_offset("'+' || MIN(attempts + 1, 5) || ' minutes'")
/// ```
pub fn now_offset(modifier: &str) -> String {
    format!("strftime('{STAMP_FORMAT}','now',{modifier})")
}

/// Format a slice of `f32` numbers into a LibSQL vector literal string (`"[0.1, 0.2, 0.3]"`).
pub fn format_vector(vector: &[f32]) -> String {
    let mut out = String::with_capacity(vector.len() * 8 + 2);
    out.push('[');
    for (i, v) in vector.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        use std::fmt::Write;
        let _ = write!(&mut out, "{v}");
    }
    out.push(']');
    out
}

/// Standard PRAGMAs applied to every local LibSQL connection.
pub const CONNECTION_PRAGMAS: &str = "
    PRAGMA busy_timeout = 5000;
    PRAGMA foreign_keys = ON;
";

/// One entry in the process-wide database registry: a local file or its replica, never both.
enum DbEntry {
    Local(Arc<libsql::Database>),
    #[cfg(feature = "replica")]
    Replica(Arc<replica::Replica>),
}

/// The registry is a `tokio::sync::Mutex` because opening a database awaits.
///
/// Two tasks that opened the same path concurrently would otherwise each build a `Database` on one
/// file and then sync or write it twice. Holding the guard across the open makes the second caller
/// wait for the first and then share it; openings happen once per path per process, so the
/// serialization costs nothing after the first caller.
fn database_registry() -> &'static tokio::sync::Mutex<HashMap<PathBuf, DbEntry>> {
    static REGISTRY: OnceLock<tokio::sync::Mutex<HashMap<PathBuf, DbEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

fn canonical_key(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("database path {} names no file", path.display()))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    Ok(parent.canonicalize()?.join(name))
}

/// Return or initialize a shared [`libsql::Database`] instance for a local database path.
///
/// # Errors
/// The path names no file, the file is already open as an embedded replica in this process, or
/// LibSQL refused to open it.
pub async fn database_for(path: impl AsRef<Path>) -> Result<Arc<libsql::Database>> {
    local_entry(&canonical_key(path.as_ref())?).await
}

/// The registry's local entry for `path`, opened on first use and shared afterwards.
async fn local_entry(path: &Path) -> Result<Arc<libsql::Database>> {
    let mut registry = database_registry().lock().await;
    match registry.get(path) {
        Some(DbEntry::Local(db)) => return Ok(Arc::clone(db)),
        #[cfg(feature = "replica")]
        Some(DbEntry::Replica(entry)) => {
            return Err(format!(
                "{} is already open as an embedded replica of {} in this process; a file is either \
                 the shared local database or a replica, and LibSQL cannot be both on one file",
                path.display(),
                entry.config().public_url()
            )
            .into())
        }
        None => {}
    }

    let path_str = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 database path: {}", path.display()))?;
    let db = Arc::new(libsql::Builder::new_local(path_str).build().await?);
    registry.insert(path.to_path_buf(), DbEntry::Local(Arc::clone(&db)));
    Ok(db)
}

/// The registry's replica entry for `path`, opened and pulled once on first use.
#[cfg(feature = "replica")]
async fn replica_entry(
    path: &Path,
    config: replica::ReplicaConfig,
) -> Result<Arc<replica::Replica>> {
    let mut registry = database_registry().lock().await;
    match registry.get(path) {
        Some(DbEntry::Replica(entry)) => {
            return match entry.config().difference(&config) {
                None => Ok(Arc::clone(entry)),
                Some(field) => Err(format!(
                    "{} is already open as a replica of {} in this process with a different {field}; \
                     one file cannot follow two primaries or two intervals, so close the first open \
                     or align the configuration",
                    path.display(),
                    entry.config().public_url()
                )
                .into()),
            }
        }
        Some(DbEntry::Local(_)) => {
            return Err(format!(
                "{} is already open as the shared local database in this process; a replica of a \
                 remote primary needs its own file, and pointing one at the live file would replace \
                 local state with the primary's log",
                path.display()
            )
            .into())
        }
        None => {}
    }

    let entry = replica::Replica::open(path, config).await?;
    entry.start_background();
    registry.insert(path.to_path_buf(), DbEntry::Replica(Arc::clone(&entry)));
    Ok(entry)
}

/// Open a new connection to a local LibSQL database with standard PRAGMAs applied.
pub async fn open_local(path: impl AsRef<Path>) -> Result<libsql::Connection> {
    let db = database_for(path).await?;
    let conn = db.connect()?;
    conn.execute_batch(CONNECTION_PRAGMAS).await?;
    Ok(conn)
}

fn migrated() -> &'static Mutex<HashSet<String>> {
    static MIGRATED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    MIGRATED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Runs migration DDL for `prefix` on `path` once per process lifetime.
///
/// Ensures multiple background tasks or HTTP handlers in the same service
/// do not repeat DDL execution.
pub async fn migrate_once<F, Fut>(
    conn: &libsql::Connection,
    path: impl AsRef<Path>,
    prefix: &str,
    ddl: F,
) -> Result<()>
where
    F: FnOnce(libsql::Connection) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let key = canonical_key(path.as_ref())?;
    let done_key = format!("{}\u{1f}{prefix}", key.to_string_lossy());
    {
        let done = migrated()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if done.contains(&done_key) {
            return Ok(());
        }
    }

    ddl(conn.clone()).await?;

    let mut done = migrated()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    done.insert(done_key);
    Ok(())
}

/// A shared database, opened once per path, with Sjel's PRAGMAs and prefix migration applied.
///
/// The async counterpart of `sjel_store::open_pool`: a capability holds one `Store` for the life of
/// the process and calls [`Store::connect`] to read or [`Store::begin`] to write. The file is the
/// same one every other capability's prefix lives in; only the engine differs.
///
/// ```no_run
/// # async fn example() -> Result<(), sjel_turso::Error> {
/// let store = sjel_turso::Store::open("/path/axon.db", "places", |conn| async move {
///     conn.execute("CREATE TABLE IF NOT EXISTS places_items (id TEXT PRIMARY KEY)", ())
///         .await?;
///     Ok(())
/// })
/// .await?;
/// let conn = store.connect().await?;
/// # Ok(())
/// # }
/// ```
pub struct Store {
    path: PathBuf,
    db: Arc<libsql::Database>,
    #[cfg(feature = "replica")]
    replica: Option<Arc<replica::Replica>>,
}

/// Where a [`Store`] gets its data.
#[derive(Debug, Clone)]
pub enum Source {
    /// The shared local file that every capability's prefix lives in.
    Local,
    /// A local copy of a remote primary, pulled in the background.
    ///
    /// Reads never leave the machine. Writes do: LibSQL forwards them to the primary, so a replica
    /// buys read latency and costs write latency.
    #[cfg(feature = "replica")]
    Replica(ReplicaConfig),
}

impl Source {
    /// The source the deployment asks for: a replica when [`ReplicaConfig::from_env`] names one,
    /// else the shared local file.
    ///
    /// # Errors
    /// A replica is configured and unusable. That is an error rather than a quiet fall back to the
    /// local file, because a device that meant to follow a primary and instead reads a stale local
    /// copy reports neither.
    #[cfg(feature = "replica")]
    pub fn from_env() -> Result<Self> {
        Ok(match ReplicaConfig::from_env()? {
            Some(config) => Self::Replica(config),
            None => Self::Local,
        })
    }
}

impl Store {
    /// Open the database at `path`, apply the PRAGMAs, and run `ddl` once per process for `prefix`.
    ///
    /// The prefix is the capability's namespace in the shared file, exactly as it is for
    /// `sjel_store::open_pool`. Reopening the same path in the same process reuses the connection
    /// pool and skips the DDL.
    ///
    /// # Errors
    /// The path names no file, or the DDL failed.
    pub async fn open<F, Fut>(path: impl AsRef<Path>, prefix: &str, ddl: F) -> Result<Self>
    where
        F: FnOnce(libsql::Connection) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        Self::open_with(path, prefix, Source::Local, ddl).await
    }

    /// Open the local file, or a synced copy of a remote primary, with one call site.
    ///
    /// This is the shape a capability branches on once, where `source` comes from its own config:
    /// the migration and the PRAGMAs are the same either way, so the branch is an open call and not
    /// a second store.
    ///
    /// # Errors
    /// The path names no file, the replica is misconfigured, the primary refused the pull, or the
    /// DDL failed.
    pub async fn open_with<F, Fut>(
        path: impl AsRef<Path>,
        prefix: &str,
        source: Source,
        ddl: F,
    ) -> Result<Self>
    where
        F: FnOnce(libsql::Connection) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let path = canonical_key(path.as_ref())?;
        match source {
            Source::Local => {
                let db = local_entry(&path).await?;
                let store = Self::local(path, db);
                store.initialise(prefix, ddl).await?;
                Ok(store)
            }
            #[cfg(feature = "replica")]
            Source::Replica(config) => {
                let entry = replica_entry(&path, config).await?;
                let db = entry.database();
                let store = Self {
                    path,
                    db,
                    replica: Some(entry),
                };
                store.initialise(prefix, ddl).await?;
                Ok(store)
            }
        }
    }

    /// Open `path` as a local copy of the remote primary `config` names.
    ///
    /// The first pull happens before the DDL runs, so the migration decides the schema against the
    /// primary's current state rather than against an empty file. After that, a background task
    /// pulls every [`ReplicaConfig::interval`].
    ///
    /// Like every other open, the first pull runs while the process-wide database registry is held,
    /// so a primary that never answers holds up the next open in this process instead of letting one
    /// capability run against a copy nobody verified.
    ///
    /// # Errors
    /// Same as [`Store::open_with`], plus a refused first pull.
    #[cfg(feature = "replica")]
    pub async fn open_replica<F, Fut>(
        path: impl AsRef<Path>,
        prefix: &str,
        config: ReplicaConfig,
        ddl: F,
    ) -> Result<Self>
    where
        F: FnOnce(libsql::Connection) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        Self::open_with(path, prefix, Source::Replica(config), ddl).await
    }

    /// Run this prefix's DDL on a fresh connection, once per process.
    async fn initialise<F, Fut>(&self, prefix: &str, ddl: F) -> Result<()>
    where
        F: FnOnce(libsql::Connection) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let conn = self.connection().await?;
        migrate_once(&conn, &self.path, prefix, ddl).await
    }

    /// A local store, in the shape the struct takes when no replica is configured.
    fn local(path: PathBuf, db: Arc<libsql::Database>) -> Self {
        Self {
            path,
            db,
            #[cfg(feature = "replica")]
            replica: None,
        }
    }

    /// Pull the primary's log now, instead of waiting for the next background interval.
    ///
    /// A read is current to the last successful pull. This is the call that makes it current now,
    /// at the cost of one network round trip, so a surface that must not show a stale row asks for
    /// it before reading rather than shortening the interval for everyone.
    ///
    /// # Errors
    /// The store is local, or the pull failed.
    #[cfg(feature = "replica")]
    pub async fn sync_now(&self) -> Result<Option<u64>> {
        let Some(entry) = self.replica.as_ref() else {
            return Err(
                "this store is local; open it with `Store::open_replica` to pull from a \
                        primary"
                    .into(),
            );
        };
        entry.sync_now().await
    }

    /// How this store's pulls have gone, or `None` when the store is local.
    ///
    /// `seconds_since_success` against the configured interval is the honest staleness bound: a
    /// replica that has not synced for three intervals is a replica whose reads nobody should trust.
    #[cfg(feature = "replica")]
    pub fn sync_stats(&self) -> Option<SyncStats> {
        self.replica.as_ref().map(|entry| entry.stats())
    }

    /// Whether this store reads a local copy of a remote primary.
    #[cfg(feature = "replica")]
    pub fn is_replica(&self) -> bool {
        self.replica.is_some()
    }

    /// The canonical path of the file this store holds.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A connection for one read, with the PRAGMAs applied where the engine accepts them.
    ///
    /// # Errors
    /// The database refused a connection.
    pub async fn connect(&self) -> Result<libsql::Connection> {
        self.connection().await
    }

    /// The one place a connection is built, so the PRAGMA policy cannot drift between reads, writes
    /// and migrations.
    async fn connection(&self) -> Result<libsql::Connection> {
        let conn = self.db.connect()?;
        #[cfg(feature = "replica")]
        if self.replica.is_some() {
            // A replica's connection is LibSQL's `RemoteConnection`, which parses every statement
            // to decide whether the primary must serve it. Its parser classifies the assigned form
            // of a PRAGMA as a write to the primary, and rejects `PRAGMA busy_timeout = <n>`
            // outright, so `execute_batch(CONNECTION_PRAGMAS)` fails on exactly the connection this
            // store exists for. The settings are also moot here: writes are the primary's, which
            // applies its own, and reads never take a writer's lock. Verify against
            // `libsql::parser::pragma_kind` and `libsql::replication::connection`'s
            // `should_execute_local` before assuming a later libsql version lifted the limit.
            return Ok(conn);
        }
        conn.execute_batch(CONNECTION_PRAGMAS).await?;
        Ok(conn)
    }

    /// A write transaction on its own connection.
    ///
    /// The caller decides the outcome: `commit` keeps the writes, and dropping the returned
    /// transaction rolls them back, so a failed write leaves the file as it was.
    ///
    /// It takes the writer lock up front, the same reason `sjel_store::write_transaction` starts
    /// `BEGIN IMMEDIATE`: a deferred transaction that reads before it writes has to upgrade into
    /// the lock, and a failed upgrade is refused at once instead of waiting out `busy_timeout`.
    ///
    /// # Errors
    /// The database refused a connection or a transaction.
    pub async fn begin(&self) -> Result<libsql::Transaction> {
        let conn = self.connection().await?;
        Ok(conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?)
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("Store");
        out.field("path", &self.path);
        #[cfg(feature = "replica")]
        out.field(
            "replica_url",
            &self
                .replica
                .as_ref()
                .map(|entry| entry.config().public_url()),
        );
        out.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-turso-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{tag}.db"))
    }

    #[tokio::test]
    async fn local_open_and_pragmas() {
        let db_path = temp_db("local_pragmas");
        let conn = open_local(&db_path).await.unwrap();

        // Check foreign_keys pragma is enabled
        let mut rows = conn.query("PRAGMA foreign_keys", ()).await.unwrap();
        let row = rows.next().await.unwrap().expect("pragma row");
        let fk: i64 = row.get(0).unwrap();
        assert_eq!(fk, 1, "foreign_keys must be enabled by default");
    }

    #[tokio::test]
    async fn migrate_once_tracks_executions() {
        let db_path = temp_db("migrate_test");
        let conn = open_local(&db_path).await.unwrap();

        let run_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..3 {
            let count_clone = run_count.clone();
            migrate_once(&conn, &db_path, "sample", |conn| async move {
                count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS sample_items (id INTEGER PRIMARY KEY, name TEXT);",
                    (),
                )
                .await?;
                Ok(())
            })
            .await
            .unwrap();
        }

        assert_eq!(run_count.load(std::sync::atomic::Ordering::SeqCst), 1);

        conn.execute(
            "INSERT INTO sample_items (name) VALUES (?1)",
            ["turso test"],
        )
        .await
        .unwrap();

        let mut rows = conn
            .query("SELECT name FROM sample_items WHERE id = 1", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("item row");
        let name: String = row.get(0).unwrap();
        assert_eq!(name, "turso test");
    }

    #[test]
    fn timestamps_agree_with_sjel_standard() {
        assert!(NOW.contains("strftime"));
        let offset = now_offset("'+7 days'");
        assert!(offset.contains("'+7 days'"));
        assert!(offset.contains(STAMP_FORMAT));
    }

    #[tokio::test]
    async fn native_vector_index_and_top_k() {
        let db_path = temp_db("vector_test");
        let conn = open_local(&db_path).await.unwrap();

        conn.execute_batch(
            "
            CREATE TABLE vector_docs (
                id INTEGER PRIMARY KEY,
                title TEXT,
                embedding F32_BLOB(3)
            );
            CREATE INDEX vector_docs_idx ON vector_docs(libsql_vector_idx(embedding));
            ",
        )
        .await
        .unwrap();

        let v1 = format_vector(&[1.0, 0.0, 0.0]);
        let v2 = format_vector(&[0.0, 1.0, 0.0]);
        let v3 = format_vector(&[0.0, 0.0, 1.0]);

        conn.execute(
            "INSERT INTO vector_docs (id, title, embedding) VALUES (1, 'Doc X', vector(?1))",
            [v1.as_str()],
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO vector_docs (id, title, embedding) VALUES (2, 'Doc Y', vector(?1))",
            [v2.as_str()],
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO vector_docs (id, title, embedding) VALUES (3, 'Doc Z', vector(?1))",
            [v3.as_str()],
        )
        .await
        .unwrap();

        // Query nearest to [0.95, 0.1, 0.0]
        let query_vec = format_vector(&[0.95, 0.1, 0.0]);
        let mut rows = conn
            .query(
                "SELECT id, title, vector_distance_cos(embedding, vector(?1)) as dist 
                 FROM vector_docs 
                 ORDER BY dist ASC 
                 LIMIT 1",
                [query_vec.as_str()],
            )
            .await
            .unwrap();

        let row = rows.next().await.unwrap().expect("nearest row");
        let id: i64 = row.get(0).unwrap();
        let title: String = row.get(1).unwrap();
        let dist: f64 = row.get(2).unwrap();

        assert_eq!(id, 1);
        assert_eq!(title, "Doc X");
        assert!(dist < 0.05, "cosine distance must be very close to 0");
    }

    /// The DDL a `Store::open` call runs, in the shape a capability writes it.
    async fn create_notes(conn: libsql::Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS notes_items (id INTEGER PRIMARY KEY, body TEXT)",
            (),
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn store_commits_a_written_transaction() {
        let db_path = temp_db("store_commit");
        let store = Store::open(&db_path, "notes", create_notes).await.unwrap();

        let tx = store.begin().await.unwrap();
        tx.execute("INSERT INTO notes_items (body) VALUES ('kept')", ())
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let conn = store.connect().await.unwrap();
        let mut rows = conn
            .query("SELECT body FROM notes_items", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("committed row");
        let body: String = row.get(0).unwrap();
        assert_eq!(body, "kept");
    }

    #[tokio::test]
    async fn dropping_a_transaction_writes_nothing() {
        let db_path = temp_db("store_rollback");
        let store = Store::open(&db_path, "notes", create_notes).await.unwrap();

        {
            let tx = store.begin().await.unwrap();
            tx.execute("INSERT INTO notes_items (body) VALUES ('dropped')", ())
                .await
                .unwrap();
        }
        tokio::task::yield_now().await;

        let conn = store.connect().await.unwrap();
        let mut rows = conn
            .query("SELECT COUNT(*) FROM notes_items", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("count row");
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 0, "an uncommitted write must not reach the file");
    }

    #[tokio::test]
    async fn reopening_the_same_path_skips_the_ddl() {
        let db_path = temp_db("store_reopen");
        Store::open(&db_path, "notes", create_notes).await.unwrap();

        // The second open runs its own DDL only if the once-per-process record is missing. A
        // closure that creates a table the first run did not would fail here with a duplicate.
        Store::open(&db_path, "notes", |conn| async move {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS notes_items (id INTEGER PRIMARY KEY, body TEXT)",
                (),
            )
            .await?;
            Ok(())
        })
        .await
        .unwrap();

        let store = Store::open(&db_path, "notes", |conn| async move {
            conn.execute("CREATE TABLE notes_items (id INTEGER PRIMARY KEY)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(store.path(), canonical_key(&db_path).unwrap());
    }

    #[tokio::test]
    async fn a_store_connection_carries_the_pragmas() {
        let db_path = temp_db("store_pragmas");
        let store = Store::open(&db_path, "notes", create_notes).await.unwrap();

        let conn = store.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA foreign_keys", ()).await.unwrap();
        let row = rows.next().await.unwrap().expect("pragma row");
        let foreign_keys: i64 = row.get(0).unwrap();
        assert_eq!(
            foreign_keys, 1,
            "every connection a store hands out must run with referential integrity on"
        );
    }

    #[tokio::test]
    async fn a_store_crosses_tokio_workers() {
        let db_path = temp_db("store_send");
        let store = Arc::new(Store::open(&db_path, "notes", create_notes).await.unwrap());

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let store = store.clone();
                tokio::spawn(async move {
                    let tx = store.begin().await.unwrap();
                    tx.execute(
                        "INSERT INTO notes_items (body) VALUES (?1)",
                        [format!("note-{i}")],
                    )
                    .await
                    .unwrap();
                    tx.commit().await.unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.await.unwrap();
        }

        let conn = store.connect().await.unwrap();
        let mut rows = conn
            .query("SELECT COUNT(*) FROM notes_items", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("count row");
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 8, "every concurrent writer must land");
    }

    /// `begin` takes the writer lock up front, before its transaction reads or writes anything.
    ///
    /// The one observable difference: while `begin`'s transaction is open, a second connection's
    /// deferred transaction can still read but is refused the moment it writes — which can only
    /// happen because the lock is already held. With a deferred `begin` the lock would still be
    /// free at that point and that write would land, which is the failure this guards.
    ///
    /// The refusal is immediate rather than waited out: SQLite does not retry a failed lock
    /// upgrade, so `busy_timeout` never enters into it.
    #[tokio::test]
    async fn begin_takes_the_writer_lock_before_it_reads_or_writes() {
        let db_path = temp_db("store_immediate");
        let store = Store::open(&db_path, "notes", create_notes).await.unwrap();

        let held = store.begin().await.unwrap();
        let other = store.connect().await.unwrap();
        // Deferred on purpose: being refused the upgrade is the subject of this test, so the
        // behaviour is named rather than left to the default.
        let other_tx = other
            .transaction_with_behavior(libsql::TransactionBehavior::Deferred)
            .await
            .unwrap();
        let mut rows = other_tx
            .query("SELECT COUNT(*) FROM notes_items", ())
            .await
            .unwrap();
        rows.next().await.unwrap().expect("count row");
        drop(rows);

        let message = other_tx
            .execute("INSERT INTO notes_items (body) VALUES ('other')", ())
            .await
            .expect_err("the writer lock is already held by `begin`")
            .to_string();
        drop(other_tx);

        held.commit().await.unwrap();
        assert!(
            message.contains("locked"),
            "unexpected refusal from the upgrade: {message}"
        );
    }
}

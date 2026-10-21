//! Turso / LibSQL storage engine for Sjel.
//!
//! Provides async multi-writer concurrency, MVCC, embedded replicas, and
//! vector search over Sjel's shared SQLite-compatible database (`axon.db`).

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Re-export the underlying LibSQL crate so capabilities do not depend on it directly.
pub use libsql;

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

fn database_registry() -> &'static Mutex<HashMap<PathBuf, Arc<libsql::Database>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<libsql::Database>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canonical_key(path: &Path) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
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
pub async fn database_for(
    path: impl AsRef<Path>,
) -> Result<Arc<libsql::Database>, Box<dyn Error + Send + Sync>> {
    let key = canonical_key(path.as_ref())?;
    {
        let registry = database_registry()
            .lock()
            .map_err(|e| format!("database registry lock poisoned: {e}"))?;
        if let Some(existing) = registry.get(&key) {
            return Ok(existing.clone());
        }
    }

    let path_str = key
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 database path: {}", key.display()))?;
    let db = Arc::new(libsql::Builder::new_local(path_str).build().await?);

    let mut registry = database_registry()
        .lock()
        .map_err(|e| format!("database registry lock poisoned: {e}"))?;
    if let Some(existing) = registry.get(&key) {
        return Ok(existing.clone());
    }
    registry.insert(key, db.clone());
    Ok(db)
}

/// Open a new connection to a local LibSQL database with standard PRAGMAs applied.
pub async fn open_local(
    path: impl AsRef<Path>,
) -> Result<libsql::Connection, Box<dyn Error + Send + Sync>> {
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
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    F: FnOnce(libsql::Connection) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn Error + Send + Sync>>>,
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

        conn.execute("INSERT INTO sample_items (name) VALUES (?1)", ["turso test"])
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
}

//! How a capability opens the one shared SQLite file, and when its migration runs.
//!
//! ## One file, table prefixes
//!
//! PRD Q45 (2026-08-27) retired Postgres. Every capability that owned a schema
//! now owns a table prefix in a single SQLite file: `comms.feed_items` became
//! `comms_feed_items`, `tasks.tasks` became `tasks_tasks`. Cross-capability
//! joins survive because it is still one database — that property is the whole
//! reason the shared instance existed, and it is the one that could not be
//! traded away.
//!
//! ## Migrate once per (file, prefix)
//!
//! Kept from the Postgres shape, for a reason that survived the move. Every
//! capability's `Store::open` runs its whole migration: a hundred-odd
//! `CREATE TABLE IF NOT EXISTS` statements in one transaction. `comms` alone
//! opens a store in 43 HTTP handlers and five timers, so under Postgres two
//! openers each held what the other needed and the server killed one session
//! (`digest drain: db error: ERROR: deadlock detected`).
//!
//! SQLite cannot deadlock that way — it has one writer — but the failure it
//! substitutes is worse for a server: every opener that runs DDL takes the
//! write lock, so a migration on each `open` serialises 43 handlers behind each
//! other for no work. Running it once per (file, prefix) removes that: the
//! second and every later `open` in a process take no write lock at all.
//!
//! The cross-process gate is now the database itself. Postgres needed
//! `pg_advisory_lock` because two sessions could interleave DDL; SQLite admits
//! one writer at a time, and [`migrate_once`] takes that writer lock up front
//! with `BEGIN IMMEDIATE` rather than upgrading into it mid-migration. A second
//! process starting at the same moment waits out `busy_timeout` and then finds
//! the tables already there, because every statement is `IF NOT EXISTS`.
//!
//! ## What this deliberately keeps
//!
//! The old design was self-healing by accident: any code path could open a store
//! against an empty database and it worked, which is what every test helper
//! leans on. That property survives here. The first `open` for a given (file,
//! prefix) still migrates, so no caller inherits an assumption that someone else
//! went first. A failed migration is not recorded, so the next `open` retries
//! rather than inheriting a half-built schema.
//!
//! ## The pool
//!
//! [`pool_for`] gives a process one pool per database file. Opening a SQLite
//! connection costs microseconds rather than the 32-39 ms a Postgres
//! `Client::connect` did, so the pool is no longer buying latency — it is
//! buying the PRAGMA discipline below, applied exactly once per connection and
//! never forgettable at a call site, and it keeps `Store::open` the checkout
//! that 134 call sites are written against.
//!
//! ## Every connection, on open
//!
//! ```text
//! journal_mode = WAL     readers do not block the writer
//! busy_timeout = 5000    a writer waits rather than failing at once
//! foreign_keys = ON      SQLite enforces REFERENCES only when asked
//! ```
//!
//! `foreign_keys` is per-connection and OFF by default, so a capability that
//! declares `ON DELETE CASCADE` gets nothing without this line.
//!
//! The connection also gets a 64-statement prepared-statement cache, up from
//! rusqlite's 16. [`QueryAll::query_all`] prepares through it, so a store that
//! reads the same SQL twice on one connection parses it once.
//!
//! ## Dependency rule
//!
//! Its dependency surface stays intentionally narrow: `rusqlite`, `r2d2` and
//! `r2d2_sqlite` are the complete database stack shared by store-owning
//! capabilities. Path resolution is NOT here — `sjel_config::database_path`
//! owns where the file lives, because that is overlay knowledge.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, Row, Transaction, TransactionBehavior};

/// So a capability needs one database dependency, not two that can disagree.
pub use rusqlite;

/// A pooled SQLite connection. Derefs to [`rusqlite::Connection`], so a call
/// site holding one reads as if it held the connection itself.
pub type PooledClient = r2d2::PooledConnection<SqliteConnectionManager>;

/// Shared by every `Store` in a process that talks to the same file.
pub type Pool = r2d2::Pool<SqliteConnectionManager>;

/// The canonical stored timestamp, as a SQL expression.
///
/// Renders `2026-08-27 21:23:35.871+00:00`: UTC, millisecond resolution,
/// ISO-8601 with a space separator. Interpolate it where Postgres had `now()`:
///
/// ```text
/// format!("UPDATE {p}_tasks SET updated_at = {now}", p = self.prefix, now = sjel_store::NOW)
/// ```
///
/// Two properties decided the shape, and both are load-bearing:
///
/// - It sorts. Fixed width to the millisecond means a plain `ORDER BY` on the
///   text column is chronological, which is what every existing index assumes.
/// - SQLite can read it back. Postgres rendered `+00` for the offset and
///   `datetime('2026-08-27 21:23:35.871000+00')` returns NULL — SQLite wants
///   `[+-]HH:MM`. Writing `+00:00` keeps `datetime()`, `julianday()` and
///   `strftime()` usable on a stored column, which `+00` would have cost.
///
/// A SQL expression rather than a registered `now()` function on purpose: an
/// operator with the `sqlite3` CLI open must be able to write the same value
/// this code writes. A custom function would exist only inside this process.
pub const NOW: &str = "strftime('%Y-%m-%d %H:%M:%f+00:00','now')";

/// The canonical stamp's `strftime` format, written down once.
///
/// [`NOW`] spells it out rather than building from this, because a `const` cannot
/// call `format!`; `the_two_now_forms_agree` pins the two together.
///
/// Public for the one case neither [`NOW`] nor [`now_offset`] covers: stamping a
/// time that is not derived from the clock. comms writes Gmail's `internalDate`
/// this way — `strftime('{STAMP_FORMAT}', ?5, 'unixepoch')` — where Postgres had
/// `to_timestamp($5)`. Anything relative to now belongs in [`now_offset`].
pub const STAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%f+00:00";

/// [`NOW`] shifted by a SQLite date modifier, still in the canonical format.
///
/// For a deadline stored in a timestamp column — a retry time, a purge date —
/// where Postgres wrote `now() + interval '30 days'`. `datetime('now','+30 days')`
/// would be the obvious translation and is the wrong one: it renders 19 characters
/// where the column holds 29, so one column would carry two widths and a plain
/// `ORDER BY` on it would stop being time order.
///
/// ```text
/// now_offset("'+30 days'")                              // a literal deadline
/// now_offset("'+' || MIN(attempts + 1, 5) || ' minutes'") // a computed backoff
/// ```
///
/// `modifier` is interpolated as a SQL expression, not bound, because a date
/// modifier can be computed from the row being written. Every caller passes a
/// literal built in this repo; it is not a place to put anything a user typed.
pub fn now_offset(modifier: &str) -> String {
    format!("strftime('{STAMP_FORMAT}','now',{modifier})")
}

/// Beyond this, a checkout gives up rather than hanging.
///
/// r2d2's default is 30 seconds. That is a sane default for a batch job and the
/// wrong one for an HTTP handler: 30 seconds is long enough that the dashboard
/// looks hung rather than broken, and long enough for the requests behind it to
/// pile up.
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(5);

/// r2d2's own default, named rather than inherited so it is visible next to the
/// setting that is NOT a default.
const MAX_CONNECTIONS: u32 = 10;

/// Applied to every connection the pool hands out.
///
/// `busy_timeout` defaults to 0 in SQLite — the second writer fails at once. At
/// the measured write rate (0.30 commits/s, PRD Q45) five seconds is past any
/// real contention and still short enough to read as a failure, not a hang.
/// The other two are explained in the module doc.
const CONNECTION_PRAGMAS: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA busy_timeout = 5000;
    PRAGMA foreign_keys = ON;
";

/// How many prepared statements a connection keeps for [`QueryAll::query_all`].
///
/// rusqlite's default is 16 (`STATEMENT_CACHE_DEFAULT_CAPACITY`, rusqlite
/// 0.40.2 src/lib.rs:168), and it is an LRU: the 17th distinct statement evicts
/// the least recently used one, which then has to be parsed again next time.
/// comms has 18 `query_all` call sites and places 16, both against one shared
/// file, so the default would start evicting exactly in the two crates with the
/// most reads. 64 holds every `query_all` site in the workspace at once and
/// costs one finalized statement handle each.
const STATEMENT_CACHE_SIZE: usize = 64;

fn pools() -> &'static Mutex<HashMap<PathBuf, Pool>> {
    static POOLS: OnceLock<Mutex<HashMap<PathBuf, Pool>>> = OnceLock::new();
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One absolute spelling for a file, so two callers naming it differently share
/// a pool and a migration target instead of quietly getting two of each.
///
/// The parent is canonicalized rather than the file: the file may not exist yet
/// (SQLite creates it on first open) and `canonicalize` refuses a missing path.
/// Creating the directory here is deliberate — a library must not mkdir at
/// import time, but `open` is the moment the caller asked for the file.
fn canonical_key(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
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

/// The pool for `path` in this process, built on first ask.
///
/// `min_idle(0)` is the setting that matters and is not a default. r2d2
/// otherwise keeps `max_size` connections warm, which is right for a server and
/// actively wrong for the CLI half of these crates: `comms sweep` would open ten
/// connections to run one query and close nine of them on exit.
pub fn pool_for(path: &Path) -> Result<Pool, Box<dyn Error>> {
    pool_for_key(&canonical_key(path)?)
}

/// `key` comes from [`canonical_key`], so callers that already hold one do not
/// canonicalize twice — and the pool map cannot end up with two spellings.
fn pool_for_key(key: &Path) -> Result<Pool, Box<dyn Error>> {
    let mut pools = pools()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(existing) = pools.get(key) {
        return Ok(existing.clone());
    }

    let manager = SqliteConnectionManager::file(key).with_init(|conn: &mut Connection| {
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_SIZE);
        conn.execute_batch(CONNECTION_PRAGMAS)
    });
    let pool = r2d2::Pool::builder()
        .max_size(MAX_CONNECTIONS)
        .min_idle(Some(0))
        .connection_timeout(CHECKOUT_TIMEOUT)
        .build(manager)?;

    pools.insert(key.to_path_buf(), pool.clone());
    Ok(pool)
}

/// Get a pool for `path` and make sure `prefix`'s tables are migrated.
///
/// The checkout here is not only a probe, though it serves as one. `min_idle(0)`
/// means `build` opens nothing, so without it an unwritable path would be
/// reported by the first query rather than by `open` — the kind of regression
/// that turns "the database is not there" into a confusing error deep in a
/// handler.
pub fn open_pool(
    path: &Path,
    prefix: &str,
    ddl: impl FnOnce(&Connection) -> Result<(), Box<dyn Error>>,
) -> Result<Pool, Box<dyn Error>> {
    let key = canonical_key(path)?;
    let pool = pool_for_key(&key)?;
    let mut conn = pool.get()?;
    migrate_once(&mut conn, &key, prefix, ddl)?;
    drop(conn);
    Ok(pool)
}

fn migrated() -> &'static Mutex<HashSet<String>> {
    static MIGRATED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    MIGRATED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Runs `work` the first time this process sees a given (database, prefix), and
/// never again for that pair.
///
/// Split out from [`migrate_once`] so the once-guard is testable without a
/// database: everything subtle here is about the key and the retry, and none of
/// it needs SQLite to be wrong.
///
/// `work` running to an error is not recorded. A half-built schema that reports
/// itself as migrated is the one outcome worse than migrating twice.
pub fn once_per_target<E>(
    database: &str,
    prefix: &str,
    work: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    // A unit separator, because a path and a prefix are both arbitrary text:
    // concatenated without one, ("a", "bc") and ("ab", "c") are the same key.
    let key = format!("{database}\u{1f}{prefix}");

    // Held across `work`, not merely across the lookup. Releasing between the
    // check and the migration lets two threads both miss and both migrate,
    // which is the precise race this exists to close.
    //
    // Poisoning is recovered rather than propagated. The key is inserted only on
    // success, so a panicking migration leaves the set correct, and refusing to
    // migrate for the remaining life of the process is a worse failure than the
    // one that poisoned it.
    let mut done = migrated()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if done.contains(&key) {
        return Ok(());
    }

    work()?;
    done.insert(key);
    Ok(())
}

/// Migrates `prefix`'s tables if this process has not already, inside one
/// immediate transaction.
///
/// `BEGIN IMMEDIATE`, not the default deferred begin. A deferred transaction
/// that reads first and then writes has to upgrade to the writer lock, and
/// SQLite answers a failed upgrade with `SQLITE_BUSY` immediately — `busy_timeout`
/// does not retry that case, because another writer may have already changed
/// what this one read. Taking the lock up front is what makes two processes
/// starting together wait for each other instead of one of them erroring out.
///
/// The DDL itself stays in the capability — it is the one part of this that is
/// genuinely per-capability. What is shared is when it runs and what stops two
/// processes running it at once.
pub fn migrate_once(
    conn: &mut Connection,
    path: &Path,
    prefix: &str,
    ddl: impl FnOnce(&Connection) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let key = path.to_string_lossy().into_owned();
    once_per_target(&key, prefix, || {
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ddl(&transaction)?;
        // Rolled back on the error path by `Transaction`'s own Drop, so a failed
        // migration leaves no half-built tables for the retry to trip over.
        transaction.commit()?;
        Ok(())
    })
}

/// Begins a transaction that is going to write, with `BEGIN IMMEDIATE`.
///
/// [`migrate_once`] has taken the writer lock up front since it was written, for
/// the reason stated above it: a deferred transaction that reads before it
/// writes must upgrade to the writer lock, and SQLite answers a failed upgrade
/// with `SQLITE_BUSY` **immediately**. `busy_timeout` does not retry that case,
/// because another writer may already have changed what this one read.
///
/// That reasoning was never migration-specific, and the rest of the workspace
/// did not get it. `capabilities/comms/src/store/feed.rs` set a feed item's
/// status inside a deferred transaction whose first statement is a `SELECT` —
/// deliberately, so a second press of `u` does not append a decision that never
/// happened — and every one of those is an upgrade. Eleven services share one
/// file here, so the drain that runs every fifteen minutes is enough to lose
/// the race: `demo-seed: POST /feed/<id>/status → 400: {"error":"database is
/// locked"}`, which is what has been failing the Pages build.
///
/// So it moves here, next to the PRAGMAs, on the same argument the module doc
/// already makes for those: applied at one place and **never forgettable at a
/// call site**. `tools/check-store-transactions.sh` keeps `.transaction()` out
/// of the capabilities so the choice cannot be made again by omission.
///
/// The cost is real and small. `BEGIN IMMEDIATE` takes the writer lock before
/// the first read rather than at the first write, so two writers serialise for
/// slightly longer. They serialise either way — SQLite admits one writer — and
/// waiting out `busy_timeout` is what this buys instead of erroring at once.
///
/// A transaction that only reads does not want this and should not use it.
pub fn write_transaction(conn: &mut Connection) -> Result<Transaction<'_>, rusqlite::Error> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

/// A TEXT column holding JSON, read into its type.
///
/// The one shape SQLite has no type for. Postgres carried it three ways and all
/// three land here: a `jsonb` column (`places.geocode_cache.response`), an
/// `integer[]` (`punctuality.stop_stats.counts`), and the TEXT columns
/// capabilities were already serializing by hand (`trips.plans.stages`).
///
/// The error path is why this exists rather than a `from_str` at each site: a
/// row whose JSON does not parse must fail as a column conversion, naming the
/// column index, instead of becoming a bare serde error with no idea where it
/// came from.
pub fn json_column<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

/// Every row of a query, mapped.
///
/// rusqlite already answers the one-row cases: `query_row` is `query_one`, and
/// `query_row(..).optional()` (via [`rusqlite::OptionalExtension`]) is
/// `query_opt`. Only the many-row case has no one-call form — it is
/// prepare, `query_map`, collect — and repeating those three lines at every
/// list-shaped read is where a `?` gets dropped. This is that one method and
/// nothing else, so the vocabulary stays rusqlite's.
pub trait QueryAll {
    fn query_all<T, P, F>(&self, sql: &str, params: P, map: F) -> rusqlite::Result<Vec<T>>
    where
        P: rusqlite::Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>;
}

impl QueryAll for Connection {
    /// `prepare_cached`, not `prepare`: the SQL text is the cache key, and every
    /// store here builds its statement with `format!("… {prefix} …")`, so the
    /// same call site produces a byte-identical string every time and SQLite
    /// stops re-parsing it. The cache lives on the connection, which is what
    /// makes it safe — the pool hands a connection to one caller at a time, so
    /// a cached statement is never shared across threads.
    ///
    /// Measured rather than assumed (2026-09-05, release build, one connection,
    /// a two-parameter ten-row read repeated 20,000 times): 4.15 µs per read
    /// through `prepare`, 2.50 µs through `prepare_cached`. The saving is the
    /// SQL parse, so it is per read and does not grow with the row count — which
    /// is why the loop-shaped readers gain most.
    ///
    /// Two properties that would otherwise need checking: a cached statement is
    /// removed from the cache while it is in use, so a `map` closure that runs
    /// the same SQL again gets its own statement rather than the one it is
    /// iterating; and SQLite re-prepares a cached statement itself when the
    /// schema changes under it, so a migration cannot leave a stale plan behind
    /// (`a_cached_statement_survives_a_schema_change` pins both).
    fn query_all<T, P, F>(&self, sql: &str, params: P, map: F) -> rusqlite::Result<Vec<T>>
    where
        P: rusqlite::Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        let mut statement = self.prepare_cached(sql)?;
        let rows = statement.query_map(params, map)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        now_offset, once_per_target, open_pool, write_transaction, Connection, QueryAll,
        CONNECTION_PRAGMAS, NOW, STAMP_FORMAT,
    };
    use std::cell::Cell;

    /// Distinct per test: the guard is process-global by design, so two tests
    /// sharing a (path, prefix) would see each other's result and pass for the
    /// wrong reason.
    fn database(tag: &str) -> String {
        format!("/tmp/sjel-store-test/{tag}.db")
    }

    /// A throwaway file per test. A temp file is the whole test fixture now —
    /// no server, no per-pid schema, no cleanup that a panic can skip, because
    /// the directory goes with the process's temp dir.
    fn temp_database(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{tag}.db"))
    }

    #[test]
    fn runs_the_first_time_and_not_again() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&database("repeat"), "s", bump).unwrap();
        once_per_target(&database("repeat"), "s", bump).unwrap();
        once_per_target(&database("repeat"), "s", bump).unwrap();

        assert_eq!(runs.get(), 1, "only the first call should do the work");
    }

    #[test]
    fn a_failure_is_retried_rather_than_recorded() {
        let runs = Cell::new(0);

        let first = once_per_target(&database("retry"), "s", || {
            runs.set(runs.get() + 1);
            Err::<(), &str>("migration blew up")
        });
        assert_eq!(first, Err("migration blew up"));

        once_per_target(&database("retry"), "s", || {
            runs.set(runs.get() + 1);
            Ok::<(), &str>(())
        })
        .unwrap();

        assert_eq!(
            runs.get(),
            2,
            "a failed migration must not mark the target as done"
        );
    }

    #[test]
    fn prefixes_are_tracked_separately() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&database("prefixes"), "one", bump).unwrap();
        once_per_target(&database("prefixes"), "two", bump).unwrap();
        once_per_target(&database("prefixes"), "one", bump).unwrap();

        assert_eq!(runs.get(), 2, "each prefix migrates on its own");
    }

    #[test]
    fn databases_are_tracked_separately() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target(&database("db-a"), "same", bump).unwrap();
        once_per_target(&database("db-b"), "same", bump).unwrap();

        assert_eq!(
            runs.get(),
            2,
            "the same prefix in two databases is two targets"
        );
    }

    /// The separator matters: without it ("…/ab", "c") and ("…/a", "bc")
    /// collide, and the second prefix silently never migrates.
    #[test]
    fn a_shared_prefix_does_not_collide() {
        let runs = Cell::new(0);
        let bump = || {
            runs.set(runs.get() + 1);
            Ok::<(), ()>(())
        };

        once_per_target("/tmp/collideab", "c", bump).unwrap();
        once_per_target("/tmp/collidea", "bc", bump).unwrap();

        assert_eq!(runs.get(), 2, "the key must not be ambiguous at the join");
    }

    /// The three settings the module doc promises, read back from a real
    /// connection. `foreign_keys` is the one that fails silently when missed:
    /// SQLite parses `REFERENCES` either way and simply does not enforce it.
    #[test]
    fn every_connection_gets_the_pragmas() {
        let path = temp_database("pragmas");
        let pool = open_pool(&path, "pragma_probe", |_| Ok(())).unwrap();
        let conn = pool.get().unwrap();

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "REFERENCES is unenforced without this");
        let busy: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5_000);
    }

    /// The parent directory is created by `open`, not by importing this crate.
    /// The overlay's `data/axon/` does not exist on a fresh machine.
    #[test]
    fn open_creates_the_directory_the_file_lives_in() {
        let path = temp_database("nested").parent().unwrap().join("a/b/c.db");
        assert!(!path.parent().unwrap().exists());
        open_pool(&path, "nested_probe", |conn| {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS nested_probe_t (id TEXT)")?;
            Ok(())
        })
        .unwrap();
        assert!(path.exists(), "the file should be where it was asked for");
    }

    /// The format contract [`NOW`] states, checked against SQLite rather than
    /// against a comment: fixed width so `ORDER BY` is chronological, and an
    /// offset SQLite's own date functions can read back.
    #[test]
    fn the_canonical_timestamp_sorts_and_parses() {
        let path = temp_database("timestamp");
        let pool = open_pool(&path, "ts_probe", |_| Ok(())).unwrap();
        let conn = pool.get().unwrap();

        let stamp: String = conn
            .query_row(&format!("SELECT {NOW}"), [], |row| row.get(0))
            .unwrap();
        assert_eq!(stamp.len(), 29, "fixed width or ORDER BY is not time order");
        assert!(stamp.ends_with("+00:00"), "got {stamp}");

        // The `+00` Postgres rendered is what this exists not to write: SQLite
        // reads it as NULL and every date function on the column goes quiet.
        let parsed: Option<String> = conn
            .query_row("SELECT datetime(?1)", [&stamp], |row| row.get(0))
            .unwrap();
        assert!(parsed.is_some(), "SQLite must be able to read {stamp} back");

        // Text order is time order, which is what the indexes assume.
        let ordered = conn
            .query_all(
                "SELECT column1 FROM (VALUES ('2026-08-27 09:00:00.000+00:00'),
                                             ('2026-08-27 10:00:00.000+00:00'),
                                             ('2026-08-27 09:59:59.999+00:00'))
                 ORDER BY column1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(ordered[2], "2026-08-27 10:00:00.000+00:00");
    }

    /// [`now_offset`] must render the same shape [`NOW`] does, or a column would
    /// carry two widths and `ORDER BY` on it would stop being time order.
    #[test]
    fn the_two_now_forms_agree() {
        let path = temp_database("now_offset");
        let pool = open_pool(&path, "offset_probe", |_| Ok(())).unwrap();
        let conn = pool.get().unwrap();

        assert!(
            NOW.contains(STAMP_FORMAT),
            "NOW must spell out STAMP_FORMAT"
        );

        let (plain, shifted): (String, String) = conn
            .query_row(
                &format!("SELECT {NOW}, {}", now_offset("'+30 days'")),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(plain.len(), shifted.len(), "{plain} vs {shifted}");
        assert!(shifted.ends_with("+00:00"));
        assert!(shifted > plain, "+30 days must sort after now");

        // A computed modifier is the other caller shape: comms builds a backoff
        // from a column on the row it is writing.
        let computed: String = conn
            .query_row(
                &format!(
                    "SELECT {}",
                    now_offset("'+' || MIN(3 + 1, 5) || ' minutes'")
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(computed.len(), 29, "got {computed}");
    }

    /// `query_all` prepares through the connection's statement cache, so the
    /// second run of one SQL string reuses the statement the first left behind.
    /// What is under test is that reuse changes nothing a caller can see: the
    /// same rows in the same order, and parameters bound fresh each time rather
    /// than inherited from the previous run.
    #[test]
    fn the_same_sql_twice_returns_the_same_rows() {
        let path = temp_database("cached-rows");
        let pool = open_pool(&path, "cache_probe", |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS cache_probe_t (
                     id   TEXT PRIMARY KEY,
                     kind TEXT NOT NULL
                 )",
            )?;
            Ok(())
        })
        .unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "INSERT INTO cache_probe_t (id, kind)
             VALUES ('a','one'),('b','two'),('c','one')",
        )
        .unwrap();

        let sql = "SELECT id FROM cache_probe_t WHERE kind = ?1 ORDER BY id";
        let ones = || {
            conn.query_all(sql, ["one"], |row| row.get::<_, String>(0))
                .unwrap()
        };

        assert_eq!(ones(), vec!["a".to_string(), "c".to_string()]);
        assert_eq!(ones(), ones(), "the cached statement must not drift");

        // A different parameter through the same cached statement, then the
        // first one again. A binding left over from the previous run is the one
        // way a reused statement could answer the wrong question.
        let twos = conn
            .query_all(sql, ["two"], |row| row.get::<_, String>(0))
            .unwrap();
        assert_eq!(twos, vec!["b".to_string()]);
        assert_eq!(ones(), vec!["a".to_string(), "c".to_string()]);
    }

    /// A cached statement holds a compiled plan, and the schema it was compiled
    /// against can change under it — `migrate_once` runs DDL on a pooled
    /// connection that may already have read through this cache. SQLite
    /// re-prepares on the next step rather than answering from the stale plan,
    /// and this is the test that says so out loud.
    ///
    /// The nested read in the middle covers the other property the cache has to
    /// have: `prepare_cached` takes the statement out of the cache while it is
    /// in use, so a `map` closure running the same SQL cannot end up stepping
    /// the statement it is already iterating.
    #[test]
    fn a_cached_statement_survives_a_schema_change() {
        let path = temp_database("cached-schema");
        let pool = open_pool(&path, "cache_alter", |conn| {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS cache_alter_t (id TEXT PRIMARY KEY)")?;
            Ok(())
        })
        .unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch("INSERT INTO cache_alter_t (id) VALUES ('a'),('b'),('c')")
            .unwrap();

        let sql = "SELECT id FROM cache_alter_t ORDER BY id";
        let before = conn
            .query_all(sql, [], |row| row.get::<_, String>(0))
            .unwrap();
        assert_eq!(before, vec!["a", "b", "c"]);

        let reference: &super::Connection = &conn;
        let nested = reference
            .query_all(sql, [], |row| {
                let inner = reference.query_all(sql, [], |row| row.get::<_, String>(0))?;
                assert_eq!(inner, before, "the nested read got the outer statement");
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(nested, before);

        conn.execute_batch("ALTER TABLE cache_alter_t ADD COLUMN note TEXT")
            .unwrap();
        conn.execute_batch("INSERT INTO cache_alter_t (id, note) VALUES ('d','added')")
            .unwrap();

        let after = conn
            .query_all(sql, [], |row| row.get::<_, String>(0))
            .unwrap();
        assert_eq!(after, vec!["a", "b", "c", "d"], "stale plan after the DDL");
    }

    /// A path whose parent is a file, not a directory. `open` must say so
    /// rather than hand back a pool whose first query fails somewhere else.
    #[test]
    fn an_unusable_path_fails_at_open() {
        let blocker = temp_database("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let result = open_pool(&blocker.join("axon.db"), "blocked", |_| Ok(()));
        assert!(result.is_err(), "an unusable path opened anyway");
    }

    /// The deferred read-then-write upgrade, reproduced.
    ///
    /// This is the failure that reached the operator as
    /// `POST /feed/<id>/status → 400: {"error":"database is locked"}`. Asserted
    /// rather than described, because the whole reason it survived 29 call
    /// sites is that nobody had watched it happen: the module doc named it, the
    /// migration guarded against it, and every other writer inherited the
    /// deferred default.
    ///
    /// `busy_timeout` is set to five seconds on both connections here and does
    /// not save the deferred reader. That is the point — a failed upgrade is
    /// not a timeout, so SQLite refuses at once rather than waiting.
    #[test]
    fn a_deferred_read_then_write_loses_the_upgrade_and_immediate_does_not() {
        use rusqlite::TransactionBehavior;

        let path = temp_database("upgrade");
        let _ = std::fs::remove_file(&path);
        let setup = Connection::open(&path).unwrap();
        setup.execute_batch(CONNECTION_PRAGMAS).unwrap();
        setup
            .execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER);
                            INSERT INTO t VALUES (1, 0);",
            )
            .unwrap();

        // The other writer. It holds the write lock for the whole of each case
        // below, which is what a fifteen-minute drain does to a keypress.
        let mut holder = Connection::open(&path).unwrap();
        holder.execute_batch(CONNECTION_PRAGMAS).unwrap();

        let mut victim = Connection::open(&path).unwrap();
        victim.execute_batch(CONNECTION_PRAGMAS).unwrap();

        // Deferred: reads first, so it holds only a read lock, and the UPDATE
        // has to upgrade into a writer lock somebody else already holds.
        let held = holder
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        held.execute("UPDATE t SET v = 1 WHERE id = 1", []).unwrap();

        let deferred = victim.transaction().unwrap();
        let _: i64 = deferred
            .query_row("SELECT v FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        let upgrade = deferred.execute("UPDATE t SET v = 2 WHERE id = 1", []);
        let error = upgrade.expect_err(
            "the deferred upgrade succeeded — this test no longer reproduces the bug it guards",
        );
        // Asserted on the code, not on the message: the whole claim is that
        // SQLite refuses with SQLITE_BUSY rather than waiting out busy_timeout,
        // and a substring match on a debug string would also pass for a
        // different error that happens to contain the word.
        match error {
            rusqlite::Error::SqliteFailure(e, _) => assert_eq!(
                e.code,
                rusqlite::ErrorCode::DatabaseBusy,
                "the upgrade failed for some reason other than a busy database"
            ),
            other => panic!("expected a SQLite failure, got {other:?}"),
        }
        drop(deferred);
        held.commit().unwrap();

        // Immediate: takes the writer lock before the read, so there is no
        // upgrade to lose. It waits out busy_timeout instead, which is the
        // trade `write_transaction` documents.
        let now = write_transaction(&mut victim).unwrap();
        let seen: i64 = now
            .query_row("SELECT v FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(seen, 1, "immediate transaction read the committed value");
        now.execute("UPDATE t SET v = 3 WHERE id = 1", [])
            .expect("an immediate transaction writes without an upgrade");
        now.commit().unwrap();

        let final_value: i64 = setup
            .query_row("SELECT v FROM t WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(final_value, 3);
    }
}

# libs/sjel-store

One home for **how a capability opens the shared database, and when its migration runs**.

A shared library, not a capability: no domain of its own, no CLI
(README.md#three-architectural-nouns). Consumers declare an `sjel-store` path dependency in the
workspace.

## One file, table prefixes

PRD Q45 (2026-08-27) retired Postgres. Every capability that owned a schema now owns a **table
prefix** in a single SQLite file:

| Postgres | SQLite |
|---|---|
| schema `comms`, table `feed_items` | table `comms_feed_items` |
| schema `tasks`, table `tasks` | table `tasks_tasks` |

Cross-capability joins survive because it is still one database. That property is why the shared
instance existed at all, and it is the one that could not be traded away.

Where the file lives is not this crate's decision — `sjel_config::database_path` owns that
(`SJEL_DB_PATH`, else `<overlay>/data/axon/axon.db`), because it is overlay knowledge. Nor is
how it is kept: `capabilities/store` declares that, in the one manifest that owns the file
(`kind = "data"`, `backup_sqlite_online`). A capability opens the database; it does not own it.

## The canonical timestamp

```
2026-08-27 21:23:35.871+00:00
```

UTC, millisecond resolution, ISO-8601 with a space separator. `sjel_store::NOW` is the SQL
expression that renders it; interpolate it where Postgres had `now()`:

```rust
format!("UPDATE {p}_tasks SET updated_at = {now}", p = self.prefix, now = sjel_store::NOW)
```

Two properties decided the shape, and both are load-bearing.

**It sorts.** Fixed width to the millisecond means a plain `ORDER BY` on the text column is
chronological, which is what every existing index already assumed under Postgres.

**SQLite can read it back.** Postgres rendered the offset as `+00`, and
`datetime('2026-08-27 21:23:35.871000+00')` returns NULL — SQLite wants `[+-]HH:MM`. Writing
`+00:00` keeps `datetime()`, `julianday()` and `strftime()` usable on a stored column. Adopting
Postgres's exact rendering would have cost that, silently: the date functions do not fail, they
return NULL.

A SQL expression rather than a `now()` function registered on the connection, on purpose. An
operator with the `sqlite3` CLI open must be able to write the same value this code writes; a
custom function would exist only inside this process.

**A deadline uses `now_offset`, not `datetime`.** Postgres wrote `now() + interval '30 days'`
for a purge date and `now() + interval '1 minute' * n` for a retry backoff.
`datetime('now','+30 days')` is the obvious translation and the wrong one: it renders 19
characters into a column holding 29, so one column would carry two widths and `ORDER BY` on it
would stop being time order. `sjel_store::now_offset` applies the same format to a shifted
clock, and takes a SQL *expression* because a backoff is computed from the row being written:

```rust
now_offset("'+30 days'")                                  // a literal deadline
now_offset("'+' || MIN(attempts + 1, 5) || ' minutes'")   // a computed one
```

`sjel_store::STAMP_FORMAT` is the last of the three, for the one case neither covers: a stamp
that is not derived from the clock. comms writes Gmail's `internalDate` with
`strftime('{STAMP_FORMAT}', ?5, 'unixepoch')`, where Postgres had `to_timestamp($5)`.

## Every connection, on open

```
journal_mode = WAL     readers do not block the writer
busy_timeout = 5000    a writer waits rather than failing at once
foreign_keys = ON      SQLite enforces REFERENCES only when asked
```

`foreign_keys` is per-connection and OFF by default, so a capability that declares
`ON DELETE CASCADE` gets nothing without it. `busy_timeout` defaults to 0 — the second writer
fails immediately. At the measured write rate (0.30 commits/s, PRD Q45) five seconds is past any
real contention and short enough to still read as a failure.

The pool applies all three through `with_init`, so they cannot be forgotten at a call site.

## Migrate once per (file, prefix)

Kept from the Postgres shape, for a reason that survived the move.

Every capability's `Store::open` runs its whole migration: a hundred-odd
`CREATE TABLE IF NOT EXISTS` statements in one transaction. `comms` alone opens a store in 43 HTTP
handlers and five timers. Under Postgres, two openers each held what the other needed and the
server killed one session:

```
digest drain: db error: ERROR: deadlock detected
```

SQLite cannot deadlock that way — it admits one writer — but the failure it substitutes is worse
for a server: every opener that runs DDL takes the write lock, so a migration on each `open`
serialises 43 handlers behind each other for no work at all. `migrate_once` runs the migration the
first time a process sees a given (file, prefix) and never again for that pair, so the second and
every later `open` take no write lock.

**The cross-process gate is now the database itself.** Postgres needed `pg_advisory_lock` because
two sessions could interleave DDL. SQLite admits one writer, and `migrate_once` takes that writer
lock up front with `BEGIN IMMEDIATE` rather than upgrading into it mid-migration — a deferred
transaction that reads and then writes answers a failed upgrade with `SQLITE_BUSY` at once, and
`busy_timeout` deliberately does not retry that case. A second process starting at the same moment
waits out the timeout and then finds the tables already there, because every statement is
`IF NOT EXISTS`.

## What this deliberately did not change

The old shape was self-healing by accident. Any code path could open a store against an empty
database and it worked, and every test helper leans on exactly that. So the first `open` still
migrates. A failed migration is not recorded as done, so the next `open` retries instead of
handing out a half-built schema.

## Writing a capability's DDL

The file starts empty, so a capability's migration states the **current** shape of its tables
rather than replaying the history that produced it. Postgres migrations that widened a `CHECK` or
added a column with `ALTER TABLE … ADD COLUMN IF NOT EXISTS` are folded into the `CREATE TABLE`.
SQLite has no `ADD COLUMN IF NOT EXISTS` and cannot alter a constraint at all, so replaying that
history was never an option; folding is the translation, and it is only correct because no
deployed SQLite file predates it.

The translation table, settled while porting tasks, transit and trips:

| Postgres | SQLite |
|---|---|
| `CREATE SCHEMA x; x.t` | `x_t` |
| `$1`, `$2` | `?1`, `?2` |
| `now()` | `{now}` from `sjel_store::NOW` |
| `to_char(now(), 'YYYY-MM-DD')` | `date('now')` |
| `now() - interval '7 days'` | `datetime('now','-7 days')` |
| `TIMESTAMPTZ` | `TEXT` |
| `DOUBLE PRECISION` | `REAL` |
| `BIGINT`, `BOOLEAN` | `INTEGER` |
| `col::TEXT` on a TEXT column | drop the cast |
| `$1::text IS NULL OR c = $1` | `?1 IS NULL OR c = ?1` |
| `LIMIT $2` with a NULL bound | `LIMIT COALESCE(?2, -1)` |
| `now() + interval '30 days'` | `sjel_store::now_offset("'+30 days'")` |
| `to_timestamp($5)` (epoch seconds) | `strftime('{STAMP_FORMAT}', ?5, 'unixepoch')` |
| `col = ANY($3)` / `col <> ALL($3)` | `col IN (SELECT value FROM json_each(?3))`, list bound as JSON |
| `LEAST(a, b)` / `GREATEST(a, b)` | `MIN(a, b)` / `MAX(a, b)` (two-argument scalar form) |
| `power(2, n)` | `1 << n` |
| `a IS DISTINCT FROM b` | `a IS NOT b` |
| `left(x, 10)` | `substr(x, 1, 10)` |
| `CURRENT_DATE - $1` | `date('now', '-' || ?1 || ' days')` |
| `EXTRACT(HOUR FROM now())` | `CAST(strftime('%H','now') AS INTEGER)` |
| `EXTRACT(EPOCH FROM (a - b))` | `(julianday(a) - julianday(b)) * 86400.0` |
| `(ts AT TIME ZONE 'UTC')::date` | `substr(ts, 1, 10)` — the stamp is already UTC text |
| `lpad($1, 8, '0')` | a second bound parameter, padded in Rust |
| `information_schema.columns` | `pragma_table_info(?1)` |
| `ILIKE` | `LIKE` (see the narrowing below) |
| `SELECT ... FOR UPDATE` | nothing — SQLite has one writer, the transaction is the lock |
| `pg_advisory_xact_lock(...)` | `BEGIN IMMEDIATE` on the connection doing the work |
| `INSERT INTO t AS a … a.col` | `INSERT INTO t … t.col` — SQLite has no INSERT alias |

That last row is not cosmetic. Postgres reads `LIMIT NULL` as `LIMIT ALL`; SQLite raises
`datatype mismatch` and the read fails outright. A negative limit is SQLite's "no upper bound".

### Four that are not translations

**`ILIKE` narrows.** SQLite's `LIKE` already ignores case, but only over ASCII, so `MÜNCHEN`
stops matching `München` where `münchen` still matches. ICU for one letter is not worth a
build-time dependency; it is stated at each call site instead.

**`RETURNING (xmax = 0)`.** Postgres answering "was that an insert or an update" for the row an
upsert just touched. SQLite exposes no such system column and both one-statement substitutes are
wrong: `changes()` reports 1 for either branch, and comparing a stored timestamp to `now()`
calls the second of two writes inside one second new. It becomes insert-or-nothing followed by a
conditional update, in one transaction (`capabilities/scouting/src/store.rs::propose_source`).

**A data-modifying CTE.** `WITH x AS (INSERT … RETURNING …) SELECT …` has no SQLite form at all:
INSERT is not allowed inside a WITH clause. It becomes the read and the write as two statements
in one transaction, which is what the CTE was buying — `capabilities/comms/src/store/cloud.rs`
queues a cloud job that way, still refusing a hash no review approved.

**A constraint name in an error.** Postgres named a CHECK and quoted the name back; SQLite
quotes the CHECK *expression*. A test asserting on the refusal has to match what the database
says now (`content_cloud_derivatives_original_data_class_check` became
`original_data_class IN ('c0','c1')`), which is the same claim about the same
constraint.

Comparing a stored timestamp against `datetime('now', …)` is a text comparison between a
29-character stamp and a 19-character one. It is still correct to the second, because the shorter
string is a prefix of the longer: `'…12:00:00'` sorts before `'…12:00:00.000+00:00'`.

## Reading rows

The vocabulary is rusqlite's, with one addition. `query_row` is the one-row read;
`query_row(…).optional()` (via `rusqlite::OptionalExtension`) is the might-not-exist read. Only
the many-row read has no one-call form in rusqlite — prepare, `query_map`, collect — so
`sjel_store::QueryAll::query_all` is that one method, and there are no others. Inventing a second
name for something rusqlite already has would be the expensive mistake.

`query_all` prepares through the connection's statement cache (`prepare_cached`), keyed on the SQL
text. Every store builds its statement with `format!("… {prefix} …")`, so one call site produces a
byte-identical string every time and SQLite parses it once per connection instead of once per read.
The cache belongs to the connection, and the pool hands a connection to one caller at a time, so a
cached statement is never shared. Its capacity is raised from rusqlite's default of 16 to 64,
because comms alone has 18 `query_all` sites and would otherwise evict its own statements.

```rust
use sjel_store::QueryAll;
use rusqlite::OptionalExtension;

let task  = conn.query_row(&sql, [&id], row_to_task).optional()?;   // 0 or 1
let tasks = conn.query_all(&sql, [&status], row_to_task)?;          // n
conn.execute(&sql, params![&id, &title])?;                          // write
conn.execute_batch(&ddl)?;                                          // migration
```

`sjel_store::json_column(row, index)` is the second addition, and the last. SQLite has no JSON
type, so everything Postgres held as `jsonb` (`places.geocode_cache.response`) or `integer[]`
(`punctuality.stop_stats.counts`) is TEXT here, beside the TEXT columns capabilities were already
serializing by hand. It fails as a column conversion naming the index, where a bare `from_str` at
the call site produces a serde error with no idea which column it came from.

## The pool

`pool_for` gives a process one pool per database file, keyed by the canonicalized path so two
spellings of one file are one pool and one migration target.

A SQLite connection costs microseconds, where a Postgres `Client::connect` cost 32-39 ms measured,
so the pool is no longer buying latency. It buys the PRAGMA discipline above — applied once per
connection, never forgettable at a call site — plus the statement-cache capacity, and it keeps
`Store::open` the checkout that the call sites are written against. A connection that lives in the
pool carries its parsed statements with it; one that is rebuilt per call would not.

Two settings are deliberate rather than inherited. `min_idle(0)`, because r2d2 otherwise keeps
`max_size` connections warm: right for a server, wrong for the CLI half of these crates, where
`comms sweep` would open ten connections to run one query. And a five-second checkout timeout
instead of r2d2's thirty, because thirty seconds reads as a hang rather than a failure and lets
requests pile up behind it.

## Testing a store

A store test needs a temp file and nothing else — no server, no per-pid schema, no cleanup a panic
can skip. The module that holds those tests is named `db_tests` (it was `postgres_tests`, and the
module name is the suite selector CI splits on).

```rust
fn open_test_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("tasks-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    Store::open(&dir.join(format!("{tag}.db"))).unwrap()
}
```

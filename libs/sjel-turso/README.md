# libs/sjel-turso

Turso and LibSQL storage engine for Sjel: **async multi-writer concurrency (`BEGIN CONCURRENT` / MVCC), embedded replicas, and native vector search**.

A shared library, not a capability: no domain of its own, no CLI (CONTRIBUTING.md#three-architectural-nouns). Consumers declare an `sjel-turso` path dependency in the workspace.

## Why this exists (PLAN.md)

PRD Q45 retired Postgres in favor of SQLite (`axon.db`) with `rusqlite`. While SQLite's WAL mode provided single-file simplicity and zero container overhead, its single-writer limitation (`SQLITE_BUSY` when multiple agents and UI threads interact) and lack of native embedded vector indexing created a ceiling.

`sjel-turso` brings LibSQL (Turso's open-source SQLite engine) into Sjel:
1. **Concurrent Writers**: Multi-writer support via MVCC and non-blocking writes.
2. **Embedded Replicas**: Microsecond local disk reads with background synchronization to remote Turso nodes when configured.
3. **Async Core**: Native Tokio asynchronous I/O matching Axum capability servers and GPUI background task loops.
4. **Byte-Level Compatibility**: Directly opens existing SQLite `.db` files (`axon.db`) without dump/reload.

## Connection Lifecycle

```rust
// Open a local LibSQL database with Sjel's connection PRAGMAs applied:
let conn = sjel_turso::open_local(&database_path).await?;

// Run schema migration once per process target:
sjel_turso::migrate_once(&conn, &database_path, "places", |conn| async move {
    conn.execute("CREATE TABLE IF NOT EXISTS places_items (...);", ()).await?;
    Ok(())
}).await?;
```

Connections apply:
- `busy_timeout = 5000` (5-second grace before refusal)
- `foreign_keys = ON` (explicit referential integrity)
- Standard Sjel ISO-8601 millisecond timestamps via `sjel_turso::NOW` and `sjel_turso::now_offset()`.

## `Store`: the shape a capability holds

`sjel_store::open_pool(path, prefix, migrate)` is what seventeen capabilities were written against.
[`Store`](src/lib.rs) is its async counterpart, and it exists so a cutover is a changed open call
rather than a rewritten crate:

```rust
// Opens the file, applies the PRAGMAs, runs this capability's DDL once per (file, prefix).
let store = sjel_turso::Store::open(&database_path, "places", |conn| async move {
    conn.execute("CREATE TABLE IF NOT EXISTS places_items (...);", ()).await?;
    Ok(())
}).await?;

let conn = store.connect().await?;  // one read
let tx = store.begin().await?;      // one write: `commit` keeps it, dropping it rolls back
```

`begin` starts `BEGIN IMMEDIATE`, so the writer lock is taken before the transaction reads
anything. That is not an optimisation: a deferred transaction that reads and then writes must
upgrade into the lock, and SQLite answers a failed upgrade with `SQLITE_BUSY` at once —
`busy_timeout` does not retry that case, which is how a read-then-write capability loses writes
under two writers. `tests::begin_takes_the_writer_lock_before_it_reads_or_writes` in `src/lib.rs`
is the regression that fails without it.

One trap in testing it: a `#[tokio::test]` runs on a current-thread runtime, and a transaction
blocked on the writer lock starves the task that would commit and release it. The wait then runs
the full `busy_timeout` and fails on its own timeout (measured 2026-10-21: 5.26s, then
`database is locked`) instead of on the behaviour being measured. Use
`#[tokio::test(flavor = "multi_thread")]` for a test that holds a lock across an await.

## One engine per binary

A capability links **either** `sjel-store` (`rusqlite`) **or** `sjel-turso` (`libsql`), never
both: each bundles its own SQLite and the duplicate C symbols do not link. That is the whole
meaning of a full cutover — the schema, the store, the handlers and the CLI move together, and
`rusqlite` leaves that binary's dependency graph. `cargo tree -p <capability> -e normal | grep
rusqlite` answers whether one is still there.

The first capability to finish that move is `capabilities/operator-profile` (2026-10-21), and
`capabilities/projects` followed it the same day. Their tables are still in the same shared file;
only the engine differs.

## Embedded replicas (default `replica` feature)

A replica is a local file that holds the primary's write-ahead log. Reads never leave the machine;
writes do, because LibSQL forwards them to the primary and the next pull brings them back. That
trade is the whole feature: it buys read latency and spends write latency, so it suits a device
that reads often and writes rarely, not the server that owns the data.

The sync client is on by default (decided 2026-10-21). It brings its own networking stack — hyper,
tonic, rustls — which took this crate's normal dependency graph from 56 to 188 packages. That cost is
paid once per workspace, because everything else that speaks TLS already links the same crates. The
alternative was an opt-in feature, and it meant every capability that wanted a replica needed
`features = ["replica"]` added to its Cargo.toml: a machine could not be set up without editing
tracked source, so `sjel setup` would have had to edit it. Configuration beats a code edit. A build
that must stay light opts out with `default-features = false`.

```rust
// A replica when the deployment's settings name one, else the shared local file.
let store = sjel_turso::Store::open_with(
    &path,
    "places",
    sjel_turso::Source::from_env()?,
    migrate,
)
.await?;
```

[`ReplicaConfig::from_env`](src/replica.rs) reads:

| Setting | Meaning |
| --- | --- |
| `SJEL_TURSO_URL` | The primary, e.g. `libsql://axon-owner.turso.io`. Unset means no replica. |
| `SJEL_TURSO_AUTH_TOKEN` | The primary's credential, from the environment or the keychain, never from a tracked file. |
| `SJEL_TURSO_SYNC_INTERVAL_SECS` | Overrides the 60-second pull interval. |

A configured replica that cannot be opened is an error, never a fallback to the local file: a
silent fallback is a device that stopped seeing the primary's writes with no signal.

What the open does, and what it refuses:

- The first pull runs before the DDL, so the migration decides the schema against the primary's
  current state and not against an empty file. A background task then pulls every interval and
  reports the first failure of each streak once.
- A replica connection carries no connection PRAGMAs. LibSQL's sync connection parses every
  statement to decide whether the primary must serve it, and its parser rejects
  `PRAGMA busy_timeout = <n>` outright while treating `PRAGMA foreign_keys = ON` as a write for the
  primary. The settings are moot there anyway: writes are the primary's and reads take no writer's
  lock. `Store::connection` records which functions to re-read if a later libsql lifts the limit.
- `Store::sync_now()` pulls on demand, for a read that must not be stale. `Store::sync_stats()`
  returns `syncs`, `failures`, `frames_synced`, `last_error` and `seconds_since_success`;
  seconds since success against the interval is the staleness bound a health surface should
  publish rather than claiming the replica is current.
- One file cannot be both the shared local database and a replica, in either order, and one file
  cannot follow two primaries or two intervals. Each is an error naming the conflict, not a second
  sync loop on the same path.
- A file that holds its own writes cannot become a replica. LibSQL refuses it and the error says to
  remove the file for a fresh copy; converting a live capability file would replace local state
  with the primary's log.

The settings reach a capability's process because `tools/service-runner.sh` passes
`<overlay>/config/deployment.env` into every process it starts (2026-10-21). Before that, a
capability had to repeat the same names in its own `[capability.<n>] env`, so there was no route from
"the deployment follows this primary" to "the program sees it".

The pull path itself is LibSQL's and is unverified here: this repository has no primary to point at,
so the tests cover this crate's decisions (refusing a misconfiguration before any network call, the
one-file conflicts, and what a local store reports), not a live sync.

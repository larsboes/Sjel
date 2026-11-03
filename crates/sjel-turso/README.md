# libs/sjel-turso

The LibSQL storage engine for Sjel: an async local database with native vector search.

A shared library, not a capability: no domain of its own, no CLI (CONTRIBUTING.md#three-architectural-nouns). Consumers declare an `sjel-turso` path dependency in the workspace.

## Why this exists

PRD Q45 retired Postgres in favor of SQLite (`axon.db`) with `rusqlite`. While SQLite's WAL mode provided single-file simplicity and zero container overhead, its single-writer limitation (`SQLITE_BUSY` when multiple agents and UI threads interact) and lack of native embedded vector indexing created a ceiling.

`sjel-turso` brings LibSQL (Turso's open-source SQLite engine) into Sjel:
1. **Concurrent writers**: `tests/multi_writer_concurrency.rs` covers concurrent inserts. Contention on the shared file in production is not measured.
2. **Async core**: Native Tokio asynchronous I/O matching Axum capability servers and GPUI background task loops.
3. **Byte-level compatibility**: Directly opens existing SQLite `.db` files (`axon.db`) without dump/reload.

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

## Embedded replicas: removed 2026-10-22

This crate followed a remote `sqld` primary as an embedded replica from 2026-10-21, on by default.
It was removed a day later, before anything used it. LibSQL's sync client pins hyper-rustls 0.25,
which pulls rustls 0.22, rustls-webpki 0.102 and h2 0.3. Those carry 10 known advisories, one
rated High (RUSTSEC-2026-0104), and libsql 0.10.0, the newest release, has no fix. Upstream
[issue #2266](https://github.com/tursodatabase/libsql/issues/2266) tracks the pin. The migration
that would lift it, [PR #2224](https://github.com/tursodatabase/libsql/pull/2224), is a stale
draft. A Cargo feature that defaults to off would not have helped: Cargo locks every declared
feature, so the crates stayed in `Cargo.lock` until the feature itself was deleted (measured).

To bring replicas back, revert the commit that removed them. Do that when the phone has to write
through a replica (R2 in `research/offline-on-the-phone.md`) and libsql has moved to rustls 0.23,
or fork libsql at that point.

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

## One engine per binary

A capability links **either** `sjel-store` (`rusqlite`) **or** `sjel-turso` (`libsql`), never
both: each bundles its own SQLite and the duplicate C symbols do not link. That is the whole
meaning of a full cutover — the schema, the store, the handlers and the CLI move together, and
`rusqlite` leaves that binary's dependency graph. `cargo tree -p <capability> -e normal | grep
rusqlite` answers whether one is still there.

The first capability to finish that move is `capabilities/operator-profile` (2026-10-21). Its
table is still in the same shared file; only the engine differs.

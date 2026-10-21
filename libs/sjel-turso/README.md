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

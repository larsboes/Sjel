# Storage engine compatibility probe

The probe uses generated files and a local ARM64 Debian image. It compares Turso with the pinned
libSQL engine on the same bytes. The engine runs without network access, with a read-only root,
read-only inputs and a writable temporary filesystem.

Generate the fixture directory from the repository root. It must not already exist.

```sh
sjel cargo run --offline --locked -p sjel-turso --example compatibility-fixtures -- "$scratch/fixtures"
sh research/benchmarks/storage-engine/probe.sh "$scratch/fixtures" "$scratch/tursodb"
```

The tested `tursodb` is from the [v0.8.3 ARM64 Linux release](https://github.com/tursodatabase/turso/releases/tag/v0.8.3).
The archive SHA-256 is `4a832c5125ea9137b020843c61731644666db90c450b027f5d5cffe455c4c3fb`.
GitHub's build provenance verified against the upstream release workflow and source commit
`607bff9fd0e547a25a97b3bf06e83bcc5d99d53c`. The probe runs the extracted binary directly.

| libSQL fixture | Turso read-only open | Turso normal open |
| --- | --- | --- |
| Rollback journal | Query matches, bytes unchanged | Query matches, bytes change on conversion to WAL |
| WAL | Query matches, bytes unchanged | Query matches, bytes unchanged |
| Rollback journal with vector index | Schema parse error, bytes unchanged | Schema parse error, bytes change |
| WAL with vector index | Schema parse error, bytes unchanged | Schema parse error, bytes unchanged |
| WAL after libSQL drops the index | Query matches, bytes unchanged | Query matches, bytes unchanged |

The query returns `3|4|1` for recursive reachability, `json_each` and exact vector ranking. Under WAL
and MVCC, the second CLI process encountered a file lock while the first held a write transaction.
After killing the first process, the committed row survived and the uncommitted row was absent in
both modes.

The deindexed fixture retains all embedding bytes and returns the same exact cosine ranking.
The candidate's experimental `toy_vector_sparse_ivf` index rejects those dense embeddings.
Its [documented similarity function](https://github.com/tursodatabase/turso/blob/v0.8.3/core/index_method/toy_vector_sparse_ivf.rs#L26-L31)
is Jaccard, which changes the ranking contract.

Run the Rust binding probe against the reviewed upstream checkout after its dependencies are
available in the local Cargo cache. It has two connections to one database handle and also builds
the same probe for arm64 iOS.

```sh
sh research/benchmarks/storage-engine/binding-probe.sh "$turso_source" "$scratch/binding"
```

With WAL and `BEGIN IMMEDIATE`, the second connection returned busy, then succeeded on retry after
the first committed. With MVCC and `BEGIN CONCURRENT`, two transactions writing distinct keys
overlapped and both committed. The arm64 iOS probe executable linked against the iPhoneOS SDK with
default Turso features disabled. It was not run on a phone.

The probe uses synthetic data and does not measure power-loss recovery, search latency at production
scale or compatibility with a copy of the deployed store.

## Side-by-side workload

```sh
sh research/benchmarks/storage-engine/binding-probe.sh "$turso_source" "$scratch/workload" workload
```

The optimized Rust probe uses libSQL 0.10.0 and Turso 0.8.3 on the same generated database. It
uses the [graph-store probe](../graph-store/README.md)'s edge schema and scale: 37,200 nodes,
84,232 edges and 22,261 call edges. It checks 120 three-hop queries and 24 exact top-10 queries
over 25,000 dense, 384-dimensional vectors. Four connections each commit 200 counter updates,
first on distinct keys and then on one shared key. Every graph result and ranked vector list
matches libSQL. All three engines preserve all 800 increments in both writer workloads.

Ranges below are from three optimized runs on one Mac. The engine order was fixed. A separate
end-to-end restore run also passed and showed higher graph read tails for Turso.

| Engine | Graph read p95 | Exact cosine p95 | Distinct-key tx/s | Shared-key tx/s |
| --- | ---: | ---: | ---: | ---: |
| libSQL WAL | 0.24–0.49 ms | 17.36–27.23 ms | 20.3–20.5k | 25.2–26.3k |
| Turso WAL | 0.72–0.77 ms | 19.44–19.94 ms | 16.0–17.5k | 15.4–16.4k |
| Turso MVCC | 0.62–0.69 ms | 22.36–22.98 ms | 47.2–47.6k | 20.8–23.5k |

MVCC committed the distinct-key workload without retries. The shared-key workload required
24–38 retries after write conflicts. The adapter checks autocommit before rolling back, since
Turso can already have aborted the transaction.

## Backup and rollback rehearsal

The native probe reopens the Turso-written WAL file directly with libSQL. SQLite 3.54.0 rejected
the MVCC file as a database. With the engine closed, the probe copies the main file and its
sidecars, opens that copy with Turso, switches it to WAL, and checks the counters and vector
results in libSQL. SQLite's `.backup` then produces readable snapshots of both WAL files.
`quick_check` returns `ok` and the restored counters, edge count and embedding count match.
The rehearsal covers closed-file recovery and rollback. On a separate generated MVCC file,
`VACUUM INTO` from an open connection retained the commit before the snapshot and excluded the
commit after it. Concurrent-write backup, a power cut and a physical iPhone remain unverified.

## Shared-store rehearsal and Documents search

Run against a private database extracted by `tools/restore.sh` from a verified store backup. Pass
the database file, the reviewed CLI and a new scratch directory as absolute paths.

```sh
sh research/benchmarks/storage-engine/store-rehearsal.sh "$restored_db" "$tursodb" "$scratch/store"
sh research/benchmarks/storage-engine/documents-search-probe.sh "$tursodb" "$scratch/documents"
```

The first missing table in catalog order was Documents' SQLite FTS5 virtual table. The synthetic
Documents probe reproduces the failure with an ordinary table before and after FTS5. Turso 0.8.3
reads the first and reports `no such table` on the second. The engine's [compatibility reference](https://github.com/tursodatabase/turso/blob/v0.8.3/COMPAT.md#full-text-search-fts)
documents the FTS5 incompatibility.

On the restored copy, removing FTS5 and its maintenance triggers let Turso match SQLite's row
counts across all ordinary tables. SQLite reopened the Turso-written copy, rebuilt the FTS5 index
from its external content table, verified that index against the content, and passed integrity
checking. The original restored file's checksum stayed unchanged.

Turso's FTS replacement matched the exact term on the synthetic Documents fixture but missed the
existing prefix and prefix-OR matches, even when filtering by status after the search. The pinned
Rust binding's arm64 iOS build with `fts` enabled also failed to compile on stable Rust with an
allocator type mismatch. The earlier build exercised the feature-disabled binding.

## Inventory owner-only rehearsal

```sh
sh research/benchmarks/storage-engine/inventory-rehearsal.sh "$restored_db" "$tursodb" "$scratch/inventory"
sh research/benchmarks/storage-engine/inventory-service-rollback.sh "$scratch/inventory/db/wal-restored.db" "$inventory_bin" "$scratch/rollback"
sh research/benchmarks/storage-engine/binding-probe.sh "$turso_source" "$scratch/native-inventory" inventory "$scratch/inventory/db/wal-restored.db"
```

The source is a restored shared-store backup. The rehearsal exports the ten Inventory tables,
their indexes and sequence counters to a separate file and checks the logical dump and declared
foreign keys. The same SQL runs on SQLite and Turso 0.8.3 over separate copies. It exercises item
revisions, stale writes, state history, wish groups and price observations. It also exercises an
atomic edit and operation receipt with a duplicate retry. The Inventory HTTP handler now has
that transaction on its SQLite path. The SQL probe does not exercise the HTTP adapter on Turso.

Turso WAL matches the SQLite acknowledgements and rejects an orphan wish-group membership. After
the candidate closes, SQLite backs up and restores the candidate's latest file, including the
acknowledged revision and its operation receipt. Integrity, foreign keys and the complete logical
dump match the SQLite control. Reopening the pre-cutover file alone would omit those writes.

The rollback rehearsal starts the Inventory service with the restored file and a separate empty
Devices registry. It reads revision 3, verifies an edit returns revision 4 and its ETag, and checks
that a stale edit returns a conflict. After restart, revision 4 and the original Turso write receipt
survive. SQLite reports valid integrity and foreign keys, the Devices file has no Inventory tables,
and the input checksum stays unchanged.

Pass the Finance server binary as the fourth argument to rehearse wishlist composition. The
rehearsal starts Finance on its own file, seeds a monthly balance, and verifies the Inventory
wishlist uses that balance for its affordability calculation. It then stops Finance and confirms
the wishlist remains available with an absent balance.

Pass the Interior binary as the fifth argument to verify its model and RoomPlan link routes
against the isolated Inventory service. The Interior file holds the link and has no Inventory
tables. The Interior CLI also loads the model from Inventory over loopback.

The comparison CLI points at the isolated Inventory service. The rehearsal verifies the scheduled
reader, compare output, specs, needs, alternatives and a price retry. The CLI's sentinel database
path remains absent and the first price remains recorded on the service.

The Rust binding probe injects a receipt insert failure after a revision update. Turso 0.8.3
defers drop-triggered rollback, and a prepared statement on the writer can still see the edit.
With explicit rollback, both connections see revision 3 and no receipt. Retrying records revision 4
once. A new Turso connection reads revision 4 and its receipt.

From that connection, `VACUUM INTO` runs while a writer holds the next edit and receipt
uncommitted. The closed snapshot contains revision 4. A separate restore after the writer commits
and closes contains revision 5. SQLite checks both files for integrity and foreign keys and matches
all ten owner tables and the sequence counters. This covers overlapping an open write in one
process. The scheduled backup path still needs an owner-side snapshot and restore rehearsal.

Switching the imported database to MVCC fails to insert Inventory history and price records. Turso
reports missing backing tables for its converted `AUTOINCREMENT` sequences. A fresh MVCC table
allocates IDs in increasing order after deletion, so the observed failure is in conversion. Full
HTTP acceptance on Turso and recovery during concurrent live writes remain open.

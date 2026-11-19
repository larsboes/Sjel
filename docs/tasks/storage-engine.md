---
id: storage-engine
title: "Measure one database engine"
state: verify
area: storage
kind: investigation
priority: 2
spec: docs/plans/storage.md
---

# Measure one database engine

## Outcome

Measure one database engine through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Review libsql and the Turso candidate before choosing with the owner.
- On copies, test unchanged-file opening, recursive queries, json_each, vector queries, concurrent writers, crash recovery and an iOS build.
- Keep capability writes behind owner contracts; whole-file sync is not an accepted outbox replacement.

## Verification

The [compatibility probe](../../research/benchmarks/storage-engine/README.md) compared libSQL 0.10.0
with Turso Database 0.8.3 on generated files. Recursive queries, `json_each` and exact vector
ranking matched on both unindexed files. Turso rejected `libsql_vector_idx` on open for both indexed
files, including read-only opens. The default CLI also refused a second process while the first
held a write transaction. The process-crash test preserved the committed row and rolled back the
uncommitted row.

Dropping the legacy index under libSQL lets Turso open the resulting WAL file without changing
its bytes. All stored embeddings and exact cosine results match. Turso's experimental sparse
index rejected the existing dense embeddings. In one process, WAL needed a retry and MVCC committed
both overlapping writes on distinct keys. Both modes preserved only committed data after process
death. The local-only Rust probe built and linked for arm64 iOS with Turso's default features disabled.

The side-by-side workload matched all recursive and vector query results. Turso MVCC committed
about 2.3 times as many distinct-key transactions per second as libSQL WAL. The shared-key
workload had no consistent gain with Turso. Turso WAL files reopened in libSQL. The closed MVCC
copy required a Turso conversion to WAL before libSQL could read it. SQLite backup and restore
checks passed for both WAL states.

Keep libSQL for the shared store. Turso MVCC merits a pilot on a rebuildable, single-owner
projection after measuring distinct-key write contention in that workload. Live MVCC backup,
power-loss recovery, physical iPhone behavior and the deployed-store schema remain unverified.

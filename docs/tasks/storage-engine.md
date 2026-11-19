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

Keep libSQL for the unchanged-file migration path. A production-store copy and power-loss
recovery remain unverified. Revisit with the owner if Turso accepts the existing index schema or
a separate migration can meet the search contract with a replacement.

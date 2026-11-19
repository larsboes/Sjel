---
id: storage-files
title: "Split capability databases"
state: backlog
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
---

# Split capability databases

## Outcome

Split capability databases through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Back up first and migrate one owner per commit: Inventory, Entities, Calendar, then remaining owners.
- Use one selected engine and one file/version per capability; cross-owner reads use APIs.
- Remove rusqlite from each migrated owner and archive, never delete, the final shared database.

## Inventory verification

The [owner-only rehearsal](../../research/benchmarks/storage-engine/README.md#inventory-owner-only-rehearsal)
matches the ten-table export, SQLite control acknowledgements, and the complete logical contents
after restoring the latest Turso WAL file into SQLite. The rehearsal preserves an acknowledged
edit and its operation receipt. Converted MVCC fails on existing `AUTOINCREMENT` sequences.

Finder routes local Inventory search through the owner. The SQLite sync path commits the edit and
operation receipt together. A failure-injection test verifies rollback before retrying. Inventory
can select an owner file independently of the shared device registry.

The Inventory service still uses SQLite. Before the live file switch, pass the owner API suite
against a Turso WAL adapter and rehearse migration and rollback with the running service. The
SQL-level WAL rehearsal does not exercise the adapter or service lifecycle.

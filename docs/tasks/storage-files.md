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

The rollback rehearsal ran the Inventory service on the restored file. It read revision 3, wrote
revision 4, rejected a stale edit, and found revision 4 after restarting. The Turso operation receipt
survived and the input file checksum matched.

## Remaining migration gates

- Route the wishlist's monthly balance through Finance before splitting the Inventory file.
- Run the owner API suite against a Turso WAL adapter, then rehearse the cutover and rollback
  with the adapter running. Register and restore the owner backup before accepting live writes.
- Keep Documents on its current search implementation until a replacement passes prefix,
  diacritic, status and pagination parity. The [engine measurements](storage-engine.md) record
  the FTS and iOS results.
- Any MVCC cutover requires recovery with the logical log and proof that backups preserve
  committed writes during concurrent use. Keep the original database through rollback acceptance.

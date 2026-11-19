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

The native Rust binding passed the WAL transaction and retry probe on the restored owner file.
An explicit rollback left both connections at revision 3 after a refused receipt. Native reopen
and SQLite backup preserved the once-committed revision 4 and its receipt.

The wishlist reads the monthly balance from Finance over loopback. A two-service rehearsal with
separate files confirmed the balance and wishlist totals. When Finance stopped, the wishlist
still returned its items and prices. The Interior purchase order uses the same Finance read.

Interior reads the catalogue and validates RoomPlan links through Inventory. The separate-file
rehearsal matched the catalogue count, accepted an owned item, and refused a wanted item. Interior
created no Inventory tables in its database.

A same-process Turso WAL snapshot ran while a second connection had an uncommitted Inventory
write. SQLite verified the ten-table snapshot at revision 4 and the later shutdown restore at
revision 5. The receipt for revision 5 appears only in the shutdown restore.

## Remaining migration gates

- Run the owner API suite against a Turso WAL adapter. Roll back failed transactions explicitly
  before reusing their connections. Turso 0.8.3 prepared statements can run before drop-triggered
  rollback. Rehearse cutover and rollback with the adapter running, and register and restore the
  owner backup before accepting live writes.
- Keep Documents on its current search implementation until a replacement passes prefix,
  diacritic, status and pagination parity. The [engine measurements](storage-engine.md) record
  the FTS and iOS results.
- Any MVCC cutover requires recovery with the logical log and proof that backups preserve
  committed writes during concurrent use. Keep the original database through rollback acceptance.

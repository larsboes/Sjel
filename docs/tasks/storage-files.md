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

The scheduled price refresh and the comparison CLI use the Inventory API for reads and mutations.
The rehearsal used an unavailable CLI database path and verified price retry, specs, needs and
alternative-group updates in the owner file.

All Inventory CLI entry points use the owner API. Creation commits the item and its initial state
together. Overlay import requires force for a nonempty database and commits the complete import
in one transaction. The rehearsal proved duplicate create refusal and rollback of a refused
initial state and a partial import.

A same-process Turso WAL snapshot ran while a second connection had an uncommitted Inventory
write. SQLite verified the ten-table snapshot at revision 4 and the later shutdown restore at
revision 5. The receipt for revision 5 appears only in the shutdown restore.

The live Inventory deployment uses the Turso WAL owner and a separate file for the Devices
registry. A receipt-backed shared-store restore supplied the ten-table export while Inventory
was held. The original shared database remains available for recovery.

The scheduled backup requested a fresh snapshot from the running Turso owner. An isolated
restore verified the archive against its receipt and matched all ten Inventory tables and
sequence counters. SQLite integrity and foreign-key checks passed. A Turso listener
also reopened the restored database and served its catalogue.

The live owner rejected unsigned sync. The signed sync test passed with an isolated Devices
registry and verified retries, nonce replay rejection, and receipt recovery.

## Remaining migration gates

- Verify a successful signed request from a paired client against the live Inventory owner.
- Move Entities and Calendar consumers to owner reads before splitting their databases.
  Finder still reads both from the shared store, the Now view reads Calendar there, and
  the phone mirror copies tables from both. [Route reads through owners](storage-owner-reads.md).
- Keep Documents on its current search implementation until a replacement passes prefix,
  diacritic, status and pagination parity. The [engine measurements](storage-engine.md) record
  the FTS and iOS results.
- Any MVCC cutover requires recovery with the logical log and proof that backups preserve
  committed writes during concurrent use. Keep the original database through rollback acceptance.

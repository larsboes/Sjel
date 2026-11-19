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

Inventory can select Turso WAL through its normal service entry point. It serves the catalogue,
search, references, item writes, imports, wishlist groups, price history, alias review and mail
proposals. The HTTP rehearsal checks stale edits, duplicate prices, rejected group members, alias
revocation and mail proposal privacy. A forced mail state failure leaves no partial item. The
receipt-backed SQLite restore preserves the acknowledged revision, item history and imported rows,
and matches the native wishlist, comparison, alias history and mail responses.

The native signed sync route shares the SQLite envelope rules and uses a separate Devices registry.
A paired-device fixture proved rollback after a refused receipt, successful retry, nonce replay
refusal, duplicate acknowledgement after restart and a stale conflict. A SQLite restore retained
the phone edit and its operation receipt.

The opt-in Inventory backup contract requests a fresh snapshot from the running native owner.
The isolated job rejects a stale snapshot when the owner refuses to export. It ships a fresh
snapshot, verifies the receipt and restores a file that both Turso and SQLite can open.
An overlapping uncommitted write does not enter the snapshot. The same service and rollback
rehearsal passed on an owner file extracted from the current shared-store archive.

## Remaining migration gates

- Install the native Inventory owner with a current extracted owner file and separate Devices
  registry. Verify owner health, the signed client path and a scheduled backup receipt on the
  deployment. Keep the shared database available through rollback acceptance.
- Move Entities, Calendar and the remaining owners through their own rehearsals.
- Keep Documents on its current search implementation until a replacement passes prefix,
  diacritic, status and pagination parity. The [engine measurements](storage-engine.md) record
  the FTS and iOS results.
- Any MVCC cutover requires recovery with the logical log and proof that backups preserve
  committed writes during concurrent use.

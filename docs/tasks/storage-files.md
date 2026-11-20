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

## Entities verification

A private extraction from a receipt-backed shared-store restore matched all nine Entities tables
row for row. Schema, indexes, integrity, foreign keys, and WAL mode checks passed on the copy.
The search cutover routes Finder through the Entities owner. The server and scheduled commands
share the opt-in owner database path. The server served the extracted catalogue with the
shared-file path pointed at an absent file.

The [Entities SQL rehearsal](../../research/benchmarks/storage-engine/README.md#entities-owner-only-rehearsal)
matched the Turso WAL commit, rollback and SQLite restore against the SQLite control.
The native store and SQLite control each passed the Entities API and store suites. The native
rehearsal committed an HTTP write, ran a job through the private owner socket and recovered the
acknowledged revision from a receipt-backed backup.

The live gateway rebuilds the phone mirror from the Entities offline projection. A download
matched the published digest, and a range response matched the requested bytes. The Entities
watchdog keeps the endpoint available to both the mirror and Finder.

The live Entities owner uses its own Turso WAL file. The name registry, Journal, and Google
Contacts jobs run through the private owner socket. A receipt-backed shared-store restore
supplied the nine-table export. Both the first scheduled Turso archive and the archive
after resuming the jobs restored with matching contents. Finder and the phone mirror
rebuild passed against the live Turso owner.

## Remaining migration gates

- Verify a successful signed request from a paired client against the live Inventory owner.
- Verify a refresh on a paired iPhone.
- Move Finder and the Now view to Calendar owner reads and replace its phone mirror table copy.
  [Route reads through owners](storage-owner-reads.md).
- Keep Documents on its current search implementation until a replacement passes prefix,
  diacritic, status and pagination parity. The [engine measurements](storage-engine.md) record
  the FTS and iOS results.
- Any MVCC cutover requires recovery with the logical log and proof that backups preserve
  committed writes during concurrent use. Keep the original database through rollback acceptance.

---
id: storage-projects-wal
title: "Move Projects onto its own Turso WAL file"
state: backlog
area: storage
kind: maintenance
priority: 2
spec: docs/plans/storage.md
---

# Move Projects onto its own Turso WAL file

## Outcome

Projects owns its index and review ledgers in a separate database. Markdown remains the authority for record content. The phone continues to sync Projects into its local replica.

## Acceptance

- Desktop reads use Projects APIs. The phone mirror builds from owner projections without the shared file.
- The CLI sends index operations to the owner. No independent writer opens the migrated tables.
- A receipt-backed shared-store restore supplies the owner tables. The first owner backup restores the records, mail proposals, application ledger and Inventory links with exact parity.
- Live service checks cover reads, a write, reindex and the existing Projects file backup.

## Verification

Desktop reads use the owner API. The default phone mirror no longer attaches the shared store. The Projects phone replica continues to use its existing sync. The Desktop core and mirror suites passed; neither client change has been deployed.

The opt-in Turso build passed 56 Projects tests, including schema migration, concurrent edits and a closed snapshot reopened in isolation. The default libSQL build passed 55.

[`projects-owner-rehearsal.sh`](../../research/benchmarks/storage-engine/projects-owner-rehearsal.sh) passed against a scratch libSQL store and Turso owner. The shared archive restored first; `owner-store-split.py projects` staged the four owner tables without copying an unrelated table. The protected snapshot and registered backup contract produced a second receipt-backed archive. Its isolated restore matched all four tables row for row. The owner read and wrote with the shared path absent.

The CLI sends every command to the owner's credentialed `/commands` job and never opens the
owner file. A route test runs a command, reads its output and refuses `edit` and unknown
commands; a CLI test checks that file arguments become absolute and record paths stay
relative. The default build passed 57 tests and the Turso build 58.

The [owner cutover command](../../tools/owner-store-cutover.py) supports Projects with the same
preflight, hold, receipt-backed restore and rollback as Health. Its owner file is
`data/projects-index/projects.db`, outside the Markdown tree in `data/projects`. Once that file
exists, the default CLI refuses to fall back to the shared tables. The cutover suite passed 15
tests for both owners, with one skipped that needs a legacy Health binary.

The extended rehearsal also drives the CLI through the owner and checks its refusal without one.
It has not run since that extension: the agent shell may not write the fixture credential.

## Remaining work

Run the extended rehearsal, then `python3 tools/owner-store-cutover.py projects --check` and
`--apply`, from a deployment operator terminal. Repeat the archive checks against live data, including reindex and the
Projects file backup. The live service remains on the shared database.

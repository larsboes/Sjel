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

## Remaining work

Route CLI operations to the service. Repeat the cutover and archive checks against live data in the deployment operator context, including reindex and the Projects file backup. The live service remains on the shared database.

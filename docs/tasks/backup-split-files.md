---
id: backup-split-files
title: "Back up each owner database"
state: backlog
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
---

# Back up each owner database

## Outcome

Back up each owner database through the existing owner, without treating implementation as live acceptance.

## Acceptance

- After each database split, declare one backup contract for its owner file.
- Pass an isolated tools/restore.sh rehearsal for each file, including Devices.

## Inventory verification

The native WAL probe used `VACUUM INTO` on a second connection in the Inventory process model
while a write was in flight. SQLite verified the closed snapshot with the prior committed edit,
its receipt, all ten tables and their sequence counters. The later committed edit appeared in
the shutdown restore.

The backup producer requests a fresh authenticated snapshot from the Inventory owner and
archives the closed file. The rehearsal verified that an existing file produces no receipt
when the owner refuses to export. A successful run wrote a new receipt on a local target and
restored all ten owner tables and their sequences. Native Turso reopened the restored catalogue.
The SQLite owner served the revisions and responses acknowledged before the backup.

## Remaining deployment checks

Enable the Inventory backup contract after starting the native owner. Verify a scheduled receipt
and restore from the configured target. Each later owner split, including Devices, needs its
own backup and restore proof.

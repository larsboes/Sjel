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
the shutdown restore. The production adapter, scheduled backup contract and `tools/restore.sh`
rehearsal remain open.

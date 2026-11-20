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

## Verification

The native WAL probe used `VACUUM INTO` on a second connection while an Inventory write was
in flight. The snapshot preserved the prior committed edit and its receipt. A separate
shutdown restore preserved the later commit.

Inventory, Entities, Calendar, Software, Punctuality, Documents, Finance, and Places
each have an enabled contract that asks the owner for a closed snapshot. All eight restored
under their immutable receipts.
Software's initial attempt could not reach the private snapshot socket during cutover.
The retry through the backup gateway succeeded after the watchdog was running and restored
its apps, decisions, and sequence counter. The [owner migration task](storage-files.md)
records the data checks for each owner.

The Documents catalog archive and canonical-originals archive each passed an isolated restore.

Finance and Places each restored their first native archive and matched every owner row.
Finance journal files retain a separate archive contract.

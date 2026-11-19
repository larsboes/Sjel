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

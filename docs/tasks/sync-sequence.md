---
id: sync-sequence
title: "Make change cursors durable"
state: backlog
area: storage
kind: fix
priority: 2
spec: docs/plans/storage.md
---

# Make change cursors durable

## Outcome

Make change cursors durable through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Write an integer-sequence change log and tombstone in the owner's record transaction.
- Calendar accepts the old stamp cursor for one release.
- Two store handles writing in one second lose no change on pull.

---
id: backup-empty-primary
title: "Handle a never-created database"
state: backlog
area: storage
kind: fix
priority: 2
spec: docs/plans/storage.md
---

# Handle a never-created database

## Outcome

Handle a never-created database through the existing owner, without treating implementation as live acceptance.

## Acceptance

- If sqld remains, a declared but never-created database reports nothing to back up yet.
- Close as cancelled with a reason if replica removal eliminates the consumer.

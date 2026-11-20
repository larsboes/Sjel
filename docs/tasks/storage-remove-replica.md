---
id: storage-remove-replica
title: "Remove unused replica infrastructure"
state: backlog
area: storage
kind: maintenance
priority: 1
spec: docs/plans/storage.md
---

# Remove unused replica infrastructure

## Outcome

Remove unused replica infrastructure through the existing owner, without treating implementation as live acceptance.

## Acceptance

- After the engine choice, remove unused sqld, libsql-auth, setup/turso, URL handling, installer and upstream entries unless replicas are explicitly justified.
- Retire its empty daily backup and verify no consumer depends on it.

---
id: storage-remove-replica
title: "Remove unused replica infrastructure"
state: done
area: storage
kind: maintenance
priority: 1
spec: docs/plans/storage.md
evidence:
  - "Removed sqld, libsql-auth, key rotation, the sjel setup turso component, its status routes, the installer and the toolchain entry on 2026-11-20; sjel-cli 416 and sjel-status 115 tests pass."
  - "sqld was absent from the live capability registry and had no attempts in sjel backup runs, so no backup or consumer depended on it."
---

# Remove unused replica infrastructure

## Outcome

Remove unused replica infrastructure through the existing owner, without treating implementation as live acceptance.

## Acceptance

- After the engine choice, remove unused sqld, libsql-auth, setup/turso, URL handling, installer and upstream entries unless replicas are explicitly justified.
- Retire its empty daily backup and verify no consumer depends on it.

## Verification

The `libsql-server` upstream verdict is `reject`. The Mac app's replica tab is removed separately in
[its follow-up](task-759d37d6-8ff9-41d6-a757-a8485d47df17.md) because the phone migration holds
the app workspace lockfile.

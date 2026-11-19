---
id: storage-files
title: "Split capability databases"
state: backlog
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
---

# Split capability databases

## Outcome

Split capability databases through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Back up first and migrate one owner per commit: Inventory, Entities, Calendar, then remaining owners.
- Use one selected engine and one file/version per capability; cross-owner reads use APIs.
- Remove rusqlite from each migrated owner and archive, never delete, the final shared database.

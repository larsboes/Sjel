---
id: storage-archive-backup
title: "Protect the unsorted archive"
state: backlog
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
---

# Protect the unsorted archive

## Outcome

Protect the unsorted archive through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Give the whole archive an independent backup contract and pass an isolated restore.
- Resolve staging/cleanup failures; a nonzero backup is not accepted.
- Prove coverage of excluded material and avoid relying on a second copy on the same physical drive.

---
id: sync-first-pair
title: "Preserve records during first pairing"
state: backlog
area: storage
kind: fix
priority: 4
spec: docs/plans/storage.md
---

# Preserve records during first pairing

## Outcome

Preserve records during first pairing through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Create records on an unpaired phone, then pair and sync.
- Every record remains on the phone and exists on the Mac exactly once.

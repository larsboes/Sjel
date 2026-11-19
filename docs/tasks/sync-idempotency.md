---
id: sync-idempotency
title: "Replay phone writes exactly once"
state: backlog
area: storage
kind: fix
priority: 4
spec: docs/plans/storage.md
---

# Replay phone writes exactly once

## Outcome

Replay phone writes exactly once through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Mint and persist a key with each phone mutation; resend it on every replay.
- Write each capability's key-to-answer ledger in the record transaction; retain keys for 30 days.
- Drop a create reply, replay it and find exactly one Mac record.

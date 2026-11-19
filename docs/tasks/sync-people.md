---
id: sync-people
depends_on: ["sync-contract","sync-idempotency"]
title: "Sync People proposals"
state: backlog
area: storage
kind: feature
priority: 4
spec: docs/plans/storage.md
---

# Sync People proposals

## Outcome

Sync People proposals through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Wait for the Entities outbox and shared domain contract.
- An offline proposal is reconciled by its owner, with explicit refusal and no duplicate people.

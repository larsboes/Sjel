---
id: sync-domain-shape
depends_on: ["sync-contract"]
title: "Unify phone domain replay"
state: backlog
area: storage
kind: feature
priority: 4
spec: docs/plans/storage.md
---

# Unify phone domain replay

## Outcome

Unify phone domain replay through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Calendar and Projects share one push/pull/apply shape with no per-domain pull loop.
- Replay task status, context and blocked-by as well as done, priority and due.
- Refusal blocks one record only, preserves its reason and protects pending local versions from pull.
- Create-only captures retain their current path.

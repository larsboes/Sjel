---
id: sync-inventory
depends_on: ["sync-contract","sync-idempotency"]
title: "Sync Inventory and wishlist"
state: backlog
area: storage
kind: feature
priority: 4
spec: docs/plans/storage.md
---

# Sync Inventory and wishlist

## Outcome

Sync Inventory and wishlist through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Add the phone store, owner feed and idempotent Inventory writes.
- An offline addition reaches the Mac once; Mac edits reach the phone.
- Use the shared domain contract before adding another writable domain.

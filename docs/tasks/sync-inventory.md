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

## Remaining work

The signed Inventory endpoint handles revisioned edits to existing items. An offline
addition also needs a signed create operation that commits the item, initial state and
operation receipt together. Verify that a lost response followed by replay returns the
same acknowledgement and leaves one item.

Extend the phone replica and its existing replay loop to queue Inventory mutations and
pull owner changes. Keep a queued operation until the owner acknowledges it and surface
revision conflicts on the phone. Test replay and a Mac-to-phone update on a paired device.

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

## Owner verification

The signed Inventory endpoint accepts an `interior.item` create with a stable operation
ID, item ID and initial state. SQLite and Turso WAL commit the item, initial state and
receipt together. Tests force receipt failure, retry the create and reject a second
operation for the same item. The signed Turso fixture replays after restart and recovers
the created item and receipt in SQLite.

## Phone replica verification

The shared phone core persists Inventory additions in the existing outbox and replays
them through the signed owner route. A lost response and an acknowledgement for another
operation keep the write queued. A matching duplicate acknowledgement clears it. A
write from an earlier pairing stays on the device.

## Remaining work

Connect the iPhone Inventory screen to the queue and its paired owner. Pull owner
changes into the local Inventory replica and surface pending writes and revision
conflicts in the app. Verify an offline addition and a Mac-to-phone edit on a paired
device.

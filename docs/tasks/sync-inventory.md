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

## Live read verification

The physical paired iPhone refreshed Inventory and Wishlist through the gateway's signed
GET route. The screen showed the live owner's counts after the refresh. This did not submit
an Inventory sync write.

## Remaining work

Connect the phone's Inventory additions to the outbox. Pull owner changes into the local
replica and surface pending writes and revision conflicts. Verify an offline addition and a
Mac-to-phone edit on a paired device.

---
id: "gpui-travel-phone-writes"
title: "Add phone trip edits through the owner"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
depends_on: ["sync-contract","gpui-travel-phone"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Add phone trip edits through the owner

## Outcome

Add phone trip edits through the owner through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Use Trips contracts for fields/bookings with explicit stale-revision refusal; do not imply NSH read-only travel already supports writes.
- Check offline queuing/replay under the shared domain sync contract before admitting another writable domain.
- Use a second genuinely branching trip to determine whether undecided branches require a model change; do not predesign them.

## Evidence

- Added `TripEditEnvelope` and `trip_edit` replay handler in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs), verifying paired device ownership, non-zero `base_revision`, and explicit HTTP 409 stale-revision refusal through `/trips/api/plans/{id}` under the shared sync contract.
- Verified with unit test `phone_trip_edit_replays_through_owner_and_refuses_stale_revision` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs).

## Unrun checks

- End-to-end phone-to-Mac trip field edit and stale-revision conflict inspection on a paired iPhone.

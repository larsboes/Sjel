---
id: "gpui-share-extension"
title: "Capture phone shares exactly once"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Capture phone shares exactly once

## Outcome

Capture phone shares exactly once through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Sign link/text/file shares with the paired device key via the shared Keychain group; do not add an unsigned shared-token Shortcut endpoint.
- Share from Safari while the Mac is unreachable, persist the outbox and deliver to Inbox once on reconnect.

## Evidence

- Added `SignedShareItem` and `ShareOutbox` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs), requiring paired-device signature verification through `group.sjel.device.keychain`, rejecting unsigned or short-token items, persisting queued shares offline, and deduplicating by `share_id` for at-most-once delivery to `/comms/captures` on reconnect.
- Verified with unit test `share_outbox_requires_paired_signature_and_delivers_exactly_once_on_reconnect` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs).

## Unrun checks

- Physical iOS Safari Share Extension capture while offline and reconnect replay to a paired Mac.

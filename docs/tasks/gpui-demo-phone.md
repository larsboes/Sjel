---
id: "gpui-demo-phone"
title: "Pair the phone to synthetic demo data"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
depends_on: ["gpui-dev-acceptance"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Pair the phone to synthetic demo data

## Outcome

Pair the phone to synthetic demo data through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Provide the demo LAN listener and Devices capability, then pair through the existing signed flow.
- Keep screenshots and device tests synthetic; no production snapshot becomes a demo seed.

## Evidence

- Added `SyntheticDemoSeed` (`SyntheticDemoSeed::canonical`, `SyntheticDemoSeed::validate`) in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs), enforcing `is_synthetic_demo: true`, `source_origin == "synthetic-fixture"`, and loopback listener binding while rejecting production snapshots.
- Verified with unit test `synthetic_demo_pairing_uses_synthetic_seed_and_rejects_production_snapshots` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs).

## Unrun checks

- Live iOS simulator pairing against the synthetic demo LAN listener.

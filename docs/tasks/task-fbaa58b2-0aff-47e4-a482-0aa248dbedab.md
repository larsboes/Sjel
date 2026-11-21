---
id: task-fbaa58b2-0aff-47e4-a482-0aa248dbedab
title: "Sync HealthKit from the iOS app"
state: "done"
area: data
kind: feature
priority: 3
depends_on: [task-c30b5f1e-8a96-4643-b3a4-f6c54dc93c7e]
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Sync HealthKit from the iOS app

## Outcome

The Sjel iOS app keeps the `health` capability current without a manual export. A Mac has no
HealthKit store, so the phone is the only live source.

## Acceptance

- The iOS app requests HealthKit read access and reads daily statistics and raw sleep samples
  (`apps/gpui/ios/host/Health.swift`). Built with the Swift HealthKit API rather than
  `objc2-health-kit`: the host already does platform work in Swift.
- When the app comes to the foreground with a paired Mac, it sends the days since the last sync
  (two days of overlap, 30 days on the first run) to `POST /health/api/days` with signed device
  requests (`apps/gpui/ios/src/health.rs`). Automatic reads are normally limited to one per
  15 minutes; failures can retry on the next foreground.
- iPhone Settings shows Health sync progress, the last upload the Mac confirmed, and a manual
  **Sync now** that bypasses the automatic interval. An empty read does not imply access was
  denied; the card points to the Apple Health permission controls.
- On a paired physical iPhone, **Sync now** records a new Mac import. **Last synced** updates
  only after the Mac stores data. Separate physical-device verification of the automatic foreground
  path remains open. Background delivery is outside this task.

## Evidence

The Health capability tests cover an upload whose sleep samples fold into the same nights
as the export. The iOS library builds for `aarch64-apple-ios`, the Swift host passes type
checking, and the receipt unit test rejects a zero-row acknowledgement. The headless Settings
test verifies that **Sync now** dispatches the action and that a confirmed time appears.

A signed development build with the HealthKit entitlement was installed on the paired iPhone.
The gateway initially returned `404` for `/health/api/days` because it had not loaded the enabled
Health route. After a gateway restart without rebuild, the same path advertised `GET, HEAD, POST`.
The operator retried **Sync now**, and the Mac recorded a new `upload: HealthKit` receipt.
After the Health database cutover, the operator retried on the same paired iPhone. The live
Turso owner returned fresh upload receipts and preserved the original receipt. Health values
were not inspected.

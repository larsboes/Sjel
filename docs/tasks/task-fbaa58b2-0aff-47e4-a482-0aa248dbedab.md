---
id: task-fbaa58b2-0aff-47e4-a482-0aa248dbedab
title: "Sync HealthKit from the iOS app"
state: verify
area: data
kind: feature
priority: 3
depends_on: [task-c30b5f1e-8a96-4643-b3a4-f6c54dc93c7e]
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
  requests (`apps/gpui/ios/src/health.rs`). Swift reads at most once every 15 minutes.
- Open: on the iPhone, opening the app updates today's values on the Mac
  (`health summary 1`).

Evidence so far: 12 health tests pass, including an upload whose sleep samples fold into the
same nights as the export. The iOS library builds for `aarch64-apple-ios`, and its unit test
passes on the Mac.

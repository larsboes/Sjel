---
id: "gpui-phone-polish"
title: "Fit shared views to the phone width"
state: "backlog"
area: "gpui"
kind: "fix"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Fit shared views to the phone width

## Outcome

Shared GPUI views read correctly at mobile density on a physical iPhone, with each fix checked on the device.

## Acceptance

- Calendar's week strip shows each day's label instead of `…`.
- The Projects search field keeps a usable width, and `+ New Project` stays inside the screen.
- The note editor shows no desktop key hints (`⌘S`, `⌘[`, `Esc to close`) at mobile density.
- Record a device screenshot per fix through `interceptor ios`.

## Evidence

- Seen on build 2357 from `apps/gpui/ios/run-device` on 2026-11-21: all four defects above.
- The phone exposes no accessibility tree to `interceptor ios tree`, so device checks tap by coordinate.

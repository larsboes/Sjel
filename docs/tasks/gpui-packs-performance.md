---
id: "gpui-packs-performance"
title: "Measure remaining Packs matrix cost"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Measure remaining Packs matrix cost

## Outcome

Measure remaining Packs matrix cost through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Measure the pinned-header/horizontal-scroller matrix before replacing it; retain row identity and shared virtual-list behavior where appropriate.
- Report p50/p95/max and frames over the 8.3 ms budget rather than treating old baseline numbers as current.

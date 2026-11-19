---
id: "gpui-chart-acceptance"
title: "Accept native chart interactions"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept native chart interactions

## Outcome

Accept native chart interactions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inspect Finance/Systems/Projects/Feed charts in both themes and narrow windows, including tooltips/legends/brush/reset/category toggle and clearing.
- Cover negative expenses, mixed currencies, partial directory scans, missing months, empty snapshots and invalid/absent dates.
- Do not chart historical completion velocity/reading cadence without capability event timestamps.

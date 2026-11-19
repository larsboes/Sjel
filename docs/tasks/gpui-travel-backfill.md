---
id: "gpui-travel-backfill"
title: "Choose safe historical place resolution"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Choose safe historical place resolution

## Outcome

Choose safe historical place resolution through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Ask how compound historical destination strings should split before running coordinate backfill; a guessed single point is not safer than null.
- Keep one owner coordinate per resolved stop and record unresolved names without silently rewriting trip structure.

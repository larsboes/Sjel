---
id: tool-target-compare
title: "Compare build roots read-only"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Compare build roots read-only

## Outcome

Report exact artifact identity separately from reclaimable space.

## Acceptance

- Keep the command deferred until cache reuse leaves a concrete comparison need.
- Extend storage target with name/size/hash comparison without modifying either tree.
- Do not delete or relink artifacts.

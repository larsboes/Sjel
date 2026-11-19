---
id: containment-opencode
title: "Accept a contained OpenCode run"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Accept a contained OpenCode run

## Outcome

OpenCode runs within the declared provider/project boundary.

## Acceptance

- Verify project writes/toolchains and refused sibling, overlay and credential reads.
- Verify provider/registry proxy egress, direct-egress refusal and admitted demo-node reading.
- Review native sandbox capabilities rather than assuming the wrapper replaces them.

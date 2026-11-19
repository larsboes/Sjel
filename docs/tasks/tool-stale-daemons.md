---
id: tool-stale-daemons
title: "Report upgraded running executables"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Report upgraded running executables

## Outcome

Host updates identify jobs still running changed or deleted code.

## Acceptance

- After upgrading, report every launchd job whose executable changed or disappeared.
- Do not restart a job without its existing lifecycle/approval contract.

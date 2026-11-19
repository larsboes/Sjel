---
id: homes-host-recovery
title: "Verify always-on host recovery"
state: backlog
area: homes
kind: feature
priority: 3
spec: docs/plans/homes.md
---

# Verify always-on host recovery

## Outcome

Verify boot, backup and power-loss behavior after a host is selected.

## Acceptance

- A cold boot starts declared services.
- A backup restores to an isolated destination.
- Power-loss recovery preserves data and reports failures.

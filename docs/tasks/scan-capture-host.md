---
id: scan-capture-host
depends_on: ["scan-roomplan-device"]
title: "Extract the shared capture host"
state: backlog
area: scanning
kind: feature
priority: 4
spec: docs/plans/scanning.md
---

# Extract the shared capture host

## Outcome

Reuse accepted RoomPlan camera, consent and outbox behavior.

## Acceptance

- RoomPlan device acceptance passes before extraction.
- Each scanner emits a typed immutable draft and retains offline retry behavior.

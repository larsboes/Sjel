---
id: tool-inference-admin
title: "Manage local inference settings"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Manage local inference settings

## Outcome

Supported admin commands reuse guarded credential handling.

## Acceptance

- Add read/set and clear-cache through the existing backend owner.
- Do not print keys or hand-edit a file the app rewrites; require approval for the selected mutation.

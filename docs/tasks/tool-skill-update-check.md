---
id: tool-skill-update-check
title: "Check global skill revisions"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Check global skill revisions

## Outcome

Global update discovery neither installs nor changes focus.

## Acceptance

- Report installed/available revisions through each owner's read-only metadata.
- Do not run an upstream update verb without documented dry-run semantics.
- Include Interceptor through metadata rather than opening its native updater.

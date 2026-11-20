---
id: tool-skill-update-check
title: "Check global skill revisions"
state: cancelled
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
reason: "Superseded by tool-unified-update, which retains the read-only revision checks."
---

# Check global skill revisions

## Outcome

Global update discovery neither installs nor changes focus. The
[unified update task](tool-unified-update.md) owns these checks.

## Acceptance

- Report installed/available revisions through each owner's read-only metadata.
- Do not run an upstream update verb without documented dry-run semantics.
- Include Interceptor through metadata rather than opening its native updater.

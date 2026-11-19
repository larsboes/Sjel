---
id: tool-runtime-agent
title: "Expose runtime status to agents"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Expose runtime status to agents

## Outcome

An agent can distinguish On-the-go deferral from failure.

## Acceptance

- Use an admitted read-only runtime route or the correct agent token.
- Return mode/defer reason without widening lifecycle or private-config access.

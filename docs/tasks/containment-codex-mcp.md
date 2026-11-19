---
id: containment-codex-mcp
title: "Verify Codex MCP admission"
state: verify
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Verify Codex MCP admission

## Outcome

A trusted Codex session reads Sjel through the agent privacy gate.

## Acceptance

- Call an admitted Sjel tool under sjel-auto permissions.
- Prove direct overlay-data access stays denied and c3 remains withheld.

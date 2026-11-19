---
id: tool-learning-rpc
title: "Check an opt-in learning round-trip"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Check an opt-in learning round-trip

## Outcome

The plugin can verify its live Pi question handoff without a throwaway script.

## Acceptance

- Reuse the plugin's own typecheck rather than introducing another dependency install.
- Gate an RPC quiz round-trip test behind explicit opt-in because it spends model tokens.

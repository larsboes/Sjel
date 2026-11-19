---
id: tool-diagnostic-run
title: "Run diagnostics on a selected deployment"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Run diagnostics on a selected deployment

## Outcome

A read-only diagnostic selects its deployment without bypassing hermetic build/test isolation.

## Acceptance

- Resolve Cargo's emitted executable from metadata instead of copying a cache path.
- Require explicit active-deployment selection and keep ordinary build/test state isolated.

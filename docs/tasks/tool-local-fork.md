---
id: tool-local-fork
title: "Check a local UI fork"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Check a local UI fork

## Outcome

A declared fork path can be checked without losing unrelated lockfile edits.

## Acceptance

- Wrap existing Cargo patch configuration in tools with explicit fork selection.
- Preserve concurrent lockfile changes and source identities; never restore an old lockfile over them.

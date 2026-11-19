---
id: tool-stale-paths
title: "Detect stale repository paths"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Detect stale repository paths

## Outcome

Doctor identifies tracked code/config paths that no longer resolve after moves.

## Acceptance

- Validate repository-relative path candidates against the current tree.
- Exclude prose examples and deliberate external paths without hiding real stale references.

---
id: tool-scoped-staging
title: "Stage session-owned hunks"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Stage session-owned hunks

## Outcome

Stage one session's changes without capturing another session's edits.

## Acceptance

- Snapshot at first edit and derive only the owned delta.
- Preserve unrelated staged files, reject changed HEAD and reconcile concurrent file changes.

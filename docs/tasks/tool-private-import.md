---
id: tool-private-import
title: "Import a workspace with history"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Import a workspace with history

## Outcome

Private repository consolidation preserves source history and unrelated destination work.

## Acceptance

- Use an explicit destination, retain a verified source backup and source commit as a parent.
- Cover dirty destinations and protected local files; verify the imported tree.
- Report state handoff paths without reading credentials.

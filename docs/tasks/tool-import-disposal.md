---
id: tool-import-disposal
title: "Trash verified import sources"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Trash verified import sources

## Outcome

Source cleanup uses the owner's intake proof instead of filename assumptions.

## Acceptance

- Add Import plan and explicitly approved apply; verify source/stored SHA-256 and stable regular-file identity.
- Cover stale source, missing original, symlink and partial-Trash failure.
- Exclude directories and excluded paths; never permanently delete.

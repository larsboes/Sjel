---
id: tool-index-architecture
title: "Generate architecture from staged inputs"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Generate architecture from staged inputs

## Outcome

Architecture generation can reflect the Git index instead of concurrent worktree edits.

## Acceptance

- Add an index-input option using git show :<path>.
- A fixture changes unstaged upstream rows and proves they do not enter index-based output.

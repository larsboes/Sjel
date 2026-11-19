---
id: tool-worktree-self
title: "Resolve repository identity in worktrees"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Resolve repository identity in worktrees

## Outcome

Doctor recognizes a worktree as the same repository as its primary.

## Acceptance

- Use top-level and common Git-directory identity rather than checkout basename.
- A worktree and primary report the same self references without hiding undeclared sibling connections.

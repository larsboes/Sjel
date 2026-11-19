---
id: tool-ci-revision
title: "Check an immutable outgoing revision"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Check an immutable outgoing revision

## Outcome

Committed checks do not execute against unfinished worktree changes.

## Acceptance

- Add ci-local fast --revision <sha> and bind success to the immutable snapshot.
- Keep hook secret scans and publication gates; do not equate selected committed inputs with an isolated execution tree.

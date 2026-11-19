---
id: tool-sandbox-target
title: "Select a writable build target"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Select a writable build target

## Outcome

A sandboxed build reports and reuses its selected writable target.

## Acceptance

- Select through existing build tooling rather than hardcoded scratch paths.
- Preserve live shared targets and report the actual target in verification evidence.

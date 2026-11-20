---
id: tool-orphan-root
title: "Reclaim a declared orphan cache"
state: cancelled
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
reason: "No current orphan cache requires explicit-root reclamation. Reopen when a concrete root needs cleanup."
---

# Reclaim a declared orphan cache

## Outcome

Add explicit-root inventory only when another concrete orphan needs cleanup.

## Acceptance

- Keep this conditional until a new consumer is observed.
- Reject active targets, source/state trees and symlink roots in guarded inventory/reclaim tests.

---
id: tool-atomic-install
title: "Install a verified standalone tool"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Install a verified standalone tool

## Outcome

One tooling command installs from an isolated build without rebuilding in a protected cache.

## Acceptance

- Derive CLI/scheduled destinations from their existing contracts.
- Refuse active executables and atomically install digest-verified bytes.
- Retain previous artifacts on failure.

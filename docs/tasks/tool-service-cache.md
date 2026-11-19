---
id: tool-service-cache
title: "Activate from a writable cache"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Activate from a writable cache

## Outcome

Service activation can use a writable build target without unsafe binary replacement.

## Acceptance

- Establish process ownership and stop before replacing an artifact.
- Add an explicit service-runner target with atomic verified installation.
- A failed build retains the old artifact and restart hold; active binaries are never overwritten.

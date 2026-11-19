---
id: tool-nonmutating-help
title: "Make nested help read-only"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Make nested help read-only

## Outcome

Inspecting deployment options never applies a profile.

## Acceptance

- Handle help and reject unknown options before writes.
- Prove settings, destinations and ledgers remain byte-identical after help.

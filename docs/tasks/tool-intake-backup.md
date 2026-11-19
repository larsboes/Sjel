---
id: tool-intake-backup
title: "Back up a filtered vault intake"
state: backlog
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
---

# Back up a filtered vault intake

## Outcome

An intake-specific receipt records coverage and exclusions without another staging copy.

## Acceptance

- Prove an isolated restore and reconcile excluded material.
- Do not claim metadata, empty directories or excluded trees from a filtered byte comparison.

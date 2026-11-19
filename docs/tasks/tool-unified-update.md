---
id: tool-unified-update
depends_on: ["tool-skill-update-check"]
title: "Unify update discovery"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Unify update discovery

## Outcome

One Sjel entry delegates to existing owners without side effects.

## Acceptance

- First provide read-only global skill checks; do not delegate to an installing check.
- Combine skills, Packs and existing package classes in one report while retaining owner verbs.

---
id: tool-launcher-rebuild
title: "Track launcher path dependencies"
state: backlog
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Track launcher path dependencies

## Outcome

A changed path dependency triggers a CLI rebuild.

## Acceptance

- Derive path dependencies from Cargo metadata or Cargo fingerprints.
- Changing a dependency of sjel-cli makes the next doctor use the new code.

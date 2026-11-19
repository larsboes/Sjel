---
id: tool-bundle-replace
title: "Retain denied app staging"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Retain denied app staging

## Outcome

A protected bundle replacement failure leaves a usable signed stage.

## Acceptance

- Detect the macOS rename denial and retain the signed staging bundle.
- Report its Finder replacement path; do not weaken access controls.

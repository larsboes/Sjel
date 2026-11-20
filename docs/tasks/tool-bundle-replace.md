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

## Current coverage

[`apps/gpui/install`](../../apps/gpui/install) signs the stage before replacement and prints the
retained path on failure. The replacement fixture preserves the old and incoming directories
on denied and partial moves. The desktop installer fixture verifies preservation on failed
signing. A native protected-folder denial and the Finder recovery guidance remain unverified.

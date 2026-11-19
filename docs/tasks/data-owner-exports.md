---
id: "data-owner-exports"
title: "Round-trip capability exports"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Round-trip capability exports

## Outcome

Round-trip capability exports through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inventory existing owner export routes and reuse them for mirror/export instead of adding a parallel route.
- Each capability exports open-format records with per-value classification and reimports into an empty store with equivalent records.
- Align shared fact shapes through their owners; retain original bytes.

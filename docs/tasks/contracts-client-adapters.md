---
id: "contracts-client-adapters"
title: "Remove copied client contracts"
state: "backlog"
area: "tooling"
kind: "investigation"
priority: 1
spec: "docs/plans/restructure.md"
---

# Remove copied client contracts

## Outcome

Clients reuse existing capability contracts without losing offline behavior.

## Acceptance

- Inspect existing API crates before extracting another contract.
- Each changed client imports its owning capability's wire model.
- Serialization and offline tests pass; travel geocoding stays with GPUI acceptance.

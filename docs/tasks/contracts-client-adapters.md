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

## Verification

The native client shares Projects API response envelopes for project lists, Inventory-link records,
mail targets, task lists and task writes. Tests cover legacy lists and task replies,
classified projections, exact record content, the revision required for link review, and offline
task handling. The Projects API and service pass 62 tests. GPUI core passes 403 tests with 6
ignored. Other copied adapters remain open.

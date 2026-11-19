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

The Projects list client uses the existing Projects response envelope. The API test accepts both
legacy summaries and classified summaries for the client while retaining classification in the
server projection. The GPUI core build and two offline replay tests pass. Other copied adapters
remain open.

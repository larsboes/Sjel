---
id: tool-unit-sync
title: "Sync one owned unit"
state: done
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
evidence:
  - "71 harness tests pass, including isolated unit refresh and invalid selector cases."
  - "tools/harnesses sync harness --unit sjel --harness claude and --harness codex succeed; status reports current."
---

# Sync one owned unit

## Outcome

Deploy one skill without materializing unrelated Pack changes.

## Acceptance

- Add an explicit unit selector with existing profile/drift guards.
- Prove sibling destinations and ledgers remain unchanged.

## Verification

The CLI routes `--unit <key>` to the existing guarded refresh engine without changing profile
selection. Tests preserve sibling destination bytes, sibling ledger records and the selected
profile. Missing unit values, unknown units, `--all` with `--unit`, and extra Pack arguments
are rejected. Whole-Pack deployment is not required.

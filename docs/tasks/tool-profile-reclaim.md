---
id: tool-profile-reclaim
title: "Reclaim one build profile"
state: verify
area: tooling
kind: maintenance
priority: 1
spec: docs/plans/tooling.md
---

# Reclaim one build profile

## Outcome

Storage cleanup can select a profile without removing its containing working cache.

## Acceptance

- Reuse existing path and live-use guards in storage reclaim.
- Reject active profiles and preserve sibling profile/cache contents.
- Verify the volume free-space change after an authorized live reclaim.

## Evidence

The storage crate tests verify profile selection, live-use refusal, and sibling preservation.
A live dry run with `--checkout-target --profile debug` selects only the checkout's debug
incremental directory. Live reclaim has not run.

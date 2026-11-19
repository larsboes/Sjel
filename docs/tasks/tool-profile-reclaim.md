---
id: tool-profile-reclaim
title: "Reclaim one build profile"
state: backlog
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

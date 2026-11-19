---
id: tool-backup-rehearse
title: "Rehearse cross-archive restores"
state: backlog
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
---

# Rehearse cross-archive restores

## Outcome

One backup command verifies an owner's originals and indexes in scratch.

## Acceptance

- Restore the newest owner archive plus its declared index archives.
- Run the owner's verifier and compare rows/digests with live without mutating live data.
- Reproduce the Documents restore check without manual joins.

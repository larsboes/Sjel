---
id: tool-ci-replay
title: "Accept full local CI replay"
state: "done"
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept full local CI replay

## Outcome

An unrestricted host completes native integrations and one full replay.

## Acceptance

- Run the four native macOS sandbox integration tests without weakening assertions.
- Retain a successful exact-revision full-replay receipt; interrupted runs and Actions results are not a local receipt.

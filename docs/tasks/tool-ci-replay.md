---
id: tool-ci-replay
title: "Accept full local CI replay"
state: verify
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Accept full local CI replay

## Outcome

An unrestricted host completes native integrations and one full replay.

## Acceptance

- Run the four native macOS sandbox integration tests without weakening assertions.
- Retain a successful exact-revision full-replay receipt; interrupted runs and Actions results are not a local receipt.

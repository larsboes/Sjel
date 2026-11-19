---
id: cv-cli-acceptance
title: "Accept installed CV discovery"
state: verify
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
---

# Accept installed CV discovery

## Outcome

The compiled public CLI exposes CV commands outside the checkout.

## Acceptance

- Run the public CLI contract against fresh binaries and verify help outside the checkout.
- Do not access CV private data or start a live harness session without its own approval.

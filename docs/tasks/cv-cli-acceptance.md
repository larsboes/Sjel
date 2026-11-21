---
id: cv-cli-acceptance
title: "Accept installed CV discovery"
state: "done"
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept installed CV discovery

## Outcome

The compiled public CLI exposes CV commands outside the checkout.

## Acceptance

- Run the public CLI contract against fresh binaries and verify help outside the checkout.
- Do not access CV private data or start a live harness session without its own approval.

## Verification

- A fresh offline scratch binary passed `crates/cv/check`, including complete and partial
  synthetic LinkedIn exports. Its help exposed `linkedin import` outside the checkout.
- CV unit tests verified that a malformed export, a failed API page, and an interrupted staging
  file do not replace the previous capture.
- Installed `sjel` discovery and live LinkedIn access were not tested.

---
id: tool-scratch-expiry
title: "Observe scratch retention recovery"
state: "done"
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Observe scratch retention recovery

## Outcome

Verification scratch expires under the existing two-day policy.

## Acceptance

- Verify that `sjel cargo --scratch` reuses the checkout's temporary fallback and refreshes its age.
- On a deployment with the [scratch policy](../../crates/storage/README.md#temporary-cargo-verification), observe expiry and recovered volume space after the retention interval.
- Keep live release locations unchanged; this verification authorizes no cleanup or service migration.

## Evidence

`tools/cargo-hermetic.test.sh` verifies selection, reuse, age refresh, and guard behavior.
Live policy admission and expiry remain unverified.

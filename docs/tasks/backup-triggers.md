---
id: backup-triggers
title: "Explain scheduled backup starts"
state: verify
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
---

# Explain scheduled backup starts

## Outcome

Explain scheduled backup starts through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Investigate why the timer was disabled, without assuming a cause from timing.
- Record explicit timer/manual trigger in each backup run.
- Keep the external drive as the sole chosen target and report absent-volume failures and retry behavior accurately.

## Verification

- The policy API showed both configured targets present with 24-hour intervals. The latest 500
  attempts contained 493 failures with the producer's absent-volume diagnostic and seven successes.
- The policy store keeps the current interval and its last update. It has no change history, so
  the reason for a prior timer disable is unverified.
- New ledger claims record `manual` or `timer`. Older rows read as `unknown`. The runner keeps the
  absent-target diagnostic ahead of the recovery hint. Tests cover trigger attribution, error
  reporting, and the 30-minute retry backoff.
- The live API reports `unknown` for older runs. Live verification of newly attributed starts
  remains open. No policy or backup destination changed in this task.

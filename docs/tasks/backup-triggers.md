---
id: backup-triggers
title: "Explain scheduled backup starts"
state: done
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
evidence:
  - "Scheduled attempts 3411–3417 on 2026-11-20 recorded trigger timer with exit code 0 and an archive receipt; manual run 3399 recorded manual."
  - "All 17 backup tests pass, covering trigger attribution, absent-target reporting and the 30-minute retry backoff."
---

# Explain scheduled backup starts

## Outcome

Explain scheduled backup starts through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Investigate why the timer was disabled, without assuming a cause from timing.
- Record explicit timer/manual trigger in each backup run.
- Keep the external drive as the sole chosen target and report absent-volume failures and retry behavior accurately.

## Verification

- The policy API showed both configured targets present with 24-hour intervals. The initial 500
  attempts contained 493 failures with the producer's absent-volume diagnostic and seven successes.
- The policy store keeps the current interval and its last update. It has no change history, so
  the reason for a prior timer disable is unverified.
- New ledger claims record `manual` or `timer`. Older rows read as `unknown`. The runner keeps the
  absent-target diagnostic ahead of the recovery hint. Tests cover trigger attribution, error
  reporting, and the 30-minute retry backoff.
- The deployed API reports `unknown` for the 500 older attempts inspected. All 17 backup tests
  pass.
- The operator started a Media backup from the desktop app. Run 3399 completed with exit code 0,
  published an archive receipt, and recorded `manual` in the live ledger.
- The scheduled run on 2026-11-20 recorded `timer` for attempts 3411–3417 (Finance, store,
  knowledge-base, Packs, Projects, Documents and Media), all with exit code 0 and an archive
  receipt (`sjel backup runs`).
- The cause of the earlier timer disable stays unverified: the policy store keeps no change
  history. The trigger field now makes each start attributable.
- Both enabled policies resolve to the same external volume. The one-off Archive restore copy
  on the internal drive is outside the recurring schedule.

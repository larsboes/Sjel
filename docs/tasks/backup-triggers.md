---
id: backup-triggers
title: "Explain scheduled backup starts"
state: backlog
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

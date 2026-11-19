---
id: "data-mail-events"
title: "Measure bilingual mail event extraction"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Measure bilingual mail event extraction

## Outcome

Measure bilingual mail event extraction through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Replay reviewed English/German invitations with explicit date/time/location frames and grounded programme details; unsupported dates and agenda entries remain rejected.
- Verify booking/ICS dates, older-page backfill, suggested existing-plan identity and event materialization without duplicates.
- Review the eleven possible newsletter entries left by the earlier gate; cleanup requires explicit approval, not automatic deletion.

---
id: "data-documents-enrichment"
title: "Evaluate source-linked document enrichment"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Evaluate source-linked document enrichment

## Outcome

Evaluate source-linked document enrichment through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Specify source identities, idempotent mail attachment intake, source-linked insights and class-gated enrichment on a frozen reviewed German/English corpus.
- Declare acceptance criteria before measurement; no c2 document reaches cloud inference and no c3 content enters a prompt.
- Measure broader document-boundary detection on annotated mixed scans; preserve approved child PDFs and exact page ranges.

## Evidence

- Mail attachment intake exists as an operator command: `comms mail attachments` stages
  receipt-thread PDFs under `<dir>/<thread-id>/`, and `documents inbox <dir> --recursive` imports
  them with the thread in the source path. Live run: 47 `belege` threads, 33 PDFs staged, 30 new
  documents, 3 already stored. A rerun saved nothing new.

## Not yet done

- No automatic run. Gmail answered HTTP 403 for 5 threads after repeated full runs; the command
  does not back off.
- Body-only receipts (14 of 47 threads, for example Booking.com "This is your receipt") stay in
  mail only.
- The triage store reaches back only to the start of its sweep history, so older receipt mail is
  out of reach.

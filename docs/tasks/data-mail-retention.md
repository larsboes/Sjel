---
id: "data-mail-retention"
title: "Trash mail only after its owner holds the evidence"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Trash mail only after its owner holds the evidence

## Outcome

A cleanup rule may trash a thread only when the thread has no value, or when the owning capability
has accepted what the thread carried: an event in Calendar, an offer with its expiry, a receipt in
Documents. An offer worth keeping is kept as an `.eml` original until it expires.

## Acceptance

- Each cleanup rule names its gate: `no-value`, or an owner whose accepted record links back to the
  thread. A gated rule selects only threads that have such a record.
- Offers (discount codes, sales, gift ideas) have an owner that stores the code, the expiry and the
  `.eml` original, and lists what is still valid.
- An expired offer and its `.eml` original become eligible for the trash rule without review.
- Bulk trash stays behind the owner's Allow; Sjel never deletes permanently.

## Done

- `comms mail cleanup` and `comms mail senders` select candidates from the overlay `cleanup`
  list; protected streams, keepers and waiting threads are never selected.
- First rule (`no-value` in effect): marketplace saved-search alerts older than 7 days.

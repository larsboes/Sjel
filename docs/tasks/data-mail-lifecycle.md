---
id: "data-mail-lifecycle"
title: "Preserve reviewed purchase corrections"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
depends_on: ["data-mail-identities"]
spec: "docs/architecture/data-intake.md"
---

# Preserve reviewed purchase corrections

## Outcome

Preserve reviewed purchase corrections through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Relate order/delivery/cancellation/return evidence across messages through source-scoped keys and owner links, not excerpt hashes.
- A delivery followed by a return remains traceable with conflicts and reviewer decisions, creates no duplicate item and never silently changes ownership.

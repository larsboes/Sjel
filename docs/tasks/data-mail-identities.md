---
id: "data-mail-identities"
title: "Connect reviewed mail identity resolution"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
depends_on: ["data-mail-evaluation"]
spec: "docs/architecture/data-intake.md"
---

# Connect reviewed mail identity resolution

## Outcome

Connect reviewed mail identity resolution through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Connect native candidate/alias review to source workflow; source adapters supply stable namespace and identifier kind.
- Prove a real owner-reviewed physical alias links evidence with visible provenance; product aliases and lexical similarity remain candidates and order numbers are not global identities.
- Reject stale/revoked evidence, item-ID reuse and ambiguous automatic linking while preserving reviewable unknowns.

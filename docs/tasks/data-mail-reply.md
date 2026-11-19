---
id: "data-mail-reply"
title: "Keep awaiting-reply mail visible"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Keep awaiting-reply mail visible

## Outcome

Keep awaiting-reply mail visible through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Reconcile the existing Waiting UI with an awaiting_reply state that survives Gmail archive/reconciliation instead of duplicating states.
- Show one Mac/CLI list after archive and clear only when the owner replies or dismisses; model proposals cannot silently clear the state.

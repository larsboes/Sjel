---
id: "agent-exact-edits"
title: "Qualify exact-edit permission exemptions"
state: "backlog"
area: "agent"
kind: "maintenance"
priority: 3
spec: "docs/architecture/agent-sessions.md"
---

# Qualify exact-edit permission exemptions

## Outcome

Qualify exact-edit permission exemptions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Implement only after approval of the protected guard edit; qualify an exemption against current unique text, not Pi fuzzy-match fallback.
- Whole-file replacement still requires a fresh read after an exempt edit.
- Resolve unknown/opaque A6 classification with tests; a small model is deferred until measured prompt reduction has no missed Block.

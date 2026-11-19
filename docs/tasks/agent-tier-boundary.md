---
id: "agent-tier-boundary"
title: "Enforce the native agent tier sandbox"
state: "backlog"
area: "agent"
kind: "fix"
priority: 3
spec: "docs/architecture/agent-sessions.md"
---

# Enforce the native agent tier sandbox

## Outcome

Enforce the native agent tier sandbox through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Install an OS write boundary or the existing agentbox boundary before shipping account tier; the static shell classifier alone is insufficient.
- Keep session tiers immutable, reject roots outside the permitted home boundary and allow plugin narrowing only; a higher tier requires a new session.
- Retain sjel tier without shell/file writes and account sudo refusal.

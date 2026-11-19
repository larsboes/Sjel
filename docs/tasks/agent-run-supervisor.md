---
id: "agent-run-supervisor"
title: "Bound unattended agent runs"
state: "backlog"
area: "agent"
kind: "fix"
priority: 3
spec: "docs/architecture/agent-sessions.md"
---

# Bound unattended agent runs

## Outcome

Bound unattended agent runs through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Enforce wall-clock and call/spending limits through a supervisor and prove it stops a run at its limit.
- An uncapped attended-approved exception is not a general unattended grant; do not launch a job without enforced bounds or explicit scoped approval.
- Keep evaluation, policy, permission and egress changes outside autonomous candidate authority.

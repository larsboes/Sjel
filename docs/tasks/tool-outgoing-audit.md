---
id: tool-outgoing-audit
title: "Report redacted outgoing findings"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Report redacted outgoing findings

## Outcome

A normal audit command identifies findings without exposing secret matches.

## Acceptance

- Add audit --outgoing --json with rule, path, line and commit metadata only.
- A synthetic fixture is diagnosable without secret or match fields.

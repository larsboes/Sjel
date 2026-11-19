---
id: tool-installed-snapshot
title: "Serve cached installed software"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Serve cached installed software

## Outcome

Agents and the dashboard read a stored snapshot rather than start a 30-second walk.

## Acceptance

- Sample daily through existing storage history and serve the last snapshot.
- Expose freshness and failures; reading the route never starts discovery.

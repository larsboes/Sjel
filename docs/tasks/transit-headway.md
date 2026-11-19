---
id: transit-headway
title: "Score missed-transfer waiting"
state: backlog
area: transit
kind: feature
priority: 2
spec: docs/plans/transit.md
---

# Score missed-transfer waiting

## Outcome

Account for the expected wait after a missed connection.

## Acceptance

- Apply P(miss) times the time to the next same-line departure.
- A fixed-input test ranks equal-catch journeys with different headways differently.

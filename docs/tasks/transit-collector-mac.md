---
id: transit-collector-mac
title: "Collect corridor observations"
state: backlog
area: transit
kind: feature
priority: 2
spec: docs/plans/transit.md
---

# Collect corridor observations

## Outcome

Poll the private watch set into idempotent line-level histograms.

## Acceptance

- Use a typed Observation schema and pure poll logic.
- Collect two weeks of weekday mornings for every line marked observable.
- Replaying a batch does not change its histograms.

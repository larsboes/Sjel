---
id: containment-demo
title: "Close live-node loopback reach"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Close live-node loopback reach

## Outcome

A contained process cannot fall through to a live node.

## Acceptance

- End-to-end read the demo node with its contained token.
- Deny live ports as a second layer and prove the live token never reaches the run.
- Retain class admission: unclassified/above-cap rows withheld; single inadmissible objects refused.

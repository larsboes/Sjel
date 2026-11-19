---
id: tool-clone-discovery
title: "Investigate duplicate behavior"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Investigate duplicate behavior

## Outcome

Use a clone detector only as optional discovery with reviewed findings.

## Acceptance

- Choose a concrete consumer and exclude generated, vendored and fixture code.
- Inspect shared behavior before extracting modules; do not make Knip or jscpd mandatory gates.
- Keep public-export and cross-file-clone coverage distinct from the existing compiler trial.

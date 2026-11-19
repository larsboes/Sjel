---
id: "data-mail-labels"
title: "Label the mail classification corpus"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Label the mail classification corpus

## Outcome

Label the mail classification corpus through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Human-label the 121 exported fixtures before examining model outputs; preserve the private corpus.
- Accept stream switching only with model agreement at least control plus ten percentage points and false eviction at most 2%; urgency needs its own gate.

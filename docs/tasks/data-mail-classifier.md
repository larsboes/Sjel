---
id: "data-mail-classifier"
title: "Evaluate an incremental mail classifier"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
depends_on: ["data-mail-labels"]
spec: "docs/architecture/data-intake.md"
---

# Evaluate an incremental mail classifier

## Outcome

Evaluate an incremental mail classifier through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Train TF-IDF/linear classification only from owner corrections between rules and the local-model rung.
- Beat rules-only control with the labelled stream-switch gates and never lower a data class; defer NER/phone structured extraction/graph linking until measured need.

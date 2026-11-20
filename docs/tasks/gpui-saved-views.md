---
id: "gpui-saved-views"
title: "Reload saved views without a model"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 1
depends_on: ["task-336bf8f4-9597-436e-84bb-ef6ff052ce59"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Reload saved views without a model

## Outcome

Reload saved views without a model through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Compose a validated view once and reload it after app restart without a model call.
- Retain schema/version/route validation, classified read admission and proposal-only mutations.

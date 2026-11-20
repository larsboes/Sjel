---
id: "gpui-view-format"
title: "Choose a validated view document format"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
---

# Choose a validated view document format

## Outcome

Choose a validated view document format through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inspect A2UI and OpenUI Lang prior art before choosing an adapter/schema; no format is selected yet.
  [The format comparison](../../research/svelte-adaptive-ui.md) covers json-render, A2UI and OpenUI
  Lang and defines an adoption gate.
- Bind only registry read routes to Sjel components; reject model-written code, write routes and text interpreted as instructions.
- Cloud composition receives only schema and pseudonymized examples; writes remain owner-mode proposals.

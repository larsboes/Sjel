---
id: "gpui-view-format"
title: "Choose a validated view document format"
state: "done"
area: "gpui"
kind: "investigation"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "A2UI document shape selected on 2026-11-20; validator work moved to task-336bf8f4-9597-436e-84bb-ef6ff052ce59."
---

# Choose a validated view document format

## Outcome

Choose a validated view document format through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inspect A2UI and OpenUI Lang prior art before choosing an adapter/schema.
  [The format comparison](../../research/svelte-adaptive-ui.md) covers json-render, A2UI and OpenUI
  Lang and defines an adoption gate.
- Bind only registry read routes to Sjel components; reject model-written code, write routes and text interpreted as instructions.
- Cloud composition receives only schema and pseudonymized examples; writes remain owner-mode proposals.

## Decision

On 2026-11-20 the owner selected a subset of the A2UI document shape: components named from a
trusted catalog, with the data model sent separately from the component tree. The Projects
registry is the catalog, and a Rust validator admits documents.

The [format comparison](../../research/svelte-adaptive-ui.md) ranked json-render first because
its renderer and Zod validation could be reused. GPUI can use neither, so Sjel writes the
renderer and the validator for any format. A2UI is an open Apache-2.0 specification, so agents
that already emit A2UI need no Sjel-specific format. OpenUI Lang stays deferred.

The validator is [task-336bf8f4](task-336bf8f4-9597-436e-84bb-ef6ff052ce59.md).

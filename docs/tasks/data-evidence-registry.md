---
id: "data-evidence-registry"
title: "Join reviewed evidence through owner registries"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
depends_on: ["data-mail-end-to-end"]
spec: "docs/architecture/data-intake.md"
---

# Join reviewed evidence through owner registries

## Outcome

Join reviewed evidence through owner registries through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- After mail acceptance, reuse Projects object/link declarations for one evidence-to-item-to-project query with classification/provenance intact.
- Do not introduce parallel ontology storage; RDF/reasoning remains deferred until an unmet interoperability/inference requirement is demonstrated.

---
id: "data-processing-roles"
title: "Measure remaining local processing roles"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
depends_on: ["data-mail-evaluation"]
spec: "docs/architecture/data-intake.md"
---

# Measure remaining local processing roles

## Outcome

Measure remaining local processing roles through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inventory assistant/summary/OCR/retrieval/generator consumers before deployment changes; source structure/keys and reviewed aliases precede inference.
- Evaluate lexical scoring and the Apple Natural Language adapter on task-specific reviewed labels; retain producer provenance and deliberate reindexing.
- Small statistical models require evidence of rules failure and independent held-out gates; mail feedback labels are not purchase-event labels.
- Keep AFM as sole extraction/mapping generator; no model server is stopped/removed and embedding/reranking replacements need their own measured plan.

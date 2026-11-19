---
id: "data-mail-evaluation"
title: "Measure mail evidence on independent labels"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Measure mail evidence on independent labels

## Outcome

Measure mail evidence on independent labels through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Expand frozen independently human-reviewed German/English physical-purchase, actual-return and project-assignment cases; agent-reviewed development sets are not human ground truth or an independent holdout.
- Report false/missed claims, incorrect targets, duplicates, provenance, context coverage and refusal separately; unsupported layouts abstain as a whole and a small baseline sets no acceptance threshold.
- Run owner-terminal freeze-native/run-native against eligible real mail; rules baseline omits the model role. Diagnose AFM admission through its owner without bypassing it; AFM is the sole generator.
- Keep corpora/reports private with mode 0600, retain no full bodies and never silently convert the pseudonymized pilot into native evaluation. Prove live classification and production chunking.

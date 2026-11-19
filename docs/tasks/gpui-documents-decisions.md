---
id: "gpui-documents-decisions"
title: "Complete reviewable document decisions"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 2
depends_on: ["gpui-documents-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
issue: "https://github.com/larsboes/Sjel/issues/246"
---

# Complete reviewable document decisions

## Outcome

Complete reviewable document decisions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Reconcile GitHub reports 245/246/247 with current implementation before coding; retain source-linked filing/compare, reviewable proposals and durable duplicate/split decisions.
- Use Documents as authority; merge retains survivor/absorbed IDs, filenames/source paths and hash-keyed dismissals, while split retains parent SHA-256, accounts for every page and keeps the original openable.
- A merge, dismissal and split survive restart and restore-index through versioned sidecars; dismissed unchanged pairs stay dismissed and every source path remains.
- Keep operator title/date/issuer/notes/page-count separate from extracted/proposed metadata; collections need demonstrated insufficiency of metadata grouping. Do not preempt concurrent owner implementation or reimplement duplicate detection.

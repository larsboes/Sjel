---
id: "gpui-search-quality"
title: "Measure Finder search misses"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Measure Finder search misses

## Outcome

Measure Finder search misses through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Reproduce a consequential non-ASCII case-folding or document-body miss before adopting FTS5.
- Preserve local-first results and per-source bounds; installed note-path acceptance remains storage-note-search, not a duplicate search task.

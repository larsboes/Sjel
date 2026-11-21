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

## Evidence

- Reproduced non-ASCII case-folding, diacritic, and German/Nordic ligature misses (`"Köln"`, `"München"`, `"Voßstraße"`, `"Zürich"`, `"Søren"`, `"Àda"`) across titles and document-body detail fields, and implemented `fold_search_text` in [`apps/gpui/core/src/search.rs`](../../apps/gpui/core/src/search.rs) (also shared with `people::filter_people`).
- Verified local-first prefix priority and non-ASCII/document-body matching in `non_ascii_case_folding_and_diacritics_match_titles_and_document_bodies` in [`apps/gpui/core/src/search.rs`](../../apps/gpui/core/src/search.rs).

## Unrun checks

- Interactive Finder query check in the running macOS app against live German/Nordic records.

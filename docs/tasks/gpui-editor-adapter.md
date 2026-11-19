---
id: "gpui-editor-adapter"
title: "Evaluate block editor round-trip parity"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
depends_on: ["gpui-note-rename"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Evaluate block editor round-trip parity

## Outcome

Evaluate block editor round-trip parity through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Compare Ely fixtures against core blocks for frontmatter, nested lists, tables, unsupported syntax, split/merge and slash behavior.
- Preserve byte-lossless refusal, debounce, in-flight revisions and save/conflict protocol; prove read-only behavior before mounting a replacement.
- Do not substitute a code editor or replace PDF rendering/search; keep the live-preview full-build choice separate.

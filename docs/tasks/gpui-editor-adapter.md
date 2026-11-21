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

## Evidence

- `apps/gpui/ui/src/editor/adapter.rs` compares `sjel_gpui_core::blocks::Doc` against `ely_gpui_component::documents::{BlockData, BlockKind}` (`evaluate_round_trip`, `ely_is_byte_lossless`, `split_frontmatter`):
  1. **Frontmatter:** Raw `BlockData::from_markdown` parses leading `---` as `BlockKind::Divider` and corrupts YAML frontmatter; pre-splitting frontmatter with `split_frontmatter` makes canonical heading/paragraph documents round-trip cleanly.
  2. **Nested lists & markers:** `BlockData::from_markdown` flattens nested list items and normalizes `*`/`+` bullets to `- `, whereas `Doc::parse` preserves indentation and list prefixes verbatim in `Block::prefix`.
  3. **GFM tables:** `BlockData::to_markdown` normalizes compact table delimiter rows (`|---|---|`) to `| --- | --- |`, whereas `Doc` preserves the exact table source lines.
  4. **Code fences, toggles & blank-line gaps:** `BlockData` preserves `mermaid` (`BlockKind::Diagram`), `math` (`BlockKind::Math`), and `<details>` (`BlockKind::Toggle`) blocks, but drops language identifiers on other fenced code blocks (` ```rust `) and normalizes multi-blank-line gaps (`\n\n\n`) to `\n\n`.
  5. **Byte-lossless gate:** Because `BlockData` normalizes nested lists, non-canonical table delimiters, arbitrary code fence language tags, and inter-block blank-line counts, `sjel_gpui_core::blocks::Doc` remains the authoritative byte-lossless backing model for `NoteEditor`, while `NoteEditor` adopts Ely `Badge`, `WordCount`, `ReadingTime`, and `KeyBar` chrome in `apps/gpui/ui/src/editor/mod.rs`.
- Verified via `sjel cargo test --manifest-path apps/gpui/Cargo.toml -p sjel-gpui-ui --lib -- editor`.

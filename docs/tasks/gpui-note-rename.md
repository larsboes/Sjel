---
id: "gpui-note-rename"
title: "Make note rename and saves lossless"
state: "backlog"
area: "gpui"
kind: "fix"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
---

# Make note rename and saves lossless

## Outcome

Make note rename and saves lossless through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Preserve or prevent typing during an in-flight rename; compare-and-write stale bases atomically through the owner store, not separate reads/writes.
- Verify Enter moves the file and rewrites wikilinks while Esc/blur revert; include non-round-tripping and hub notes.
- Ask about hub folder semantics before changing them; review the missing list-row rename affordance separately from title rename.
- Run iOS build and paired-device regression; phone titles remain read-only while replica identity is path-keyed.

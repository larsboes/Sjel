---
id: "gpui-editor-preview"
title: "Review the single-block preview prototype"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Review the single-block preview prototype

## Outcome

Review the single-block preview prototype through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Select an owner-approved one-paragraph benchmark note; install only when concurrent GPUI work permits it, then compare the default-off flag on/off.
- Verify unchanged wrapping, caret-only markers, bold/links, click/source mapping, typing, bidirectional selection, double/triple click, undo/redo, actual IME and candidate-window geometry.
- Measure click/type p50/p95/max and frames over 8.3 ms on the same release note; benchmarks disable saving and a multi-block run cannot isolate a first-paragraph prototype.
- Present geometry/accessibility/caret-blink/touch risks and ask before choosing projected text versus invisible input or extending beyond one paragraph; no full-build route or default enablement is approved.

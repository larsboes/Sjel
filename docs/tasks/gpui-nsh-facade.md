---
id: "gpui-nsh-facade"
title: "Isolate NSH behind an app-owned facade"
state: "cancelled"
area: "gpui"
kind: "maintenance"
priority: 1
reason: "The Mac NSH trial and renderer dependency are removed. Shared GPUI phone acceptance owns live regression checks."
spec: "docs/architecture/native-ui-boundaries.md"
---

# Isolate NSH behind an app-owned facade

## Outcome

Isolate NSH behind an app-owned facade through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Restrict direct NSH imports to nsh_ui and theme/nsh; move process-wide screen statics/global hosts into per-AppView state.
- Open NSH Mail in two Mac trial windows and prove actions/state do not cross windows.

---
id: "gpui-packs-performance"
title: "Measure remaining Packs matrix cost"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Measure remaining Packs matrix cost

## Outcome

Measure remaining Packs matrix cost through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Measure the pinned-header/horizontal-scroller matrix before replacing it; retain row identity and shared virtual-list behavior where appropriate.
- Report p50/p95/max and frames over the 8.3 ms budget rather than treating old baseline numbers as current.

## Evidence

- Pre-resolved per-row and per-cell matrix state (`ResolvedMatrixRow`, `ResolvedMatrixCell`) in `PacksPage::settle_derived()` across [`apps/gpui/ui/src/packs/types.rs`](../../apps/gpui/ui/src/packs/types.rs), [`apps/gpui/ui/src/packs/matrix.rs`](../../apps/gpui/ui/src/packs/matrix.rs), [`apps/gpui/ui/src/packs/cards.rs`](../../apps/gpui/ui/src/packs/cards.rs), and [`apps/gpui/ui/src/packs/mod.rs`](../../apps/gpui/ui/src/packs/mod.rs), retaining stable row identity (`matrix-row-{pack}-{skill}`) and consolidating notices and status pills onto Ely `InlineMessage` and `Badge`.
- Measured 40 frames of a 64-skill × 4-harness matrix (256 cells) in `packs_matrix_frame_cost_stays_within_budget`: `p50 = 0.86 ms`, `p95 = 3.06 ms` (well within the 8.3 ms 120 Hz budget).

## Unrun checks

- Live 120 Hz scrolling profile in the running macOS app window.

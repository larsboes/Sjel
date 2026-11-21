---
id: "gpui-travel-backfill"
title: "Choose safe historical place resolution"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Choose safe historical place resolution

## Outcome

Choose safe historical place resolution through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Ask how compound historical destination strings should split before running coordinate backfill; a guessed single point is not safer than null.
- Keep one owner coordinate per resolved stop and record unresolved names without silently rewriting trip structure.

## Evidence

- Added `compound_candidates`, `is_compound_place`, `BackfillClassification`, and `classify_backfill` in [`apps/gpui/core/src/travel/places.rs`](../../apps/gpui/core/src/travel/places.rs), holding compound historical destination strings (`"Berlin / Potsdam"`, `"Tokyo & Kyoto"`, `"München → Salzburg"`, `"Wien und Graz"`) out of automatic geocoding while keeping one owner coordinate per resolved single stop.
- Updated [`apps/gpui/ui/src/travel/mod.rs`](../../apps/gpui/ui/src/travel/mod.rs) and [`apps/gpui/ui/src/travel/overview.rs`](../../apps/gpui/ui/src/travel/overview.rs) so automatic backfill skips compound destinations and renders an explicit split warning badge (`"Compound destination · split before locating"`) with candidate stops and a `"Choose split in Trip Editor"` action.
- Verified with unit test `compound_historical_destinations_require_split_choice_before_coordinate_backfill` in [`apps/gpui/core/src/travel/places.rs`](../../apps/gpui/core/src/travel/places.rs).

## Unrun checks

- Interactive operator review of historical compound trip stops in the running desktop app.

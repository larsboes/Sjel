# Native UI boundaries

The [application README](../../apps/gpui/README.md) owns current platform interfaces.
The [GPUI plan](../plans/gpui.md) links unfinished native acceptance.
Repository priorities come from [PLAN.md](../../PLAN.md); agent expansion does not supersede
maintenance or daily workflows.

## Platform and ownership

Desktop GPUI and phone NSH share capability contracts and core view-models, ordering,
timeline geometry and sync rules. The dependency manifests own versions. Phone rendering does
not depend on a GPUI frame pump. The Swift host owns keys, native capture and lifecycle;
NSH owns mounted UIKit screens. A shared rule belongs in core or a capability, not both screens.

Native heavy-media views preserve platform PDF/map behavior. Capability writes, stale-revision
refusal, undo, proposals, sync status and cross-capability routing remain Sjel-owned.
Current fork and renderer decisions live in [UI-stack research](../../research/ui-stack.md).

Phone Documents/Mail are online-first over signed device requests. Only selected offline
documents and opened mail are retained with complete Data Protection. Offline actions queue
in the existing outbox and replay through the same owner write route, never a second writer.
Shared phone requests remain behind pairing; an unsigned shared-token capture endpoint is
not a substitute.

## Reuse and testing

Use Ely controls on desktop and NSH controls on the phone. Reusable missing desktop interaction
belongs in the owned fork, not a parallel widget implementation. Sjel-specific views retain
domain layout where a generic component cannot preserve it.

Before replacing a control, prove its production-root contract: drafts, validation, focus,
keyboard, pointer actions, disabled states and failure reporting. A remote terminal needs a
persistent transport-fed parser and ordered callbacks, not local spawn or repeated static replay.
A workspace dock remains non-modal and preserves seat identity, sessions, focus and stored width.

Ely tests own generic mechanics. Sjel tests own conversion, action identity, routing,
classification, proposals and persistence. Remove a test only when named retained coverage
owns the same failure boundary. Share window initialization without global parking exemptions;
keep pure assertions outside entity tests.

The block editor preserves frontmatter, unsupported syntax, byte-round-trip refusal,
debounce and revision/conflict safety. Live preview remains a default-off single-paragraph
prototype until app/IME/geometry measurements support an owner-approved full-build choice.

## Views and demo data

A schema-validated view binds Sjel components only to registry read routes. No model code runs.
A write is an owner-mode proposal. Cloud composition sees schema and pseudonymized examples;
mail/feed text remains inert text. Format selection remains unresolved.

The demo node uses synthetic data only. A production snapshot is not a seed or screenshot
source. Native rendering, keyboard/accessibility and device behavior require actual acceptance;
compilation and unit tests do not establish them.

# Native UI boundaries

The [application README](../../apps/gpui/README.md) owns current platform interfaces.
The [GPUI plan](../plans/gpui.md) links unfinished native acceptance.
Repository priorities come from [PLAN.md](../../PLAN.md); agent expansion does not supersede
maintenance or daily workflows.

## Platform and ownership

The desktop and phone share capability contracts, core view models, ordering, timeline geometry
and sync rules. The dependency manifests own versions. The Swift host owns keys, native capture
and lifecycle. On the phone, all application pages, panels, tabs, and back navigation use one
embedded GPUI window with Ely controls. The Swift host applies keyboard and safe-area geometry,
propagates appearance changes, and requests frames while the surface is active.
A shared rule belongs in core or a capability.

PDFKit and RoomPlan retain their native presentation. Travel renders its cached raster map in
GPUI. Capability writes, stale-revision
refusal, undo, proposals, sync status and cross-capability routing remain Sjel-owned.
Current fork and renderer decisions live in [UI-stack research](../../research/ui-stack.md).
Keep the host and Ely on the same GPUI Kit and engine versions. Match `gpui-pre-leaks` to Kit's
`gpui-pre` version, and check the Mac NSH trial when GPUI Fast changes. The Mac trial shares the
host's GPUI Fast instance. The phone uses the corresponding `gpui-pre` engine with the
GPUI Mobile backend.

Phone Documents/Mail are online-first over signed device requests. Only selected offline
documents and opened mail are retained with complete Data Protection. Offline actions queue
in the existing outbox and replay through the same owner write route, never a second writer.
Shared phone requests remain behind pairing; an unsigned shared-token capture endpoint is
not a substitute.

## Reuse and testing

Fix reusable control behavior in Ely and platform behavior in the GPUI backend. Keep phone
navigation and domain layout in Sjel and system integrations in the Swift host.

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

---
id: "gpui-phone-links"
title: "Register phone deep links"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Register phone deep links

## Outcome

Register phone deep links through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Declare the sjel scheme in the host and route onOpenURL into the existing shared target/phone-screen path.
- Verify known links, reminders and unknown-first-segment fallback without widening paired-device access.

## Evidence

- Added `PhoneDeepLink` and `resolve_phone_deep_link` in [`apps/gpui/core/src/route.rs`](../../apps/gpui/core/src/route.rs), routing `sjel://` links and `calendar:YYYY-MM-DD` reminder taps into the shared `Target` model with known-first-segment fallback and safe `Page::Home` fallback for unknown first segments.
- Verified with unit test `phone_deep_links_resolve_known_routes_reminders_and_unknown_segment_fallbacks` in [`apps/gpui/core/src/route.rs`](../../apps/gpui/core/src/route.rs).

## Unrun checks

- Live `onOpenURL` tap verification on an iOS device.

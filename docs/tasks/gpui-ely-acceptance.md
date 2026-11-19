---
id: "gpui-ely-acceptance"
title: "Accept migrated desktop controls"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept migrated desktop controls

## Outcome

Accept migrated desktop controls through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Check migrated forms/buttons/tabs/breadcrumbs in light/dark and narrow/minimum windows with validation, drafts, focus, keyboard and disabled states intact.
- Verify one toast per mutation, per-window viewport isolation, stale/loading/empty/error distinctions and persisted pane widths.
- Check installed remote Ely terminal and caret-menu Escape/outside/pointer/keyboard behavior without losing block input or terminal seat identity.
- Remove parallel controls/tests only when a named retained test covers the same failure boundary; keep pure logic outside windows and run fork tests on both engines for changed components.

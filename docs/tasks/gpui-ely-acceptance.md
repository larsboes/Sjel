---
id: "gpui-ely-acceptance"
title: "Accept migrated desktop controls"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept migrated desktop controls

## Outcome

Accept migrated desktop controls through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Check migrated forms/buttons/tabs/breadcrumbs in light/dark and narrow/minimum windows with validation, drafts, focus, keyboard and disabled states intact.
- Verify one toast per mutation, per-window viewport isolation, stale/loading/empty/error distinctions and persisted pane widths.
- Check installed remote Ely terminal and caret-menu Escape/outside/pointer/keyboard behavior without losing block input or terminal seat identity.
- Verify that horizontal tabs reveal the active choice on keyboard and host selection changes.
- Remove parallel controls/tests only when a named retained test covers the same failure boundary; keep pure logic outside windows and run fork tests on both engines for changed components.

## Verification

- The GPUI Kit upgrade passed both Ely engine checks, the Sjel UI suite, locked Mac checks for
  the NSH trial and `gpui-pre`, and a locked iOS simulator build.
- Headless coverage exercises keyboard reveal, manual scrolling and remote resize recovery.
  Live keyboard, pointer, narrow-window and remote resize acceptance remains open.

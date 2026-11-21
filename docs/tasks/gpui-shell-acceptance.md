---
id: "gpui-shell-acceptance"
title: "Accept persisted desktop layout controls"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept persisted desktop layout controls

## Outcome

Accept persisted desktop layout controls through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Restart and verify Today Auto/Focus/Travel modes and section customization; preference failures produce one toast.
- Inspect native Apple Maps placement/clipping, map switching and detached external fallback; check narrow windows.
- Check Packs effective/missing filtering and installed terminal/caret-menu behavior without resetting user state.

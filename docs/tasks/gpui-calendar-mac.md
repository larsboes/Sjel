---
id: "gpui-calendar-mac"
title: "Accept native Calendar interactions"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept native Calendar interactions

## Outcome

Accept native Calendar interactions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Verify Day/Week/Month and availability in light/dark, including overlaps, overnight/all-day events, dense month overflow and bounded viewport repair.
- Exercise keyboard selection/edit dispatch, drag/move/resize, cancellation, invalid time/DST refusal, empty-slot prefill, tentative styling and successful Undo restoring the actual event.
- Preserve exclusive all-day ends, timezone/source metadata, proposal review, stale-time refusal and Mail/Travel navigation through the owner.
- Confirm Google/iCloud/replica sync marks match observed state and unknown provider health is not reported healthy; verify the Ely calendar inside the installed production root.

---
id: "gpui-calendar-phone"
title: "Accept native phone Calendar gestures"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
depends_on: ["gpui-calendar-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept native phone Calendar gestures

## Outcome

Accept native phone Calendar gestures through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- On a paired device check the Ely day grid, quarter-hour edits, touch and long-press behavior,
  resize and cancellation, availability, and all-day and overnight layouts.
- Move an event and verify its Mac value and successful sync timestamp; verify Undo through real dispatch, not only an unavailable-shell unit test.
- Measure scroll cost and check both themes. The Mac and phone share the Ely day grid and core
  timeline rules.

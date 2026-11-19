---
id: "gpui-people-writes"
title: "Accept Mac People mutations"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept Mac People mutations

## Outcome

Accept Mac People mutations through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Create/edit with a cleared value, add/remove facts, declare fields and confirm two-press deletion in the installed app.
- Verify list refresh in place, stale validation, Ely draft retention, Enter submit, Escape cancellation and focus return.
- Use synthetic source fixtures for implementation; do not read private People data merely to build the feature.

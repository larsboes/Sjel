---
id: "gpui-project-tasks"
title: "Accept task and project controls"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept task and project controls

## Outcome

Accept task and project controls through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- In the installed Mac app set every task status, sort/group/filter, edit due/priority/status and remaining properties through the side panel.
- Check keyboard navigation, title rename and board drag/status progression; closed states and done booleans stay consistent.
- Test optimistic failure restoration; phone status/context/blocked-by writes remain refused until their outbox contract supports them.

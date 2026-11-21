---
id: "gpui-project-tasks"
title: "Accept task and project controls"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept task and project controls

## Outcome

Accept task and project controls through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- In the installed Mac app set every task status, sort/group/filter, edit due/priority/status and remaining properties through the side panel.
- Check keyboard navigation, title rename and board drag/status progression; closed states and done booleans stay consistent.
- Test optimistic failure restoration; phone status/context/blocked-by writes remain refused until their outbox contract supports them.

## Evidence

The Projects UI suite passes 18 tests, including focus filters and blocker links, due-date chart
counts, mounting and exiting guided review, and inline keyboard editing. Installed Mac workflow
acceptance remains open.

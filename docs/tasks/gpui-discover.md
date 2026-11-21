---
id: "gpui-discover"
title: "Accept native Discover workflows"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept native Discover workflows

## Outcome

Accept native Discover workflows through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Keep scout/Discover navigation inside GPUI and test source status, scans, save, dismiss, restore and calendar promotion with provenance.
- Refresh state without restart and retain owner write modes.

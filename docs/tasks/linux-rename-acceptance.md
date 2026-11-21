---
id: "linux-rename-acceptance"
title: "Accept Linux service migration"
state: "done"
area: "tooling"
kind: "maintenance"
priority: 1
spec: "docs/plans/restructure.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept Linux service migration

## Outcome

Linux upgrades retain service and firewall behavior after the rename.

## Acceptance

- Run systemd-unit migration on a live Linux host.
- Verify the sjel_filter nftables replacement; generated-output tests alone do not close acceptance.
- Preserve compatibility readers until persisted inputs and supported upgrades are checked.

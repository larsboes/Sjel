---
id: tool-remote-patches
title: "Report declared host patches"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Report declared host patches

## Outcome

Doctor can inspect only the overlay's named remote hosts.

## Acceptance

- Run a read-only package/reboot-status probe over SSH for declared hosts.
- Report unavailable hosts explicitly without upgrading or rebooting them.

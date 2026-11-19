---
id: tool-script-log-time
title: "Timestamp script service logs"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Timestamp script service logs

## Outcome

Process scripts and containers receive the same stderr timestamps as Rust services.

## Acceptance

- Reuse the server timestamp format in the runner pipe.
- Preserve exit status, line content and process ownership.

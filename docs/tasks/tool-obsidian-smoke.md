---
id: tool-obsidian-smoke
title: "Smoke-test a running Obsidian view"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Smoke-test a running Obsidian view

## Outcome

A debug-port check catches live view errors that unit tests miss.

## Acceptance

- Attach to a selected running debug port, reload the plugin and open its view.
- Fail on console errors; do not relaunch or change focus without explicit approval.

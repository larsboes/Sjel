---
id: "agent-terminal-mac"
title: "Accept the native Mac terminal"
state: "done"
area: "agent"
kind: "maintenance"
priority: 3
spec: "docs/architecture/agent-sessions.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept the native Mac terminal

## Outcome

Accept the native Mac terminal through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Run the shell preset, resize htop, reach the first scrollback line and verify live zsh command/exit marks.
- Check tabs, presets/folders, keyboard, selection, clipboard, appearance and End against the installed remote terminal, not a substituted local PTY.
- Preserve agent-token 401 refusal on every terminal route, process-group shutdown and start/end audit rows without input/output.

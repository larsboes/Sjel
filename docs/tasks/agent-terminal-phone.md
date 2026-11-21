---
id: "agent-terminal-phone"
title: "Accept paired-phone terminal sessions"
state: "done"
area: "agent"
kind: "maintenance"
priority: 4
depends_on: ["agent-terminal-mac"]
spec: "docs/architecture/agent-sessions.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept paired-phone terminal sessions

## Outcome

Accept paired-phone terminal sessions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Add and exercise the claude preset in the selected checkout; after locking the phone for two minutes, reattach and replay missed output.
- Attach Mac and phone together and compare output; test authentication per foreground session, background lock/visible-output clearing, keyboard, selection, paste and reconnect.
- Keep terminal routes loopback-only and agent-token closed; only the paired signed-device listener admits phone access.

---
id: "agent-client-phone"
title: "Accept cross-device agent sessions"
state: "done"
area: "agent"
kind: "maintenance"
priority: 4
depends_on: ["agent-client-mac"]
spec: "docs/architecture/agent-sessions.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept cross-device agent sessions

## Outcome

Accept cross-device agent sessions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Pair by QR over the tailnet and test the actual ACL; list saved/active sessions, start from the phone and follow up in the same Mac transcript.
- Interrupt/reconnect, resume sequence numbers and reload transcript after truncation; approve/deny from each client and clear the other card.
- Test stop, session grants, re-pairing and signed requests without widening loopback capability reach.

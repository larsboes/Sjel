---
id: "gpui-live-views"
title: "Render live views in the agent panel"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 1
depends_on: ["gpui-saved-views","agent-client-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Render live views in the agent panel

## Outcome

Render live views in the agent panel through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Mount validated live view documents in the existing agent panel without executing model code.
- Preserve source/read-route identity, data classes and owner-mode proposals through updates and failure states.

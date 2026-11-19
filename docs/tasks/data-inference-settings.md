---
id: "data-inference-settings"
title: "Expose backend health in existing Settings"
state: "backlog"
area: "data"
kind: "maintenance"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Expose backend health in existing Settings

## Outcome

Expose backend health in existing Settings through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Use capabilities/settings and crates/inference to serve authenticated backend/model/key-presence and role/backend/model reads without exposing secret values.
- Probe one role through the existing model-check owner and compare verdicts; no tracked model/key path can be absent from the surface.
- Use the existing GPUI Settings section, route manifest and generated architecture rather than a new client/verb framework; first delivery adds no new configuration writes.
- Provider-side checks and undo precede later writes. Migration/removal of key files requires separate explicit secret authorization.

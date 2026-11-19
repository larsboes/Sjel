---
id: "data-import-app"
title: "Accept a real folder import in the app"
state: "verify"
area: "data"
kind: "maintenance"
priority: 2
depends_on: ["data-import-routing"]
spec: "docs/architecture/data-intake.md"
---

# Accept a real folder import in the app

## Outcome

Accept a real folder import in the app through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Plan/run/status in the installed GPUI Import page match sjel-import status for a real mixed folder.
- Preserve route refusals, provenance, retry idempotency and source originals.

---
id: "gpui-travel-phone"
title: "Accept offline phone Travel"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
depends_on: ["gpui-travel-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept offline phone Travel

## Outcome

Accept offline phone Travel through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- An upcoming trip reaches verified offline-ready status then opens its timeline, map and booking PDFs in airplane mode.
- Check trip maps, pan/pinch, filters, stop details, cached detail and zoom 10–14 coverage on a paired device.
- Resolve contradictory fare-alert evidence with a real regression: a new low notifies once, repeats do not, and saved-trip checks work offline.

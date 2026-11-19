---
id: "data-import-routing"
title: "Accept every mixed-folder import route"
state: "backlog"
area: "data"
kind: "maintenance"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Accept every mixed-folder import route

## Outcome

Accept every mixed-folder import route through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Finance and Notes require accepting owners instead of pending routes; skip playground bundles, swiftpm and agent-state folders, and admit Keynote documents without reading credentials.
- A real mixed-folder run, interruption and retry account for every input, preserve source provenance and deduplicate bytes. Reject binary-plist placeholders instead of claiming usable originals.
- Retain originals until each batch, attachments and byte-inclusive backup/isolated restore pass; Media local attachments and mounted libraries retain separate contracts.
- Inspect the installed Media gallery after actual intake; existing backup proof does not authorize source deletion.

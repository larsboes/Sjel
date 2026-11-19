---
id: "gpui-registry-phone"
title: "Accept governed offline object browsing"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept governed offline object browsing

## Outcome

Accept governed offline object browsing through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Synchronize existing records, browse classified object properties/relations offline, then queue an edit and confirm browsing refuses until sync settles.
- Keep empty-project identity and fail closed on unknown metadata; never invent a default class.

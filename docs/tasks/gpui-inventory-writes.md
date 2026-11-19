---
id: "gpui-inventory-writes"
title: "Accept Inventory and wishlist mutations"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept Inventory and wishlist mutations

## Outcome

Accept Inventory and wishlist mutations through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Create/edit both views with read revisions, reject stale edits and reload; inspect state history and layout impact preview before explicit changes.
- Verify three priced wish candidates, dated history, needs/limit lines, alternatives-group cheapest-member budgets and daily idempotent price alerts.
- Keep Inventory the only writer, refuse search-page price sources and retain transparent source identity.

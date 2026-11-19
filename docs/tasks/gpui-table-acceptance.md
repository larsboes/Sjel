---
id: "gpui-table-acceptance"
title: "Accept native table accessibility and editing"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept native table accessibility and editing

## Outcome

Accept native table accessibility and editing through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inspect Inventory/Import/Projects/Documents plus remaining converted lists in both themes/narrow windows: sort/filter/select/keyboard-open/inline-edit/per-row actions/scroll retention.
- Resolve table accessibility labels in the fork and keyboard editing of Column::choices; decide whether the missing outer border is needed from visual evidence.
- Preserve existing Transactions, Documents filtering and installed Inventory layout evidence without treating read-only checks as mutation acceptance.

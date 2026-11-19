---
id: "gpui-mail-adapter"
title: "Prove read-only Ely Mail parity"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
depends_on: ["gpui-mail-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Prove read-only Ely Mail parity

## Outcome

Prove read-only Ely Mail parity through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- First compare a read-only adapter preserving proposal IDs, selected/opened state, metadata, authenticity, held writes, sync failure and caller message order.
- Verify safe preview-only bodies before adopting a reader; routing remains Sjel-owned.
- Composer adoption is not approved by this task and requires a reviewed draft adapter and explicit confirmation through the existing capability write mode.

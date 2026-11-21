---
id: "gpui-dev-acceptance"
title: "Accept desktop demo switching"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept desktop demo switching

## Outcome

Accept desktop demo switching through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Inspect seeded Documents and its reader and verify Agent session seed coverage on synthetic-only demo data.
- Click Open on dev node and Back to live; report unavailable demo stacks without silent failure.
- Review the still-missing distinct dev accent without duplicating environment resolution or copying production data.

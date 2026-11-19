---
id: "gpui-phone-links"
title: "Register phone deep links"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Register phone deep links

## Outcome

Register phone deep links through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Declare the sjel scheme in the host and route onOpenURL into the existing shared target/phone-screen path.
- Verify known links, reminders and unknown-first-segment fallback without widening paired-device access.

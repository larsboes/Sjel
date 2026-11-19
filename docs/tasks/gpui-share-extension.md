---
id: "gpui-share-extension"
title: "Capture phone shares exactly once"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 4
spec: "docs/architecture/native-ui-boundaries.md"
---

# Capture phone shares exactly once

## Outcome

Capture phone shares exactly once through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Sign link/text/file shares with the paired device key via the shared Keychain group; do not add an unsigned shared-token Shortcut endpoint.
- Share from Safari while the Mac is unreachable, persist the outbox and deliver to Inbox once on reconnect.

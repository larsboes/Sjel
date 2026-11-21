---
id: "gpui-mail-phone"
title: "Accept phone Mail replay and attachments"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 4
depends_on: ["gpui-mail-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept phone Mail replay and attachments

## Outcome

Accept phone Mail replay and attachments through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Triage ten real eligible mails online and compare Mac state; take one action offline and prove reconnect applies it once.
- Cache only opened mail with Data Protection, show pending outbox actions, and open a mail attachment in Documents.
- Verify NSH list/reader and destructive/class-raising confirmation paths on a paired device.

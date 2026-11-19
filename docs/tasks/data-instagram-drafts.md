---
id: "data-instagram-drafts"
title: "Prepare Instagram drafts in Media"
state: "backlog"
area: "data"
kind: "feature"
priority: 3
depends_on: ["data-media-large"]
spec: "docs/architecture/data-intake.md"
---

# Prepare Instagram drafts in Media

## Outcome

Prepare Instagram drafts in Media through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Extend Media collections with ordered attachments, caption, planned date and draft/edited/ready/posted states; native Media creates, orders and advances a draft.
- Keep Instagram in Media and LinkedIn in Posts; Media receives owner-selected originals and does not read the photo library.
- Editing remains external and posting manual from the phone; this task authorizes no platform API or consent system.

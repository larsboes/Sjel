---
id: sync-projects-feed
depends_on: ["sync-contract","sync-sequence"]
title: "Add the Projects changes feed"
state: backlog
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
---

# Add the Projects changes feed

## Outcome

Add the Projects changes feed through the existing owner, without treating implementation as live acceptance.

## Acceptance

- GET /api/changes includes delete and archive tombstones.
- Replace list-and-sweep pull and Reply::missing matching.
- Sync tests pass; a Mac deletion removes the phone record.

---
id: storage-owner-reads
title: "Route reads through owners"
state: backlog
area: storage
kind: maintenance
priority: 2
spec: docs/plans/storage.md
---

# Route reads through owners

## Outcome

Route reads through owners through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Finder search uses capability search routes; desktop Repo uses API crates.
- Mirror building requests owner exports instead of reading shared tables.
- Cover all three readers before file splitting.

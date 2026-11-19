---
id: "data-mail-end-to-end"
title: "Accept reviewed mail proposals"
state: "verify"
area: "data"
kind: "maintenance"
priority: 2
depends_on: ["data-mail-evaluation","data-mail-identities","data-mail-lifecycle"]
spec: "docs/architecture/data-intake.md"
---

# Accept reviewed mail proposals

## Outcome

Accept reviewed mail proposals through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Exercise existing-mail backfill and subsequent sweeps locally; approval/rejection survives retries and links the correct existing project/item without duplicate ownership.
- Cover multi-item orders, duplicate messages, ambiguous matches, stale revisions, delivery/cancellation/return distinctions and c2/c3 boundaries.
- Classify each message and respect stricter thread class; refuse c3 before prompting, withdraw unreviewed/rejected evidence on class change and retry failed withdrawal. Approved records are not silently erased.
- c2 evidence cannot create an unclassified Inventory item or enter a c1 Project record; scheduled scanning/HTTP replay stay disabled until explicit activation and acceptance.

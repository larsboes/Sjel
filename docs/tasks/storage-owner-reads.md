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

## Verification

Finder gets local Inventory results from Inventory's bounded search route. The route checks the
same words and returns the same hit fields without returning matching notes. Other Finder sources,
the desktop reader and the mirror remain in scope.

Inventory and Interior obtain monthly balance from Finance's owner route. The wishlist rehearsal
used separate Inventory, Finance and Devices files and exercised an unavailable Finance service.
Interior uses Inventory's owner catalogue for model and RoomPlan requests. The separate-file
rehearsal verified model loading and confirmed that only the owned item could be linked.
The scheduled wishlist price refresh and comparison CLI use Inventory's owner routes.
Inventory's remaining CLI reads, creation and overlay import also go through the service.

Finder requests a bounded name and kind projection from Entities. The owner endpoint preserves
the five-hit cap and matches every query word without returning field values or dated facts.
The status service skips an unavailable owner while continuing to search its other sources.

The phone mirror consumes the Entities offline projection. Only people with an explicitly
classified C0 or C1 offline label are exported. The gateway refuses older copies and local
diagnostic builds, and the phone ignores People data in legacy cached copies.

Finder and Now request bounded Calendar projections through the owner. The desktop Repo's
Calendar read uses the same entries route as the shell. The phone mirror uses the owner's
classified changes feed and carries only C0 or C1 rows. A fixture with stale shared rows
verified that the phone copy uses the owner projection. Live Finder and Now requests passed
against native Calendar, and the gateway rebuilt the phone mirror.

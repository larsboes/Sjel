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

Places resolves station names through Punctuality's exact EVA lookup. The lookup accepts
padded and legacy unpadded codes. The isolated native owner matched the SQLite control
for the name and the underlying punctuality histogram.

The Places attendee backfill reads Calendar entries through the owner. Its proposal tests
use the wire response, and the live owner returned 125 entries with attendee arrays.

Finder requests Finance subscription names and categories through a five-result projection.
It starts the enabled Finance service on demand after a refused connection and respects
maintenance holds through the service runner. The live Finder returned a matching subscription
after starting the stopped Finance owner.

Finder requests at most five Documents results through the authenticated owner route.

Places reads Finance expenses and candidate locations through credentialed Unix routes.
Live parity checked the spend count and cent total. Trip plans and paged Transit journeys
supply the live travel layer. The Media owner supplies indexed photo locations through a
protected Unix projection. The phone mirror reads the entire Places registry through its
protected owner projection, and the rebuilt copy matches all 257 rows.

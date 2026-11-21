---
id: "gpui-documents-phone"
title: "Accept offline phone Documents"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 4
depends_on: ["gpui-documents-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept offline phone Documents

## Outcome

Accept offline phone Documents through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- From Today open Documents and return; verify cursor paging, a real multipage PDF at fit width, pinch zoom and original SHA-256 equal to the library.
- Keep offline, disconnect the Mac/enable airplane mode, then list and open the retained original with NSFileProtectionComplete.
- Copy only deliberately retained documents; live requests use the signed paired-device client and do not mirror all Documents.

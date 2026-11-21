---
id: "gpui-documents-mac"
title: "Accept the Mac Documents workflow"
state: "done"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
issue: "https://github.com/larsboes/Sjel/issues/245"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept the Mac Documents workflow

## Outcome

Accept the Mac Documents workflow through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- In the installed app verify paged/sorted/resized document lists, PDF fit/zoom, thumbnails/page jumps, extracted-page search and side-by-side filing comparison.
- Verify Finder drag/drop and picker import send bytes to Documents and preserve original identity; owner proposals and durable duplicate/split decisions remain required, not implied by the reader.
- Check a real multi-page PDF and error/empty states without substituting headless tests for native acceptance; no blank/wrong page may replace an extraction/system-viewer fallback.
- Compare count with the owner API, verify server-side query beyond the first 500 records, and file by owner-typed category/project/archive without a retired dashboard dependency; keep bounded page cache, cancellation and transfer limits.

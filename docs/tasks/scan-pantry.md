---
id: scan-pantry
depends_on: ["scan-capture-host"]
title: "Scan grocery barcodes"
state: backlog
area: scanning
kind: feature
priority: 4
spec: docs/plans/scanning.md
---

# Scan grocery barcodes

## Outcome

Resolve EAN/UPC codes into an Inventory-owned pantry domain.

## Acceptance

- Review Open Food Facts licence and quality before use.
- Known products work from a local cache; stock tracks quantity, expiry and depletion.
- A barcode lookup sends only the public barcode.

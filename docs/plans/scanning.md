# Scanning

Phone work follows Priority 4. Capture is offline-first and immutable; owners validate drafts and write only after review. The Mac alone holds provider credentials.

<a id="3-rooms-roomplan"></a>

## Open

- [Accept RoomPlan on a device](../tasks/scan-roomplan-device.md).
- [Extract the shared capture host](../tasks/scan-capture-host.md).
- [Capture paper documents](../tasks/scan-documents.md).
- [Capture owned objects](../tasks/scan-inventory.md).
- [Scan grocery barcodes](../tasks/scan-pantry.md).
- [Propose receipt imports](../tasks/scan-receipts.md).
- [Add consented interior guidance](../tasks/scan-interior-guidance.md).

## Consent

Default to Local only. Still analysis requires a fresh action. Live guidance is off by default,
ends with the scan, samples at most every 15 seconds and at most four frames per scan.
Frames are not archived. Paid guidance requires separate provider, billing and spending approval.
A failed request must not interrupt capture. Pantry belongs to Inventory; restock is always reviewed.

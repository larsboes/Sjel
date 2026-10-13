# Plan

Ideas and open questions. A decision that has shipped moves into the code and leaves this file.
The contract of each unit stays in its `ISA.md`.

## Capture: what is still open

Shipped 2026-10-13: `comms` `/captures`, `sjel capture`, and the dashboard page `/inbox`.

### Capture from the phone

The phone reaches Sjel only through the LAN listener, and that listener admits only requests signed with a paired device's key (`libs/sjel-server/src/lan.rs`). An iOS Shortcut cannot sign a request, so it cannot post to `/captures`. Open.

### `#go` has no destination

It stays in the inbox. A candidate destination is `places`.

### Model suggestions

Domain rules suggest a tag (`dashboard/src/lib/capture.ts`). The model does not yet suggest anything for what the rules leave.

### Prices a page does not declare

`inventory wunsch` reads a price only from meta tags. Uniqlo declares none and has no JSON-LD (measured 2026-10-13). Its price is only in its own app state, as `"prices":{"base":{...,"value":39.9}}`. Open: a per-shop reader, or a price typed in at triage.

### Groups of wishes

A group with one constraint for all its items ("travel-friendly, fewer but better") has no shape in inventory. For now it is an untagged note in the inbox.

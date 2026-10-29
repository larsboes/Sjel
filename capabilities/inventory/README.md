# inventory

What the household owns, wants and gave away: furniture, gear and clothes in one table.
`interior` reads the rows to check layouts, and `trips` reads them over HTTP for packing lists.
It starts at boot (`autostart`), because packing lists need it while `interior` is on-demand.

```
inventory inventory                 owned, wanted and gone, with prices and the open total
inventory categories                the categories in use, and spellings that collide
inventory add <url> [--label X]     a shared link becomes a wanted row
                                    [--price 79,90] [--category kleidung] [--kind piece]
                                    [--size M] [--color white] [--seasons winter,spring] [--id x]
inventory import [--force]          inventory/*.toml into the tables
inventory serve                     HTTP API on port 8101
```

The dashboard pages are `/inventory` and `/wishlist`.

## Tables

| Table | What |
|---|---|
| `inventory_item` | one row per thing: sizes in cm, money in cents, clothing and gear fields |
| `inventory_item_state` | `owned`, `wanted` or `gone`, appended over time; a bought wish is a second row |
| `inventory_sync_operation` | operation ids already applied from a phone, so a retry is not applied twice |
| `inventory_mail_proposal` | source-linked mail evidence and reviewed decisions |

`kind` is `piece` (a thing), `slot` (a need with target sizes and no product yet) or `gear`.

## The English schema

Column and JSON names are English (`width`, `price_cents`, `size`, `seasons`, `notes`, …). A store
written with the earlier German names is renamed in place the first time the new code opens it,
inside one transaction, and German vocabulary in stored values is translated (`nord` becomes
`north`, `pflicht` becomes `required`). `store::LEGACY_FIELDS` lists every rename. German names
are still accepted on input: in `PUT`, `PATCH`, a phone's queued sync, and the overlay TOML files.

Run only binaries built from the same commit as the store. An older binary would add empty
German columns next to the renamed ones.

## HTTP

| Route | What |
|---|---|
| `GET /api/inventory` | every entry with its state |
| `GET /api/wishlist` | the open wishlist, its low and high totals, and the months it costs |
| `GET /api/wish-groups` | wish groups sharing a constraint, with members and cost totals |
| `POST /api/wish-groups` | create or update a wish group with its title and constraint |
| `PUT /api/wish-groups/{id}/items` | set the item members of a wish group |
| `DELETE /api/wish-groups/{id}` | delete a wish group |
| `POST /api/items` | create an entry; `state` is required |
| `PUT /api/items/{id}` | replace an entry |
| `PATCH /api/items/{id}` | change the named fields; `null` clears one, an unknown field is refused |
| `GET`, `POST /api/items/{id}/state` | the state history, or append a change |
| `POST /api/sync` | signed mutations from a paired phone |
| `GET`, `POST /api/mail-proposals` | list or stage mail evidence without changing an item |
| `POST /api/mail-proposals/withdraw` | delete undecided and rejected proposals of one mail thread, `{"source_prefix":"mail:v1:<thread>:"}` |
| `POST /api/mail-proposals/{id}/decision` | reject, create a delivered item, or match an existing owned item after review |
| `GET /api/items/{id}/mail-match-snapshot` | read the item and state revisions needed for a reviewed match |
| `GET /api/item-refs/{reference}` | resolve a typed item reference and current state |

Item field writes raise the row's `revision` and return it as `ETag`. A client that sends the revision
it read as `If-Match` gets `409` with the current row when another device wrote in between, and
nothing is overwritten.

Mail proposals retain a source reference and a short excerpt. A reviewed delivery can create an
owned item; a match checks both item and state revisions. Inventory items do not retain a data
class, so a `c2` proposal cannot create an item. It can be rejected or matched without changing
the item. Proposal routes are closed to agent tokens.

## Imports and the vault

`inventory/*.toml` in the overlay (`data/inventory/inventory`, else `data/interior/inventory`) was
the source of the first import. Since then the tables are the truth: `import` refuses to run over
existing rows without `--force`.

A slot's prose is in its row: `target`, `reason`, `notes` and `open_question`. Until 2026-10-20
`vault-writeback` also gave every slot a note under the vault's `Atlas/Interior/`, seeded once
from those columns and then maintained only in a marked region. On 2026-10-20 all 18 notes still
matched their rows, so the notes went to the vault's `Archive/` and the writeback was removed
([ruling](../projects/README.md#why-this-shape-sjel-owns-projects-tasks-and-habits)). Nothing in
this capability writes the vault.

## Not built

Intake from a link works (`inventory add <url>`). Intake from the object does not. Scanning an
object needs the extraction ladder to end at something better than text, and the only barcode
path is `tauri-plugin-barcode-scanner`, which reads the Mac's pairing QR code.

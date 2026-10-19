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
inventory vault-writeback           write slots into their marked region in the vault
inventory serve                     HTTP API on port 8101
```

The dashboard pages are `/inventory` and `/wishlist`.

## Tables

| Table | What |
|---|---|
| `inventory_item` | one row per thing: sizes in cm, money in cents, clothing and gear fields |
| `inventory_item_state` | `owned`, `wanted` or `gone`, appended over time; a bought wish is a second row |
| `inventory_sync_operation` | operation ids already applied from a phone, so a retry is not applied twice |

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
| `POST /api/vault/writeback` | the same as the CLI command |
| `POST /api/sync` | signed mutations from a paired phone |

Every write raises the row's `revision` and returns it as `ETag`. A client that sends the revision
it read as `If-Match` gets `409` with the current row when another device wrote in between, and
nothing is overwritten.

## Imports and the vault

`inventory/*.toml` in the overlay (`data/inventory/inventory`, else `data/interior/inventory`) was
the source of the first import. Since then the tables are the truth: `import` refuses to run over
existing rows without `--force`.

`vault-writeback` gives every slot a note under `Atlas/Interior/` in the vault named by
`obsidian.root` in `<overlay>/config/inventory.json`. The first run seeds the note; after that it
rewrites only the marked region, which holds numbers and never a sentence a person wrote.

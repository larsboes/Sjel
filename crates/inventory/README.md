# inventory

Household inventory for owned, wanted and retired items. Interior reads item attributes for layout checks, and Trips reads gear for packing lists.

## Why this shape: state is history

An item is separate from its state changes. State transitions are appended, so a purchase or retirement does not erase what the item used to be. A bought wish becomes an owned record rather than rewriting the original wish.

Wish groups keep alternatives and shared needs together. Their comparison uses explicit numeric specifications and dated price observations rather than product descriptions alone.

Mail-derived evidence is staged as a proposal. It cannot change inventory until reviewed, and C2 proposals cannot create item records.

## Run

```sh
inventory inventory
inventory categories
inventory add <url> [--label <label>]
inventory import [--force]
```

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. Its live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No direct mail-to-inventory mutation.** Mail evidence requires review before it creates or matches an item.
- **No vault write-back.** Inventory rows are the source of truth; the capability does not maintain a second editable copy in the vault.

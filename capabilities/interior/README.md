# interior

Checks apartment layouts against measured room geometry, clearance rules and inventory items. It can search or compose candidate arrangements and evaluate delivery, sunlight and purchase order.

## Why this shape: proposals before placements

The solver evaluates explicit room rules and item declarations. Search and composition return candidate layouts; they do not silently change measured placements. The reviewed RoomPlan placement path validates a confirmed observation-to-item link, item ownership, layout rules and the expected revision before it records a placement.

RoomPlan captures are stored as observations. Reconciliation can propose candidate matches, but only confirmed observation-to-item links can support a reviewed RoomPlan placement change. Inventory remains the owner of item records.

## Run

```sh
interior model
interior layouts
interior check <layout>
interior search <layout> --move <item-a>,<item-b>
interior compose --pieces <item-a>,<item-b>
interior serve
```

The service is on-demand and managed through `tools/service-runner.sh`. Its live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No duplicate inventory.** Interior reads item definitions from Inventory and stores only layout-specific geometry and placements.
- **No automatic placement from scan matches.** RoomPlan observations remain proposals until links and placement changes are confirmed.

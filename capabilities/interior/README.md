# interior

Checks furniture layouts of a flat against the flat's clearance rules, and searches positions
that pass them.

```
interior model                       the measured room, and what in it is still guessed
interior roomplan [--capture D]      what the RoomPlan capture holds, against room.toml
interior placements [--set id=x,y[,r]]  where the pieces really stand
interior layouts                     layouts on disk
interior check <layout> [--json]     check against rules.toml; exit 1 on a hard violation
interior tolerance <layout>          up to which measurement error the verdict holds
interior delivery <layout>           does every piece get through the door to its place
interior delivery --width 120 --depth 60   the same for an imagined piece
interior sun <layout>                when in the year which piece is in direct sun
interior declarations                which items are still judged by their name
interior purchase-order              which need to buy first, and when it is affordable
interior search <layout> --move a,b [--step 20] [--limit 6] [--out name]
interior compose --pieces a,b,c      furnish a flat from scratch (beam search)
interior plan [layout...] --out f    plans as HTML with their verdicts
interior serve                       HTTP API; the port comes from service.toml
```

The German subcommands (`toleranz`, `einbringung`, `sonne`, `deklaration`, `kaufen`) and the
`--b`/`--t`/`--zerlegbar` flags still work.

The dashboard page is `/interior` (`dashboard/src/routes/interior/`). It renders what this
capability returns and computes nothing itself: every verdict, corridor width and plan SVG comes
from here.

## A capture arrives from the phone

`POST /api/roomplan/captures` is the one write on the RoomPlan surface. The phone scans a room with
`RoomCaptureSession` and posts two things: the `roomplan-capture/v1` draft its native layer built
(`libs/roomplan-contract`, which had no reader on this side until this endpoint) and the parametric
USDZ as base64 — base64 because the shell proxies JSON and a paired device posts JSON, so a second
content type would need a second path through both.

Three things happen in that order, and the order is the design: the draft is validated against the
contract, the flat's own model is loaded (a capture for a flat this Mac does not know is refused
before anything is written), then the file is written to `<assets_root>/captures/<flat>/<draft id>/`
with the draft beside it as `observation.json` and read back through `roomplan::read_usdz`. The
capture is named by the phone's draft id, so a re-post replaces it instead of stacking a second copy
beside it, and a file the reader cannot parse is removed rather than left to become the "newest
capture" that `/api/roomplan/reference` and the dashboard would then draw.

The answer carries the comparison — the same document `interior roomplan --json` prints, from
`roomplan::comparison_json`, with the draft id where the CLI puts a path. The comparison is computed
from the file this Mac now holds, never from the numbers the phone sent: a phone that lied about its
walls would otherwise have its numbers reviewed instead of its walls.

## Where the data lives

No flat is in this repository (`tests/containment.rs` checks that). Everything comes from the
private overlay, found through `SJEL_PERSONAL_ROOT`. Without it the capability stops.

| Where | What |
|---|---|
| `<overlay>/data/interior/flats/<id>/room.toml` | polygon, walls, openings, fixtures, routes, location |
| `<overlay>/data/interior/flats/<id>/rules.toml` | walkway widths, named thresholds, light corridor, house rules |
| `<overlay>/data/interior/flats/<id>/layouts/*.toml` | the arrangements under discussion; archived ones in `layouts/archiv/` |
| `interior_placement` in the shared store | where pieces really stand, one row per piece |
| `inventory_*` in the shared store | the items, owned by `capabilities/inventory` and read through its crate |
| `assets_root` in `<overlay>/config/interior.json` | photos and RoomPlan captures, served on request only |

The flat is `SJEL_INTERIOR_FLAT` or `--flat`. With exactly one flat in `flats/`, that one is used.
Several flats without a choice are an error.

Files use English keys. Files written with the earlier German keys (`hauptraum`, `oeffnungen`,
`fix_moebel`, `schwere`, `abstaende`, …) still load: every field has its German name as an alias,
and German opening kinds (`tuer`, `glastuer`, `fenster`) become `door`, `glass_door` and `window`
on load. Names you chose yourself, such as item ids, layout names and threshold names, are data
and stay as written.

## The sample flats

`tests/fixtures/overlay/` is a complete `data/interior/` with invented geometry. It keeps the
German file format on purpose, so the tests prove the aliases. `muster` is the main sample;
`muster-nordkueche` mirrors its kitchen to the north wall. `tests/live_parity.rs` compares
against the real flat and the recorded results of the earlier TypeScript engine, and skips itself
without an overlay.

```bash
SJEL_PERSONAL_ROOT=capabilities/interior/tests/fixtures/overlay \
SJEL_INTERIOR_FLAT=muster target/debug/interior check a-frei
```

## How a layout is checked

Two primitives do the work. An exact Euclidean distance transform gives every free grid cell its
distance to the nearest occupied one. A maximum-bottleneck search gives the narrowest point of
the widest path between two waypoints. The grid step is 5 cm (`geometry::RES`).

There are two kinds of rule ids:

- **House rules** (`R1` to `R9`): the flat declares them in `rules.toml` with a severity and a
  text. The checker looks both up. A missing declaration stops the check.
- **Invariants** (`collision`, `room_boundary`, `walkway`, `access`, …): geometry that no flat can
  relax. They are not declared and carry no text.

`hard` blocks and `soft` warns. A layout passes only without an open hard violation. A rule the
flat declares but the checker cannot apply is listed under `unchecked`, so "passes" means "passes
the rules that were measured".

Every comparison against a threshold is also recorded as a reserve: how much room is left, or by
how much it was missed. `tightest_reserve_cm` is the margin a layout passes by.

## What a piece can declare

Without declarations the checker infers a piece's needs from its id (`bett*` is a bed). A piece
can state its needs on its inventory row instead:

```toml
opens        = "south"                       # the side that needs doors or drawers opened
open_clear   = 65                            # cm in front of that side, measured on the piece
wall_ok      = false                         # that side must not face a wall
expands      = { dir = "south", to = 165 }   # total depth unfolded
access_sides = 1                             # walkable sides needed
access_clear = 60
room_divider = true                          # stands free; left out of the wall-contact score
```

Sides are in the piece's own orientation and turn with `rot`. `interior declarations` shows, per
item, what the inference assumes, the declaration that writes exactly that down, and which layout
verdicts would change.

## Reports

- **tolerance**: bisects the measurement error that the estimated sizes (`estimated` on the
  item) may have before the verdict flips, and names the rules that break first.
- **delivery**: searches a path from the entrance to the target place, position and rotation
  together. The narrow side must fit the door. A piece with `disassembles = true` is not measured
  at the door.
- **sun**: sun position after the NOAA method, and the light patch through each glazing on four
  days of the year. Needs a location in `room.toml` and glass heights on the openings.
- **purchase order**: wanted items by urgency, then by whether a layout uses them, then by price,
  against inventory's monthly balance.
- **search** and **compose**: take minutes, so the HTTP API answers with a job number
  (`/api/jobs/{id}`). Results are proposals; `--out` or the dashboard writes one as a layout.

## Layouts

A layout file keeps its header comment through every rewrite: the header holds the reasoning for
a decision. A new layout never overwrites an existing one. Archiving moves the file to
`layouts/archiv/` and never deletes it.

## Known gaps

- The real flat declares little. 38 of 42 rows declare nothing. `bett_bestand` is 210 × 140
  against a 100 cm door and does not say `zerlegbar`. R9 needs `[lage]` and glazing heights, and
  13 open needs carry no price.
- The rules are tested against one real flat. A second real flat is what would show which of
  them generalize.

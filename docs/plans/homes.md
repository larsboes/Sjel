# Homes

The houses Lars runs: what Sjel knows about each one, and the tools he uses against their
systems. Proposed 2026-11-08. Nothing here is built.

## Why this area exists

Lars ruled on 2026-11-08 that everything he keeps is managed through Sjel, and that a missing home
is built in Sjel. Two things about the family home have no owner today:

- **Site notes.** The network inventory, WLAN state and known problems moved out of the
  family-homeserver repo on 2026-11-07. They sit in `<overlay>/data/family-home/`, a folder with no
  capability behind it.
- **Tools.** Sjel removed its home-automation Pack and capabilities on 2026-10-24, because they
  described one household (Sjel `7e300702`). The archive is in the family-homeserver repo under
  `ops/sjel-retired-2026-10/`: about 5,300 lines of Python and shell. The largest pieces are
  `fritz` (864), `homectl` (616), `printctl.py` (481), `ha-cli` (452) and `ha_ws.py` (193).

The 2026-10-24 reason still holds for the instance data. It does not hold for the clients: a
Home Assistant websocket client, a Fritz!Box TR-064 client and a Pi-hole API client know no
house. They are external-system code, which is what a capability holds (`on-placement`).

## What stays outside Sjel

The family-homeserver repo stays its own repo. The Pi clones only that repo, Sjel does not run on
the Pi, and family members administer the house with Lars. The placement rule against "another
private repository for one physical instance" targets Sjel overlays. This repo is the deploy
source for a host Sjel does not run. It records that decision in its
`docs/plans/2026-11-standalone-repo.md` (2026-11-07).

Sjel reads that house through its interfaces only: the backup receipt `last.json` (already read
by `tools/doctor`) and the Home Assistant API.

## Proposal: one capability, `home`

Crates `home` and `home-api`, per the one-API-crate-per-capability decision (`PLAN.md`,
2026-10-25).

| Part | Holds | Ported from |
|---|---|---|
| Site registry | One record per house in the overlay: name, `places` id, Home Assistant URL, router host, Pi-hole host, credential item names | `home-assistant.vars` (the `FRITZ_*`, `HA_URL`, `PIHOLE_HOST` keys) |
| Site notes | `<overlay>/data/home/<site>/`, read and written through `home` | `data/family-home/` |
| Home Assistant client | REST and websocket, entity list, state, service call | `ha-cli`, `ha_ws.py` |
| Entity check | Fails when a dashboard or automation YAML names an entity the instance lacks | `ha-dashgen check`, step 2 of the family repo's mobile UI plan |
| Router | Device list, WLAN state, drift against a stored baseline | `fritz`, `netmon`, `wlanwatch` |
| DNS filter | Status, top blocked domains, enable/disable | `pihole` |

Not ported:
- `homectl`: its template layer was deleted on 2026-11-07, after it drifted 90 lines from the
  live automations.
- `ha-dashgen`, `ha-dashboard`, `ha-deploy`: the dashboards moved to Home Assistant's YAML mode,
  deployed by `git pull` on the Pi.
- `energy-dashboard`, `espctl`: no current use found. Port on first need.

**Data class.** Site notes for the family home name other people's devices and habits. They are
`c2` by default, so they reach a model only as `trusted_view` (`PLAN.md`, 2026-11-03).

## The second home

`PLAN.md` (2026-11-03) plans an always-on Mac mini as Lars's own home server. It runs Sjel, not
Home Assistant, so it shares less with the family Pi than "a base template for both" suggests.
What it can share, by my estimate: the nightly backup with a receipt that doctor reads. Extract
that when the Mac mini exists. Until then, `home` treats the Mac mini as a second site with no
Home Assistant.

## Order

1. `home` with the site registry and site notes. Move `data/family-home/` to
   `data/home/family/`.
2. Home Assistant client and the entity check. The parents' phone view needs the check.
3. Router client, then the family repo's WiFi runbook calls `sjel home net` in place of `fritz`.
4. DNS filter.
5. Printing, only if the printer is still in use: Moonraker client as its own capability, not
   part of `home`.

## Open

- Is the Elegoo printer still in use? It decides item 5.
- Name: `home` or `homes`. This plan assumes `home`, for the CLI verb `sjel home`.

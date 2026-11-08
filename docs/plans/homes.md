# Homes

A house Lars runs is a site. This area gives sites an owner in Sjel: what Sjel knows about each
one, and house-independent clients for the systems a site runs. Proposed 2026-11-08. Nothing here
is built.

## Why this area exists

Lars ruled on 2026-11-08 that everything he keeps is managed through Sjel, and that a missing home
is built in Sjel. Since the home-automation Pack and capabilities left Sjel on 2026-10-24
(`7e300702`), two things have no owner:

- **Site notes.** Network inventory, WLAN state and known problems sit in an overlay data folder
  with no capability behind it.
- **Clients.** The retired Pack's Python and shell clients for Home Assistant, the router and a DNS
  filter. They are archived in a private repository.

The 2026-10-24 reason, "describes one household", still holds for instance data. It does not hold
for a client that knows no house: that is external-system code, which a capability holds
(`on-placement`). Instance values stay in the overlay.

## What stays outside Sjel

A site whose host does not run Sjel keeps its own deploy repository: the host clones it, and
people other than Lars may administer it. Sjel reads such a site through its interfaces only: a
backup receipt (already read by `tools/doctor`) and the Home Assistant API.

## Proposal: one capability, `home`

Crates `home` and `home-api`, per the one-API-crate-per-capability decision (`PLAN.md`,
2026-10-25).

| Part | Holds |
|---|---|
| Site registry | One record per site in the overlay: name, `places` id, service URLs, credential item names |
| Site notes | `<overlay>/data/home/<site>/`, read and written through `home` |
| Home Assistant client | REST and websocket: entity list, state, service call |
| Entity check | Fails when a dashboard or automation YAML names an entity the instance lacks |
| Router client | Device list, WLAN state, drift against a stored baseline |
| DNS filter client | Status, top blocked domains, enable and disable |

Not ported: the automation template materializer and the dashboard generators. The template
layer drifted from the live automations once the materializer retired, and dashboards move to Home
Assistant's YAML mode, deployed from git.

**Data class.** Site notes name other people's devices and habits. They are `c2` by default, so
they reach a model only as `trusted_view` (`PLAN.md`, 2026-11-03).

## The always-on host

`PLAN.md` (2026-11-03) plans an always-on Mac mini. It runs Sjel, not Home Assistant, so it shares
little with a site host that runs containers. My estimate of what it can share: a nightly backup
with a receipt that doctor reads. Extract that when the Mac mini exists. Until then, `home` treats
it as a site with no Home Assistant.

## Order

1. `home` with the site registry and site notes. Move the parked notes folder into
   `data/home/<site>/`.
2. Home Assistant client and the entity check.
3. Router client.
4. DNS filter client.
5. 3D printing (Moonraker) as its own capability, only if a printer is in use.

## Open

- Name: `home` or `homes`. This plan assumes `home`, for the CLI verb `sjel home`.
- Item 5 waits on whether a printer is still in use.

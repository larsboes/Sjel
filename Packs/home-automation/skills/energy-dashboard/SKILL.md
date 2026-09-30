---
name: energy-dashboard
description: Configures the Home Assistant Energy Dashboard over the WebSocket API (energy/get_prefs, energy/save_prefs) — the REST API does not expose these prefs — wiring solar production/power, grid import/export, and battery charge/discharge sources, and reading the current sources back. Source entities are passed as flags or a raw JSON array, never hardcoded; existing grid price/cost config and the solar Forecast.Solar entry are preserved on write, so re-running is idempotent. The secret HA_TOKEN comes from a Bitwarden item at runtime and the non-secret HA_URL from the committed overlay config, never hardcoded. Use when wiring up or changing the Energy Dashboard sources, pointing the dashboard at solar/grid/battery sensors, or inspecting the current energy preferences. Do not use for general entity-state queries, service calls, or automation reloads (use ha-cli), or filling and deploying HA automation templates (use homectl).
allowed-tools: Bash
---

# energy-dashboard

Configure the Home Assistant Energy Dashboard automatically. The dashboard's preferences
sit behind HA's **WebSocket** API (`energy/get_prefs` / `energy/save_prefs`) — the REST API
does not expose them — so this is a pure-Python-3 stdlib tool with a minimal built-in
WebSocket client (no `websockets` dependency). Two commands: `show` (read the current
sources) and `set` (write sources built from flags). Credentials come from a Bitwarden item
at runtime, never hardcoded; source entities are always parameters, so nothing is pinned to
one house.

## Setup (once)

1. **In Home Assistant:** create a long-lived access token (Profile → Security → *Long-lived
   access tokens*). That token is `HA_TOKEN`.
2. **Non-secret config:** set `HA_URL` (e.g. `http://<host>:8123`) in the axon-overlay overlay's
   `config/home-assistant.vars` (resolved via `$SJEL_HOME_ROOT`). The WebSocket endpoint is
   derived from it (http→ws, https→wss, `/api/websocket`).
3. **In Bitwarden:** item `home-assistant/ha` (override with `$HA_BW_ITEM`) holding **only** the
   secret custom field `HA_TOKEN` — the **same item the `ha-cli` skill uses**. Provision via Sjel
   `setup-secret.sh`.
4. **Point `bw` at the vault once:** `bw config server <vault-url>`.

## Run

Unlock Bitwarden interactively, then call (the tool reads `$BW_SESSION`, never the master password):

```bash
export BW_SESSION=$(bw unlock --raw)

# Read the current sources
scripts/energy-dashboard show
scripts/energy-dashboard show -j            # raw prefs JSON

# Configure sources (each source is added only if its entity is given)
scripts/energy-dashboard set \
  --solar-production sensor.solar_lifetime_energy \
  --solar-power      sensor.solar_power \
  --grid-import      sensor.grid_import_energy \
  --grid-export      sensor.grid_export_energy \
  --battery-from     sensor.battery_discharge_energy \
  --battery-to       sensor.battery_charge_energy

# Advanced: supply the whole energy_sources array as raw JSON
scripts/energy-dashboard set --sources-file ./sources.json
```

## Gotchas

- **`$BW_SESSION` must be set and unlocked.** The tool refuses to run without it rather than
  hanging on a master-password prompt. `bw unlock --raw` on bitwarden-cli **v2026.2.0** can emit
  an empty string in non-interactive shells (upstream issue #19649) — pin a known-good `bw` if you
  get an empty session. A self-signed Vaultwarden cert needs `export NODE_EXTRA_CA_CERTS=/path/to/ca.pem`
  before `bw`, or `bw` itself hangs before this tool is even reached.
- **Credentials never print.** A missing secret is reported by *name* (`HA_TOKEN`), never by
  value; a missing/placeholder `HA_URL` is reported against the vars file. If a fetch looks wrong,
  check the vars key and the bw field name match exactly.
- **Energy prefs are WebSocket-only.** `ha-cli`'s REST API can't read or write them — that's why
  this skill exists as a separate tool speaking the WS protocol directly.
- **`set` is idempotent and preserves UI state.** It reads current prefs first, then overlays the
  existing grid price/cost fields and the solar Forecast.Solar entry onto the new sources — a write
  never wipes what was set in the UI. Re-running with the same flags is a no-op in effect.
- **A source is only written if its entity flag is given.** Running `set --solar-production …` with
  no grid/battery flags writes *only* a solar source and drops any previously-configured grid/battery
  sources — pass every source you want to keep, or use `--sources-file` for full control.
- **`battery` needs both legs.** `--battery-from` (discharge) and `--battery-to` (charge) are required
  together; giving one without the other is an error.
- **Entity IDs aren't validated against the instance.** A typo'd sensor is accepted by
  `energy/save_prefs` and simply shows no data on the dashboard — cross-check names with
  `ha-cli get <entity>` first.

## Examples

**"What's the Energy Dashboard pointing at right now?"**
→ `energy-dashboard show` → the solar/grid/battery source table, or "(no energy sources configured)".

**"Wire the dashboard to my solar + grid sensors."**
→ `energy-dashboard set --solar-production sensor.solar_lifetime_energy --grid-import sensor.grid_import_energy --grid-export sensor.grid_export_energy`
→ `Energy Dashboard configured:` + the written sources, price config preserved.

**"Restore a full source layout from a saved file."**
→ `energy-dashboard set --sources-file ./sources.json` → writes the raw `energy_sources` array verbatim.

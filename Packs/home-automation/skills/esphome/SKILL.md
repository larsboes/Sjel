---
name: esphome
description: Builds and OTA-flashes ESPHome firmware nodes through the esphome Docker container on a home-automation host — lists node YAMLs, compiles, uploads over WiFi, and streams device logs. Reaches the container over SSH (key auth) and the dashboard for a reachability check, with connection facts read from the committed overlay config at runtime, never hardcoded. Use when flashing an ESP32 or ESP8266, compiling an ESPHome node, running an OTA firmware update, tailing ESPHome logs, or adding a new sensor node. Do not use for Home Assistant automation-template materialization (use homectl), Fritz!Box network queries (use fritz), or general Docker container management.
allowed-tools: Bash
---

# esphome

Build and OTA-flash ESPHome nodes through the containerized `esphome` CLI on the home-automation
host. `espctl` is a thin wrapper: node builds/flashes run via `docker exec` over SSH; the dashboard
is polled directly only for a quick `version` reachability check. Node YAMLs live on the host in the
directory bind-mounted to `/config` in the container.

## Container assumption

This drives a Docker container named **`esphome`** (override with `$ESPHOME_CONTAINER`) — typically
a service in the host's `docker-compose`/`compose.yaml`, with the host's ESPHome config dir mounted
to `/config`. Every command shells `docker exec <container> esphome …` over SSH; there is no bare
local ESPHome install in the loop. The dashboard normally listens on `:6052`. For OTA to work, the
ESPHome service must bind the LAN IP (`--address` in the compose service), and exposure of `:6052`
should be restricted to the LAN.

## Setup (once)

1. **On the host:** run the `esphome` container (compose service named `esphome`) with the config dir
   mounted to `/config`, and ensure key-based SSH access to the host (the tool uses `BatchMode=yes` —
   no password prompt).
2. **In the overlay config:** the connection facts live in the committed overlay at
   `<overlay>/config/home-assistant.vars` (path via `$SJEL_HOME_ROOT`, default
   the active deployment overlay): `ESPHOME_SSH` (an ssh target such as `user@host`), `ESPHOME_URL` (the
   dashboard base URL, e.g. `http://host:6052`), and `ESPHOME_CONTAINER` (the container name). All three
   are non-secret and committed there; each can be overridden per-run via the matching `$ESPHOME_*` env
   var (env wins over the config file).
3. **No secret for this skill.** It reaches the container over SSH with **key auth** — there is no
   password or token, so there is no Bitwarden item to provision. (If a secret is ever needed later, add
   a bw fetch then; today there is none.)

## Run

The tool reads its connection facts from the overlay config — no unlock step:

```bash
scripts/espctl version          # dashboard reachability
scripts/espctl nodes            # list node YAMLs in the container
scripts/espctl compile bedroom  # compile bedroom.yaml (no flash)
scripts/espctl upload bedroom   # OTA-flash bedroom.yaml over WiFi
scripts/espctl logs bedroom     # stream logs (Ctrl-C to stop)
scripts/espctl run bedroom      # compile + upload + tail logs
```

## Workflow for a new node

1. Add `<node>.yaml` (plus a `secrets.yaml` on the host, not committed) to the host's ESPHome config dir.
2. Sync it to the host / into the `/config` mount.
3. **First flash is over USB** on the machine the device is plugged into; subsequent flashes are OTA
   via `upload`. `compile`/`upload` accept the node name with or without the `.yaml` suffix.

## Gotchas

- **The `esphome` container must be running** and reachable via `docker exec` on the SSH host — `nodes`
  returns an empty list (not an error) when the container is up but no non-`secrets` YAMLs exist yet.
- **Connection facts come from the overlay config.** If a command dies with a missing/placeholder error,
  fill `ESPHOME_SSH` / `ESPHOME_URL` / `ESPHOME_CONTAINER` in `<overlay>/config/home-assistant.vars`
  (or set the matching `$ESPHOME_*` env var). A `TODO-` placeholder is treated as unset.
- **No secret / no `bw`.** This skill uses SSH key auth, so there is no Bitwarden item and no `$BW_SESSION`
  to unlock — SSH key access to the host is the only credential.
- **OTA needs the LAN IP bound.** If `upload` cannot reach the device, confirm the ESPHome service binds the
  LAN IP (`--address` in the compose service) rather than a container-internal address.
- **`compile` can take minutes** on a cold build (SSH command timeout is 900s); `logs`/`run` stream until
  Ctrl-C.

## Examples

**"Flash the bedroom node over the air."**
→ `espctl upload bedroom` → compiles then OTA-flashes `bedroom.yaml`; tail with `espctl logs bedroom`.

**"Is the ESPHome dashboard up?"**
→ `espctl version` → prints `dashboard: <version>` or a clear unreachable error at `ESPHOME_URL`.

**"What nodes are configured?"**
→ `espctl nodes` → lists the `*.yaml` node files in the container's `/config` (excluding `secrets`).

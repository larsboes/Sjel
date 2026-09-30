---
name: ha-dashboard
description: Deploys and backs up Home Assistant Lovelace dashboards over the WebSocket API — list existing dashboards, back up a dashboard's config to a file, push a config file to a dashboard, and create a new sidebar dashboard. Pure-Python-3 stdlib WebSocket client (no deps); the non-secret HA_URL comes from the committed overlay config and the secret HA_TOKEN from a Bitwarden item at runtime, never hardcoded. Use when deploying or backing up a Lovelace dashboard, saving or restoring dashboard config, listing dashboards, or creating a new dashboard. Do not use for querying or controlling entities or calling services (use ha-cli), router or network device listing (use fritz), DNS or ad-blocking (use pihole), or filling and deploying HA automation templates (use homectl).
allowed-tools: Bash
---

# ha-dashboard

Deploy and back up Home Assistant Lovelace dashboards over the WebSocket API. Lovelace
dashboard config lives behind HA's **WebSocket** API, not its REST API, so this is a
pure-Python-3 stdlib WebSocket client (`socket` + `ssl` + `base64` + `hashlib`) — no
`websockets` package, no pip install. Commands: `list`, `backup`, `deploy`, `create`.
The non-secret `HA_URL` comes from the committed overlay config; the secret `HA_TOKEN`
comes from a Bitwarden item at runtime, never hardcoded.

## Setup (once)

1. **In Home Assistant:** create a long-lived access token (Profile → Security → *Long-lived
   access tokens*). That token is `HA_TOKEN`.
2. **In the overlay config:** set `HA_URL` (e.g. `http://<host>:8123`, or an `https://`
   reverse-proxy URL) in `<overlay>/config/home-assistant.vars` (override the root with
   `$SJEL_HOME_ROOT`). The WebSocket URL is derived from it.
3. **In Bitwarden:** an item `home-assistant/ha` (override with `$HA_BW_ITEM`) holding only the
   secret field `HA_TOKEN` — the same item ha-cli uses. Provision via Sjel `setup-secret.sh`.
4. **Point `bw` at the vault once:** `bw config server <vault-url>`.

## Run

Unlock Bitwarden interactively, then call (the tool reads `$BW_SESSION`, never the master password):

```bash
export BW_SESSION=$(bw unlock --raw)

# Inspect
scripts/ha-dashboard list                       # every named dashboard + the default
scripts/ha-dashboard list -j                     # JSON

# Back up before overwriting (default dashboard, or a named one)
scripts/ha-dashboard backup                       # -> default_dashboard_<ts>.json
scripts/ha-dashboard backup --url-path <slug>     # -> <slug>_dashboard_<ts>.json
scripts/ha-dashboard backup --url-path <slug> -o mydash.json

# Deploy a config file (JSON always; YAML only if PyYAML happens to be installed)
scripts/ha-dashboard deploy mydash.json --url-path <slug>
scripts/ha-dashboard deploy overview.json          # omit --url-path = default dashboard

# Create a new sidebar dashboard, then deploy into it
scripts/ha-dashboard create <slug> --title "My Dashboard" --icon mdi:home
```

Omit `--url-path` to target the default (Overview) dashboard; pass a slug for a named one.

## Gotchas

- **`$BW_SESSION` must be set and unlocked.** The tool refuses to run without it rather than hanging on
  a master-password prompt. `bw unlock --raw` on bitwarden-cli **v2026.2.0** can emit an empty string in
  non-interactive shells (upstream issue #19649) — pin a known-good `bw` if you get an empty session.
  A self-signed Vaultwarden cert needs `export NODE_EXTRA_CA_CERTS=/path/to/ca.pem` before `bw`, or `bw`
  itself hangs before this tool is even reached.
- **Credentials never print.** A missing/placeholder `HA_URL` in the overlay config, or a missing
  `HA_TOKEN` bw field, is reported by *name*, never by value. If a fetch looks wrong, check the
  `home-assistant.vars` key and the bw item's field name match exactly.
- **Always `backup` before `deploy`.** `lovelace/config/save` overwrites the whole dashboard in one shot —
  there is no merge and no undo. Back up first; the JSON dump restores it verbatim.
- **`deploy` targets an existing dashboard; it does not create one.** Deploying to a slug that doesn't
  exist yet fails — run `ha-dashboard create <slug> --title …` first (or make it in Settings → Dashboards),
  then deploy. The default dashboard (no `--url-path`) always exists.
- **Config format is JSON by design.** The tool is dep-free, so it reads/writes JSON natively. HA authors
  dashboards as YAML in docs, but the WS API exchanges JSON objects either way — `backup` gives you JSON,
  and `deploy` takes JSON. `.yaml`/`.yml` files work only if PyYAML is already importable; otherwise
  convert to JSON. Round-tripping backup → deploy needs no YAML.
- **`https://` HA URLs use verified TLS.** A self-signed HA cert makes the WebSocket handshake fail cert
  verification. Fix the trust chain or point `SSL_CERT_FILE` at the trusted CA certificate for that call
  (plain `http://` on a trusted private network needs neither).

## Examples

**"What dashboards exist?"**
→ `ha-dashboard list` → slug / title / mode per named dashboard, plus the always-present default.

**"Back up the energy dashboard before I edit it."**
→ `ha-dashboard backup --url-path energie -o energie.json` → verbatim JSON config on disk.

**"Roll it back."**
→ `ha-dashboard deploy energie.json --url-path energie` → the saved config is pushed back as-is.

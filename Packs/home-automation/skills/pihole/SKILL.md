---
name: pihole
description: Controls a Pi-hole v6 DNS resolver over its REST API — query stats, per-device egress (what each client phones home to), top domains/clients, pause or resume blocking, and add allow/deny rules. The Pi-hole host and admin password come from a Bitwarden item at runtime, never hardcoded. Use when the user wants Pi-hole DNS stats, per-device egress visibility, top domains or clients, blocked-query counts, ad-blocking pause/resume, or an allowlist/denylist entry. Do not use for router device listing (use fritz), Home Assistant entity control (use homectl), or non-Pi-hole DNS servers.
allowed-tools: Bash
---

# pihole

Drive the home Pi-hole over its **v6 REST API** (`/api/...`, auth via `X-FTL-SID`). Pi-hole logs
every DNS query, so it answers "what does this device talk to on the internet" — per-device egress,
the payoff after isolating IoT onto its own VLAN. Pure Python 3 stdlib, no deps: `python3` on the
host is enough.

## Setup (once)

1. **In the overlay config (non-secret):** set `PIHOLE_HOST` (base URL/host of the Pi-hole; scheme +
   `:port` optional) in `<overlay>/config/home-assistant.vars` (override its root with
   `$SJEL_HOME_ROOT`). The tool fails cleanly, naming a missing/`TODO-` key, until it's filled in.
2. **In Bitwarden (secret only):** an item `home-assistant/pihole` (override with `$PIHOLE_BW_ITEM`)
   with the custom field `PIHOLE_PASSWORD` (the admin/API password). Provision via Sjel
   `setup-secret.sh`. The item may not exist yet — the tool fails cleanly, naming the missing field,
   until it does.
3. **Point `bw` at the vault once:** `bw config server <vault-url>`.
4. **On the network:** the Fritz!Box DHCP DNS must point at the Pi for whole-house coverage, and
   Pi-hole must run in `network_mode: host` so per-client source IPs survive (Docker NAT would hide
   them and break per-device egress).

## Run

Unlock Bitwarden interactively, then call (the tool reads `$BW_SESSION`, never the master password):

```bash
export BW_SESSION=$(bw unlock --raw)
scripts/pihole status                       # overview + blocking state
scripts/pihole egress <device-ip>           # what one device phones home to
scripts/pihole top-domains 20
scripts/pihole top-clients
scripts/pihole block-off 300                # pause blocking 5 min
scripts/pihole block-on
scripts/pihole deny telemetry.example.com
scripts/pihole allow updates.example.com
```

`scripts/pihole --help` (or `<command> --help`) lists every command and its args.

## Egress allow-listing (turn isolation into containment)

After isolating an IoT device, lock it to only its needed vendor domains: review `egress <ip>`,
then `deny` everything else (or build a per-device allowlist in the UI). A compromised camera then
can't exfiltrate or pivot — DNS for anything but its vendor is refused.

## Gotchas

- **`$BW_SESSION` must be set and unlocked.** The tool refuses to run without it rather than hanging
  on a master-password prompt. `bw unlock --raw` on bitwarden-cli **v2026.2.0** can emit an empty
  string in non-interactive shells (upstream issue #19649) — pin a known-good `bw` if the session
  comes back empty.
- **Vaultwarden with a self-signed cert:** `export NODE_EXTRA_CA_CERTS=/path/to/ca.pem` before `bw`,
  or `bw` itself hangs before this tool is even reached.
- **Credentials never print.** To debug the fetch, check that `PIHOLE_HOST` is set in
  `home-assistant.vars` and the bw item's field *name* matches `PIHOLE_PASSWORD` exactly — a missing
  key/field is reported by name, never by value.
- **Sessions expire.** The tool re-auths per invocation (`POST /api/auth` → `sid` → `X-FTL-SID`);
  no session state is cached across runs.
- **Query `status` tells reached-vs-blocked.** `GRAVITY`/`DENYLIST`/`REGEX` = blocked; `FORWARDED`/
  `CACHE`/`ALLOWLIST` = the device actually reached that domain. Anything not blocked is real egress.

## Examples

**"What is this IoT device phoning home to?"**
→ `pihole egress <device-ip>` → recent `client / status / domain` rows → scan the non-blocked ones
for unexpected vendors/telemetry.

**"Is ad-blocking on, and how much is it catching?"**
→ `pihole status` → `blocking: true/false` + today's blocked count and percentage.

**"Pause blocking for a flaky app, then re-enable."**
→ `pihole block-off 300` (5 min) … `pihole block-on`.

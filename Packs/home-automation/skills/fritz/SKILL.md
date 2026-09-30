---
name: fritz
description: >-
  Queries an AVM Fritz!Box (7590/FRITZ!OS) read-only over two transports — TR-064 via
  fritzconnection for the authoritative host table, and the AVM JSON API (login_sid.lua +
  data.lua) for what TR-064 cannot see: active channel and width per band, DFS exposure, mesh
  topology with per-AP client counts, the timestamped WLAN event log, and subsystem load.
  Credentials come from a Bitwarden item at runtime, never hardcoded. Use when listing network
  devices, checking what is on the network, Fritz!Box device discovery, WLAN clients, guest-WiFi
  status, spotting an unknown device, or diagnosing a slow or unstable WLAN (channel width,
  DFS/radar eviction, repeater backhaul, airtime saturation). Do not use for Home Assistant
  entity control (use ha-cli), DNS or ad-blocking (use pihole), local subnet scanning and drift
  (use netmon), or non-Fritz routers.
allowed-tools: Bash
---

# fritz

Read the Fritz!Box's own view of the network — cleaner and more complete than an ARP scan (real
hostnames, connection type, active/inactive), and the only place the radio-level truth lives.
Two transports in one tool:

- **TR-064** (`fritzconnection`) — the authoritative host table and guest-WLAN state.
- **AVM JSON API** (`login_sid.lua` + `data.lua`, stdlib only) — channel, width, DFS exposure,
  mesh topology, event log, subsystem load. **A slow-WLAN diagnosis needs this one.**

`uv run` handles the dep; the host needs `uv` (one binary). **Read-only by contract** — every
request is a GET or an `xhr=1` fetch, and no subcommand may be added that writes configuration.

## Setup (once)

1. **On the Fritz!Box:** enable TR-064 (Heimnetz → Netzwerk → Netzwerkeinstellungen → *"Zugriff für Apps
   erlauben"*), and create a dedicated user (System → FRITZ!Box-Benutzer) with the *"FRITZ!Box Einstellungen"*
   right. Use that user, not the box password.
2. **In Bitwarden:** an item named by `FRITZ_BW_ITEM` (default `home-assistant/fritz`) carrying the
   password — either as a custom field `FRITZBOX_PASSWORD` or in the item's own password slot.
   Provision via Sjel `setup-secret.sh`.
3. **In the overlay config** (`<overlay>/config/home-assistant.vars`): `FRITZBOX_HOST`,
   `FRITZBOX_USERNAME`, and optionally `FRITZ_BW_ITEM`. An env var of the same name wins per key.
4. **Point `bw` at the vault once:** `bw config server <vault-url>`.

## Run

Unlock Bitwarden interactively, then call (the tool reads `$BW_SESSION`, never the master password):

```bash
export BW_SESSION=$(bw unlock --raw)

# TR-064 — the authoritative host table
scripts/fritz devices          # active hosts: ip / mac / interface / active / name
scripts/fritz devices --all -j # include inactive, JSON
scripts/fritz guest            # guest-WLAN enabled?

# JSON API — what TR-064 cannot tell you
scripts/fritz overview         # model, uptime, device count, unmeshed flag, subsystem load
scripts/fritz chan             # active channel + width per band, and whether that sits in DFS
scripts/fritz mesh             # which AP serves how many clients, and how it backhauls
scripts/fritz log -f wlan      # timestamped radar / DFS / channel-change events
scripts/fritz wlan --detail    # every WLAN client and its serving AP
scripts/fritz api <page>       # raw data.lua page — the escape hatch
```

**Diagnosing "the WiFi is slow" — four commands, in this order:** `overview` (is a subsystem
saturated?) → `chan` (is 5 GHz sitting in DFS, and how wide?) → `log -f wlan` (did something
happen, and when?) → `mesh` (how much traffic crosses the air twice?).

## Gotchas

- **`page=overview` returns HTML, not JSON, on FRITZ!OS 8.25.** The overview payload lives at
  `page=wStat`. Page names drift between firmware versions — that is what `fritz api <page>` is
  for, and why the opinionated subcommands name their source page in the output.
- **Ping is not a liveness test for a repeater.** A FRITZ!Repeater can serve a dozen clients
  happily while returning 100 % packet loss to ICMP. Use `fritz mesh`, not `ping`.
- **A connected repeater is not a meshed repeater, and the difference is the whole ballgame.**
  An unmeshed repeater broadcasts the house SSIDs on the house channels while the box has no
  control over it. Clients then ping-pong between it and the box, dropping on every jump.
  Check `mesh --raw | grep -c mesh_repeater_no_trusted` (must be 0) and whether every AP
  appears in `mesh` with the box as parent. **The tell is an AP that is powered and serving
  zero clients:** the box steers nobody to an AP it does not trust, so the repeater burns
  airtime and returns nothing. Fix it before investigating channels, width or DFS — those
  investigations are expensive and, in the one case measured here, were all wrong.
- **The client-side signature of an unmeshed second AP** is RSSI stepping 15–20 dB back and
  forth on an unchanged channel and SSID, roughly once a minute, each step with a loss window.
  That is two radios, not fading. `netmon/wlanwatch` catches it; a spot check never will.
- **TR-064 must be enabled on the box** and the user needs the *"FRITZ!Box Einstellungen"* right — without it
  `get_hosts_info()` raises an auth/permission error, not an empty list.
- **The box lists its own valid usernames, unauthenticated:** `curl -s "http://<host>/login_sid.lua?version=2"`
  returns a `<Users>` block. Check there before assuming a login name — it costs nothing and
  avoids burning failed attempts against the box's rate limiter.
- **Failed logins are rate-limited.** `<BlockTime>` in the same response counts the lockout down;
  the tool refuses to attempt a login while it is non-zero rather than deepening the block.
- **`$BW_SESSION` must be set and unlocked.** An unlocked Bitwarden **desktop app does not unlock
  the CLI** — they hold separate vault state, and `bw status` will keep saying `locked`. The tool
  refuses to run without a session rather than hanging on a master-password prompt.
- **`bw` serves a stale cache until you `bw sync`.** An item edited in the web vault or another
  client reads as empty here until then; check `lastSync` in `bw status` before believing a
  missing field.
- **`bw unlock --raw` on bitwarden-cli v2026.2.0** can emit an empty string in non-interactive
  shells (upstream issue #19649) — pin a known-good `bw` if you get an empty session. Note it
  also cannot be driven from a non-TTY: run it in a real terminal and pass the token onward.
- **Vaultwarden with a self-signed cert:** `export NODE_EXTRA_CA_CERTS=/path/to/ca.pem` before `bw`, or `bw`
  itself hangs before this tool is even reached.
- **`interface_type` is the LAN/WLAN discriminator** in `get_hosts_info()` (not a separate field); `status`
  (bool) is active/inactive. This is the discovery signal ARP can't give you (raw MACs, no names).
- **Per-client RSSI/MCS/rate is not exposed by `data.lua` on 8.25.** The association and its
  serving AP are; the radio numbers are not. Read those client-side
  (`system_profiler SPAirPortDataType` on macOS) or from the box's support file.
- **Values in the vars file may carry inline `#` comments.** The parser ends a quoted value at
  its closing quote and an unquoted one at ` #`; naive quote-stripping swallows the comment into
  the value and surfaces much later as an unexplained auth failure.
- **Credentials never print.** A missing field is reported by name, never by value.

## Examples

**"What's on my network / any unknown devices?"**
→ `fritz devices --all` → full host table (name, IP, MAC, WLAN vs LAN, active) → scan for unnamed/unexpected MACs.

**"Why is the WiFi slow?"** — in this order, cheapest and most-often-right first.
→ `fritz mesh` + `mesh --raw | grep -c mesh_repeater_no_trusted` → **start here.** Every AP a
mesh member with the box as parent? An unmeshed one is both a ping-pong source and an airtime
parasite, and re-pairing it is two button presses.
→ `fritz mesh` again for topology → clients behind a wirelessly-backhauled repeater spend their
traffic twice on the air, and worse if the repeater relays on the box's own channel.
→ `fritz log -f wlan` → dates the event. Chronic misconfiguration cannot explain "it worked yesterday".
→ `fritz wlan --detail` → the slowest clients hold the medium longest per byte. Hunt those next.
→ `fritz chan` → **last.** 160 MHz on a DFS channel means the network can be evicted by radar;
the non-DFS block is only 80 MHz wide, so 160 MHz and DFS-free cannot both hold. True, and
still the least likely thing to be causing today's complaint.

> `fritz overview` reports a per-subsystem **energy** share, not utilisation. "WLAN 100 %" means
> the radios are powered, not that they are saturated. It cost a session to learn that; do not
> cite it as congestion.

**"Is the guest WiFi on?"**
→ `fritz guest` → `Guest WLAN: enabled|disabled`.

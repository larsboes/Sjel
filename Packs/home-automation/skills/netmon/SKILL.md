---
name: netmon
description: Monitors a home LAN and its traffic from the local host — active host discovery with MAC and vendor, per-host open-port scans, a saved baseline plus drift diff (new/missing devices), WAN throughput totals, and DNS-egress visibility (which domains each device phones home to) via the resolver's query log. Subnet is auto-derived from the primary interface or set by env/flag; Fritz!Box throughput credentials come from a Bitwarden item at runtime, never hardcoded. Use when scanning who is on the network, spotting a new or unknown device, checking open ports, watching bandwidth or WAN throughput, auditing what a device phones home to, or baselining and diffing the network. Do not use for the router's own authoritative device list (use fritz), or filling and deploying Home Assistant automation templates (use homectl).
allowed-tools: Bash
---

# netmon

Network visibility from wherever you run it — especially after isolating IoT onto its own VLAN.
`scan`/`ports`/`baseline`/`diff` need only `nmap` + the Python stdlib and no credentials.
`traffic` reads the Fritz!Box over TR-064 (credentials from Bitwarden at runtime).
`dns`/`egress` read a resolver's query log over SSH. Nothing is pinned to one house.

## Configuration

Non-secret config lives in the committed axon-overlay overlay (`config/home-assistant.vars`,
resolved via `$SJEL_HOME_ROOT`, falling back to the active deployment overlay). The one secret comes from
Bitwarden at runtime.

| What | Source | Default |
|------|--------|---------|
| Subnet to scan | `--subnet` / `$NETMON_SUBNET` / `NETMON_SUBNET` in `home-assistant.vars` | auto-derived from the primary interface as a `/24` |
| SSH target (dns/egress) | `$NETMON_SSH` / `NETMON_SSH` in `home-assistant.vars` | none — required for `dns`/`egress` |
| Resolver container | `$NETMON_DNS_CONTAINER` | `dnsmasq` |
| Fritz!Box host + user (traffic) | `FRITZBOX_HOST` / `FRITZBOX_USERNAME` in `home-assistant.vars` | none — required for `traffic` |
| Fritz!Box password (traffic) | Bitwarden item `$NETMON_BW_ITEM`, secret field `FRITZBOX_PASSWORD` | `home-assistant/netmon` |

For subnet and SSH the env var wins if set, else the overlay config, else (subnet only)
auto-derivation; the `--subnet` flag overrides everything. The bw item holds **only** the secret
`FRITZBOX_PASSWORD` — the non-secret `FRITZBOX_HOST`/`FRITZBOX_USERNAME` live in `home-assistant.vars`
(same split and shape as the `fritz` skill). Provision the password via Sjel `setup-secret.sh`.

## Run

```bash
# Inventory + drift (no credentials)
scripts/netmon scan                       # live hosts + MAC + vendor
scripts/netmon ports 10.0.0.45            # open ports on one host (default --top-ports 100)
scripts/netmon baseline                    # save the known-good snapshot
scripts/netmon diff                        # new / missing hosts since baseline

# WAN throughput (needs bw + fritzconnection)
export BW_SESSION=$(bw unlock --raw)
scripts/netmon traffic

# DNS egress (needs $NETMON_SSH)
export NETMON_SSH=user@resolver-host
scripts/netmon dns                         # top domains, all clients
scripts/netmon egress 10.0.0.128           # one device's external domains
```

## Optional enrichment

```bash
pip install --break-system-packages fritzconnection mac-vendor-lookup scapy
```
- `fritzconnection` → required for `traffic` (live WAN rate + totals via TR-064).
- `mac-vendor-lookup` → full OUI→vendor names (else a small built-in prefix map).
- `scapy` → `netmon passive [--secs N]` (mDNS/SSDP/ARP listen; needs sudo).

## DNS egress visibility — prerequisites

`dns`/`egress` parse the resolver's query log. For **complete** coverage:
1. Enable query logging on the resolver (dnsmasq: `log-queries` + `log-facility=-`).
2. Point the router's DHCP-advertised DNS server at that resolver so every device resolves
   through it (or deploy Pi-hole for a UI + per-client stats + egress allow/deny lists).

Without step 2 you only see devices already using that resolver.

## Gotchas

- **`nmap` is a hard prerequisite for scan/ports/baseline/diff and is NOT installed on the target
  Pi.** Install it first (`sudo apt install nmap`, or `brew install nmap` on macOS) — the tool
  fails with an explicit install hint rather than a stack trace. There is no full-featured pure-arp
  fallback: without `nmap` there is no active discovery. The ARP cache only *enriches* an nmap scan
  (fills MACs nmap saw without sudo); it is not a substitute for the scan itself.
- **`nmap -sn` resolves MAC vendors only with `sudo`.** Without sudo, MACs and vendors come from the
  ARP cache + the built-in OUI map, so some rows show a blank vendor.
- **macOS `arp` strips leading zeros in MAC octets** (`0:17:88…`) — `netmon` re-pads them
  (`norm_mac`) so vendor lookup still matches.
- **Subnet is auto-derived as a `/24`** from the primary interface. Non-/24 networks need
  `$NETMON_SUBNET`, `--subnet`, or `NETMON_SUBNET` in `home-assistant.vars` set explicitly.
- **`traffic` needs an unlocked `$BW_SESSION`** (`bw unlock --raw`) — credentials are fetched at
  runtime and never printed. Same Bitwarden caveats as `fritz` (self-signed Vaultwarden →
  `NODE_EXTRA_CA_CERTS`; a possibly-empty session from bitwarden-cli v2026.2.0).
- **Baseline lives at `~/.cache/netmon/baseline.json`** — outside any repo, so it is never committed.

## Examples

**"Who's on my network / any new devices?"**
→ `netmon scan` for the live list; `netmon baseline` once, then `netmon diff` later to surface
new (`+`) and missing (`-`) hosts against the known-good snapshot.

**"What is this IoT device phoning home to?"**
→ `export NETMON_SSH=user@resolver-host` → `netmon egress <device-ip>` → ranked external domains
that device queried (needs resolver query logging, see prerequisites above).

**"How much WAN traffic right now?"**
→ `export BW_SESSION=$(bw unlock --raw)` → `netmon traffic` → live down/up Mbit/s + lifetime GB totals.

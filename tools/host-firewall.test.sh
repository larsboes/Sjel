#!/bin/bash
# capabilities/host-firewall — what the renderer produces from declared inputs (Axon#193).
#
# The render layer is the whole of what this repository can honestly test: applying rules to a
# real host is the overlay's work by the issue's own boundary, and this machine is a Mac with
# no nftables. So these cases assert the ruleset's CONTENT and ORDER, which is where the real
# defects live — an ICMPv6 rule missing, or an accept below the drop that never runs.
#
# Every address here is from RFC 5737 / RFC 3849 documentation space. Unroutable by design, so
# no fixture in this file can describe, or leak, a real network.
set -uo pipefail

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
FW="$(cd "$_here/.." && pwd)/capabilities/host-firewall/host-firewall"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
check() {
  if [ "$2" = "$3" ]; then printf '  ✓ %s\n' "$1"
  else printf '  ✗ %s\n      expected: %s\n      got:      %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
}

conf() { # conf <name> <lines...> -> path
  local p="$SCRATCH/$1.toml"; shift
  printf '%s\n' "$@" > "$p"
  printf '%s' "$p"
}

C="$(conf full \
  'wan_interface = "eth0"' \
  'trusted_interfaces = ["tailscale0"]' \
  'permitted_networks = ["192.0.2.0/24", "2001:db8::/32"]' \
  'permitted_ports = ["22/tcp", "53/udp"]' \
  'public_ports = ["443/tcp"]' \
  'allow_ping = true')"
OUT="$(SJEL_HOST_FIREWALL_CONFIG="$C" "$FW" render)"

echo "posture"
check "input policy is drop"   "1" "$(printf '%s' "$OUT" | grep -c 'hook input priority filter; policy drop')"
check "forward policy is drop" "1" "$(printf '%s' "$OUT" | grep -c 'hook forward priority filter; policy drop')"
check "output is accept, deliberately" "1" "$(printf '%s' "$OUT" | grep -c 'hook output  priority filter; policy accept')"
check "one inet table, so v4 and v6 cannot drift apart" "1" "$(printf '%s' "$OUT" | grep -c '^table inet axon_filter {')"

echo
echo "the rules that must exist"
check "established/related is accepted" "1" "$(printf '%s' "$OUT" | grep -c 'ct state established,related accept')"
check "invalid is dropped"              "1" "$(printf '%s' "$OUT" | grep -c 'ct state invalid drop')"
check "loopback is accepted"            "1" "$(printf '%s' "$OUT" | grep -c 'iif lo accept')"
# Dropping these does not present as a firewall problem — it presents as IPv6 being broken and
# as random stalls on both families. Asserted individually so a future edit cannot thin the set.
for t in nd-neighbor-solicit nd-neighbor-advert nd-router-advert packet-too-big; do
  check "icmpv6 $t is permitted" "1" "$(printf '%s' "$OUT" | grep -c "$t")"
done
check "icmpv4 keeps the PMTU-critical subset" "1" "$(printf '%s' "$OUT" | grep -c 'icmp   type { destination-unreachable, time-exceeded, parameter-problem } accept')"

echo
echo "ordering — where a correct-looking ruleset goes wrong"
# An accept below the drop never runs. The policy sits on the chain declaration, so what must
# hold is that every accept is inside the chain and after the ct-state rule that keeps the
# applying session alive.
ct_line="$(printf '%s' "$OUT" | grep -n 'ct state established,related' | cut -d: -f1)"
lo_line="$(printf '%s' "$OUT" | grep -n 'iif lo accept' | cut -d: -f1)"
port_line="$(printf '%s' "$OUT" | grep -n 'dport 22 accept' | head -1 | cut -d: -f1)"
check "established/related precedes loopback"      "yes" "$([ "$ct_line" -lt "$lo_line" ] && echo yes || echo no)"
check "established/related precedes the port rules" "yes" "$([ "$ct_line" -lt "$port_line" ] && echo yes || echo no)"

echo
echo "inputs are inputs, not literals"
check "the declared interface reaches the rules"  "yes" "$(printf '%s' "$OUT" | grep -q 'iifname "eth0"' && echo yes || echo no)"
check "the declared trusted interface is accepted" "1"  "$(printf '%s' "$OUT" | grep -c 'iifname "tailscale0" accept')"
check "an ipv4 source becomes an ip saddr rule"    "1"  "$(printf '%s' "$OUT" | grep -c 'ip  saddr 192.0.2.0/24 iifname "eth0" tcp dport 22 accept')"
check "an ipv6 source becomes an ip6 saddr rule"   "1"  "$(printf '%s' "$OUT" | grep -c 'ip6 saddr 2001:db8::/32 iifname "eth0" tcp dport 22 accept')"
check "udp keeps its protocol"                     "2"  "$(printf '%s' "$OUT" | grep -c 'udp dport 53 accept')"
check "a public port is source-unscoped"           "1"  "$(printf '%s' "$OUT" | grep -c 'iifname "eth0" tcp dport 443 accept')"

# The point of the whole capability: nothing about a machine is baked in.
check "no address is hardcoded in the renderer" "0" \
  "$(grep -cE '([0-9]{1,3}\.){3}[0-9]{1,3}' "$FW")"

echo
echo "the choices that must be visible"
C2="$(conf noping 'wan_interface = "eth0"' 'permitted_ports = ["22/tcp"]' 'allow_ping = false')"
OUT2="$(SJEL_HOST_FIREWALL_CONFIG="$C2" "$FW" render)"
check "allow_ping=false drops echo-request"        "0" "$(printf '%s' "$OUT2" | grep -c 'type echo-request accept')"
check "but keeps the required icmpv6 subset"       "1" "$(printf '%s' "$OUT2" | grep -c 'nd-neighbor-solicit')"
check "an unscoped port says so in the ruleset"    "1" "$(printf '%s' "$OUT2" | grep -c 'any source — permitted_networks is empty')"

out="$(SJEL_HOST_FIREWALL_CONFIG="$C2" "$FW" check 2>&1)"
case "$out" in *"reachable from any source"*) echo "  ✓ and check warns about it out loud" ;;
  *) echo "  ✗ check is silent about a port open to any source"; fails=$((fails + 1)) ;; esac
case "$out" in *"nft not installed"*|*"nft parses"*|*"check needs root"*) echo "  ✓ check states whether nft verified the syntax" ;;
  *) echo "  ✗ check does not say whether the ruleset was syntax-checked"; fails=$((fails + 1)) ;; esac

echo
echo "refusals"
SJEL_HOST_FIREWALL_CONFIG="$SCRATCH/absent.toml" "$FW" render >/dev/null 2>&1
check "a missing config refuses rather than rendering a default" "2" "$?"
C3="$(conf noiface 'permitted_ports = ["22/tcp"]')"
SJEL_HOST_FIREWALL_CONFIG="$C3" "$FW" render >/dev/null 2>&1
check "a config without wan_interface refuses" "2" "$?"
C4="$(conf badport 'wan_interface = "eth0"' 'permitted_ports = ["22/sctp", "http/tcp"]')"
SJEL_HOST_FIREWALL_CONFIG="$C4" "$FW" check >/dev/null 2>&1
check "a malformed port entry fails check" "1" "$?"

echo
if [ "$fails" -eq 0 ]; then echo "host firewall: all checks passed"
else echo "host firewall: $fails check(s) failed"; fi
exit "$fails"

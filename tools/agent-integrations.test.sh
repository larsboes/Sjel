#!/bin/bash
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$TOOLS_DIR/agent-integrations.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export PATH="/usr/bin:/bin:/usr/sbin:/sbin"
mkdir -p "$HOME/.claude"

list_output="$($SCRIPT list)"
case "$list_output" in
  *"interceptor"*"Configured on this machine: claude"*) ;;
  *) echo "list did not name interceptor and the configured harness" >&2; exit 1 ;;
esac

json="$($SCRIPT status --json)"
printf '%s' "$json" | python3 -c '
import json, sys
data = json.load(sys.stdin)
upstreams = [i["upstream"] for i in data["integrations"]]
assert upstreams == ["interceptor"], upstreams
row = data["integrations"][0]["harnesses"][0]
assert row["state"] == "missing", row
assert "graph_state" not in row, row
'

machine="$($SCRIPT status --machine)"; machine="${machine%%$'\n'*}"
fields="$(printf '%s' "$machine" | awk -F'|' '{print NF}')"
[ "$fields" = 14 ] || { echo "status --machine row has $fields fields, not 14: $machine" >&2; exit 1; }

if $SCRIPT install claude 2>"$tmp/err"; then
  echo "install without an upstream must fail" >&2
  exit 1
fi
grep -q 'usage: agent-integrations.sh install <upstream>' "$tmp/err"

if $SCRIPT update nothing 2>"$tmp/err"; then
  echo "update of an unknown upstream must fail" >&2
  exit 1
fi
grep -q "unknown upstream 'nothing'" "$tmp/err"

echo "agent-integrations.test.sh: ok"

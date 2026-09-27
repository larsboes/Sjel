#!/bin/bash
# Test for the machine-manifest resolution in tools/lib/paths.sh — which of an overlay's
# machines the running host is. An overlay describes a deployment, a deployment may own
# several machines, and picking the wrong manifest means reading another machine's
# enabled set, container runtime and state mounts.
#
# Each case is here because it fails silently otherwise: a missed hostname match reads
# the legacy file and looks fine, and an explicit name pointing at nothing would fall
# back to a different machine rather than stopping.
set -uo pipefail

SCRATCH="$(mktemp -d "/tmp/machine-resolution.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB_DIR=""
for _c in "$_dir/lib" "$_dir/tools/lib"; do
  if [ -f "$_c/paths.sh" ]; then LIB_DIR="$_c"; break; fi
done
if [ -z "$LIB_DIR" ]; then
  echo "machine resolution: cannot find paths.sh next to $_dir" >&2
  exit 1
fi

# This test's whole subject is which machine.toml paths.sh picks. An operator's exported
# SJEL_MACHINE_TOML/SJEL_OVERLAY_ROOT answers that question before the scratch root gets to,
# so the assertions measured the real machine (tools/lib/test-support.sh#isolate_axon_env).
source "$LIB_DIR/test-support.sh"
isolate_axon_env

mkdir -p "$ROOT/tools/lib" "$OVERLAY/config/machines"
cp "$LIB_DIR/paths.sh" "$LIB_DIR/toml.sh" "$ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"

HOST="$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo "")"
if [ -z "$HOST" ]; then
  echo "machine resolution: no hostname available, cannot run" >&2
  exit 1
fi

manifest() {  # <path>
  mkdir -p "$(dirname "$1")"
  printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = []\n' > "$1"
}

fails=0
# Each case runs paths.sh in its own shell: it exports and unsets, so re-sourcing it in
# one process would carry state between cases and hide exactly what is being tested.
resolved() {
  bash -c "source '$ROOT/tools/lib/paths.sh' >/dev/null 2>&1 && printf '%s' \"\$SJEL_MACHINE_TOML\"" 2>/dev/null
}
check() {  # check <description> <expected path>
  local desc="$1" want="$2" got
  got="$(resolved)"
  if [ "$got" != "$want" ]; then
    echo "FAIL: $desc"; echo "  got:  $got"; echo "  want: $want"; fails=$((fails + 1))
  fi
}

# 1. The original layout. An overlay that never grows a second machine keeps working.
manifest "$OVERLAY/config/machine.toml"
check "single-file layout resolves" "$OVERLAY/config/machine.toml"

# 2. A manifest named after this host wins over the legacy file.
manifest "$OVERLAY/config/machines/$HOST.toml"
check "hostname match wins over the legacy file" "$OVERLAY/config/machines/$HOST.toml"

# 3. An explicit name in axon.local.toml wins over the hostname, so a host can carry a
#    role name rather than whatever DNS calls it.
manifest "$OVERLAY/config/machines/service-node.toml"
printf 'overlay = "%s"\nmachine = "service-node"\n' "$OVERLAY" > "$ROOT/axon.local.toml"
check "explicit machine name wins over hostname" "$OVERLAY/config/machines/service-node.toml"

# 4. Naming a machine that does not exist must stop, not fall back: falling back would
#    silently operate on a different machine's enabled set.
printf 'overlay = "%s"\nmachine = "does-not-exist"\n' "$OVERLAY" > "$ROOT/axon.local.toml"
out="$(bash -c "source '$ROOT/tools/lib/paths.sh' 2>&1 >/dev/null; true")"
rc=0
bash -c "source '$ROOT/tools/lib/paths.sh' >/dev/null 2>&1" || rc=$?
if [ "$rc" -eq 0 ]; then
  echo "FAIL: naming a missing machine should fail, exited 0"; fails=$((fails + 1))
fi
if ! printf '%s' "$out" | grep -qF "does-not-exist"; then
  echo "FAIL: error message does not name the missing machine"; fails=$((fails + 1))
fi

# 5. With machines/ present but no match for this host, the legacy file still answers.
rm -f "$ROOT/axon.local.toml" "$OVERLAY/config/machines/$HOST.toml"
check "no hostname match falls back to the legacy file" "$OVERLAY/config/machine.toml"

if [ "$fails" -gt 0 ]; then
  echo "machine resolution: $fails check(s) failed"
  exit 1
fi
echo "machine resolution: all checks passed"

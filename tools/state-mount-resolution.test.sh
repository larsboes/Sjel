#!/bin/bash
# Tests for axon_state_mount_for in tools/lib/paths.sh, over planted overlays. What it replaced
# is scripts each carrying their own hardcoded path to a tool's state directory.
#
# Every case here is one that passes on the machine that wrote the fallback and fails on the
# next one. On this workstation the hardcoded path and the declared mount agreed only because
# somebody made a symlink in July — a coincidence, holding up a contract.
#
# Needs bun on PATH. These functions read machine.toml's [[state_mount]], an array-of-tables
# past tools/lib/toml.sh's single-line contract, so they go through Bun.TOML. Without bun this
# file fails hard rather than skipping every case and reporting green — the one failure mode a
# test suite cannot warn you about. tools/service-runner.test.sh holds the same line.
#
# Run: tools/state-mount-resolution.test.sh
set -uo pipefail

command -v bun >/dev/null 2>&1 || {
  echo "state-mount-resolution: bun not on PATH — cannot run (array-of-tables needs Bun.TOML)" >&2
  exit 1
}

SCRATCH="$(mktemp -d "/tmp/state-mount.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB_DIR=""
for _c in "$_dir/lib" "$_dir/tools/lib"; do
  if [ -f "$_c/paths.sh" ]; then LIB_DIR="$_c"; break; fi
done
[ -n "$LIB_DIR" ] || { echo "state-mount-resolution: cannot find paths.sh next to $_dir" >&2; exit 1; }

# Every case below plants a machine.toml and asks which [[state_mount]] resolves. An operator's
# exported SJEL_MACHINE_TOML points that lookup at the real one instead of the planted one
# (tools/lib/test-support.sh#isolate_axon_env).
source "$LIB_DIR/test-support.sh"
isolate_axon_env

fails=0

# plant <mounts-toml> [machine-name] -> echoes the planted Axon root.
#
# Each call gets its own mktemp dir rather than a counter: plant runs inside a command
# substitution, so any variable it increments is lost with that subshell. An earlier version
# counted, every case therefore landed in the same directory, and case 5's axon.local.toml
# leaked into every case after it — which the suite caught by failing four of them.
plant() {
  local base; base="$(mktemp -d "$SCRATCH/case.XXXXXX")"
  local root="$base/axon" overlay="$base/overlay"
  mkdir -p "$root/tools/lib" "$overlay/config/machines"
  cp "$LIB_DIR/paths.sh" "$LIB_DIR/toml.sh" "$root/tools/lib/"
  printf 'overlay = "%s"\n' "$overlay" > "$root/axon.toml"
  local target="$overlay/config/machine.toml"
  if [ -n "${2:-}" ]; then
    target="$overlay/config/machines/$2.toml"
    printf 'overlay = "%s"\nmachine = "%s"\n' "$overlay" "$2" > "$root/axon.local.toml"
  fi
  printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = []\n%s' "$1" > "$target"
  printf '%s' "$root"
}

# Each call runs in its own shell: paths.sh exports and unsets, so re-sourcing in one
# process would carry state between cases and hide what is being tested.
call_out() { bash -c "source '$1/tools/lib/paths.sh' >/dev/null 2>&1 && $2 ${3:-}" 2>/dev/null; }
call_rc()  { bash -c "source '$1/tools/lib/paths.sh' >/dev/null 2>&1 && $2 ${3:-}" >/dev/null 2>&1; echo $?; }
call_err() { bash -c "source '$1/tools/lib/paths.sh' >/dev/null 2>&1 && $2 ${3:-}" 2>&1 >/dev/null; }

want() { # want <desc> <got> <expected>
  if [ "$2" != "$3" ]; then
    echo "  FAIL $1"; echo "    got:  $2"; echo "    want: $3"; fails=$((fails + 1))
  else
    echo "  ok   $1"
  fi
}

# 1. Configured: the declared path comes back, with a leading ~ expanded rather than handed
#    on as a literal only a shell would understand.
r="$(plant '
[[state_mount]]
tool = "knowledge-base"
path = "~/some/where"
data_class = "personal"
')"
want "a configured mount resolves, ~ expanded" "$(call_out "$r" axon_state_mount_for knowledge-base)" "$HOME/some/where"

# 2. An absolute path is passed through untouched.
r="$(plant '
[[state_mount]]
tool = "knowledge-base"
path = "/opt/elsewhere"
')"
want "an absolute mount is returned as-is" "$(call_out "$r" axon_state_mount_for knowledge-base)" "/opt/elsewhere"

# 3. Undeclared: must fail rather than answer. This is the case the hardcoded fallbacks used
#    to absorb, which is how the manifest and the scripts could disagree indefinitely.
r="$(plant '
[[state_mount]]
tool = "knowledge-base"
path = "/opt/x"
')"
want "an undeclared tool exits 1" "$(call_rc "$r" axon_state_mount_for mach-mono)" "1"
case "$(call_err "$r" axon_state_mount_for mach-mono)" in
  *mach-mono*) echo "  ok   the error names the tool it could not resolve" ;;
  *) echo "  FAIL the error does not name the tool"; fails=$((fails + 1)) ;;
esac

# 4. Ambiguous: two declarations for one tool are refused, not resolved by order — the same
#    call axon_manifest_for makes for a duplicate capability name.
r="$(plant '
[[state_mount]]
tool = "knowledge-base"
path = "~/first"

[[state_mount]]
tool = "knowledge-base"
path = "~/second"
')"
want "two declarations for one tool exit 2" "$(call_rc "$r" axon_state_mount_for knowledge-base)" "2"

# 5. Non-default location: resolution follows the SELECTED machine manifest. A second machine
#    declaring a different root is the entire reason this is not a constant.
r="$(plant '
[[state_mount]]
tool = "knowledge-base"
path = "/opt/service-node"
' service-node)"
want "the selected machine's mount wins" "$(call_out "$r" axon_state_mount_for knowledge-base)" "/opt/service-node"

# 6. A manifest with no mounts at all fails cleanly rather than returning empty-and-happy.
r="$(plant '')"
want "no state mounts at all exits 1" "$(call_rc "$r" axon_state_mount_for knowledge-base)" "1"

if [ "$fails" -gt 0 ]; then
  echo "state-mount-resolution: $fails check(s) failed" >&2
  exit 1
fi
echo "state-mount-resolution: all checks passed"

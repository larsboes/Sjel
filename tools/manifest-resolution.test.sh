#!/bin/bash
# Test for axon_manifest_for in tools/lib/paths.sh — the one place that turns a
# capability name into a manifest path. Builds a throwaway Axon root and a throwaway
# overlay beside it, then asserts each resolution case.
#
# The cases exist because getting any of them wrong is silent: a missed overlay root
# makes a private capability unrunnable, and a silently-resolved duplicate starts the
# wrong service under the right name.
set -uo pipefail

SCRATCH="$(mktemp -d "/tmp/manifest-resolution.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"

# The real lib under test, copied so paths.sh self-locates into the scratch root.
_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB_DIR="$_dir/lib"
if [ ! -f "$LIB_DIR/paths.sh" ]; then
  echo "manifest resolution: cannot find paths.sh next to $_dir" >&2
  exit 1
fi

# An operator's exported SJEL_OVERLAY_ROOT outranks the scratch root's own axon.toml inside
# paths.sh, which would make this test resolve the real overlay it is supposed to be replacing
# (tools/lib/test-support.sh#isolate_axon_env).
source "$LIB_DIR/test-support.sh"
isolate_axon_env

mkdir -p "$ROOT/tools/lib"
cp "$LIB_DIR/paths.sh" "$LIB_DIR/toml.sh" "$ROOT/tools/lib/"

mkdir -p "$OVERLAY/config"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = []\n' > "$OVERLAY/config/machine.toml"

new_manifest() {  # <dir>
  mkdir -p "$1"
  printf 'kind = "process"\nname = "%s"\n' "$(basename "$1")" > "$1/service.toml"
}

new_manifest "$ROOT/capabilities/root-only"
new_manifest "$OVERLAY/capabilities/overlay-only"
new_manifest "$ROOT/capabilities/both"
new_manifest "$OVERLAY/capabilities/both"
new_manifest "$ROOT/spine-thing"   # a top-level manifest is its own declaration

source "$ROOT/tools/lib/paths.sh"

fails=0
check() {  # check <description> <expected-rc> <expected-stdout> <name>
  local desc="$1" want_rc="$2" want_out="$3" name="$4" out rc=0
  out="$(axon_manifest_for "$name" 2>/dev/null)" || rc=$?
  if [ "$rc" != "$want_rc" ]; then
    echo "FAIL: $desc — exit $rc, wanted $want_rc"; fails=$((fails + 1)); return
  fi
  if [ "$out" != "$want_out" ]; then
    echo "FAIL: $desc — got '$out', wanted '$want_out'"; fails=$((fails + 1)); return
  fi
}

check "root capability resolves" 0 "$ROOT/capabilities/root-only/service.toml" root-only
check "overlay capability resolves" 0 "$OVERLAY/capabilities/overlay-only/service.toml" overlay-only
check "spine manifest at the root resolves" 0 "$ROOT/spine-thing/service.toml" spine-thing
check "unknown name is exit 1, no output" 1 "" nothing-here
check "duplicate is exit 2, no output" 2 "" both

# A refusal that does not say where the two manifests are leaves the reader to guess.
err="$(axon_manifest_for both 2>&1 >/dev/null || true)"
for path in "$ROOT/capabilities/both/service.toml" "$OVERLAY/capabilities/both/service.toml"; do
  if ! printf '%s' "$err" | grep -qF -- "$path"; then
    echo "FAIL: duplicate error omits $path"; fails=$((fails + 1))
  fi
done

# A deployment that owns no overlay capabilities has no such directory at all. That is
# the common case, and it must not turn into an error.
rm -rf "$OVERLAY/capabilities"
check "root capability still resolves without an overlay caps dir" \
  0 "$ROOT/capabilities/root-only/service.toml" root-only
check "overlay name is unknown once the dir is gone" 1 "" overlay-only

if [ "$fails" -gt 0 ]; then
  echo "manifest resolution: $fails check(s) failed"
  exit 1
fi
echo "manifest resolution: all checks passed"

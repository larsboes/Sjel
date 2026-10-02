#!/bin/bash
# tools/doctor's orphan-unit report, over a planted HOME and a planted overlay.
#
# The check answers one question — is this com.axon.* unit one this machine should have? —
# and getting it wrong is expensive in both directions. Too strict and it tells the operator
# to remove a unit something depends on, which is exactly what it did for macmon back when
# macmon was a hand-written unit the dashboard declared as a sidecar (#65). Too loose and a
# capability's leftover unit keeps starting a service the machine no longer enables,
# silently, which is the whole reason the check exists.
#
# macmon is an ordinary enabled capability now, so it covers the plain "enabled" case here
# rather than the exemption case. The exemption path is still exercised by dashboard.
#
# So both directions are asserted here, against the real doctor rather than a reimplementation
# of its rule.
#
# Driven against a real checkout, in CI's `repo gates` job. Handing doctor a partial tree does
# not fail loudly — its persistence check returns early when tools/service-runner.sh is
# missing, so the report comes back empty and every "must still be flagged" assertion passes
# for the wrong reason.
set -uo pipefail

_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

FAKE_HOME="$WORK/home"
UNIT_DIR="$FAKE_HOME/Library/LaunchAgents"
OVERLAY="$WORK/overlay"
mkdir -p "$UNIT_DIR" "$OVERLAY/config"

# One enabled capability, so the machine has a non-empty set to compare against.
cat > "$OVERLAY/config/machine.toml" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["comms", "macmon"]
EOF

plant_unit() { printf '<plist/>\n' > "$UNIT_DIR/com.axon.$1.plist"; }

# The three cases the issue names, planted together so one run covers all of them.
plant_unit comms       # enabled          -> never an orphan
plant_unit dashboard   # spine component  -> exempt via its own root manifest
plant_unit macmon      # enabled capability -> never an orphan (was a dashboard sidecar)
plant_unit trips       # a disabled capability's leftover unit -> ORPHAN
plant_unit nothing-declares-this                                # -> ORPHAN

# Only the persistence section matters here; doctor legitimately fails other checks against a
# planted HOME, and asserting on its exit code would be asserting on those instead.
OUT="$( cd "$_root" && HOME="$FAKE_HOME" SJEL_OVERLAY_ROOT="$OVERLAY" tools/doctor 2>&1 )"
PERSIST="$(printf '%s\n' "$OUT" | grep 'persistence is installed for' || true)"

flagged() { printf '%s\n' "$PERSIST" | grep -q "persistence is installed for '$1'"; }

# --- must NOT be flagged ------------------------------------------------------------------
for name in comms dashboard macmon; do
  if flagged "$name"; then
    fail "'$name' was reported as an orphan. Persistence notes were:
$PERSIST"
  fi
done

# --- must STILL be flagged ----------------------------------------------------------------
# The half that stops the fix from being "exempt everything". Without these the check could be
# deleted entirely and this file would still pass.
for name in trips nothing-declares-this; do
  if ! flagged "$name"; then
    fail "'$name' is a genuine orphan and was not reported. Persistence notes were:
$PERSIST"
  fi
done

# --- the exemption is derived, not spelled ------------------------------------------------
# If a name that no manifest mentions is exempt, the derivation has been replaced by a list.
# dashboard is exempt because it HAS a root-level service.toml, never because it is named.
if [ ! -f "$_root/dashboard/service.toml" ]; then
  fail "dashboard/service.toml is gone — the spine exemption derives from it, so nothing is left to derive"
fi
if grep -rqE '"macmon"' "$_root/tools/sjel-cli/src/doctor/"; then
  fail "the doctor (tools/sjel-cli/src/doctor/) names macmon directly — the exemption must come from the manifest"
fi

if [ "$fails" -gt 0 ]; then
  echo "persistence orphans: $fails check(s) failed"
  exit 1
fi
echo "persistence orphans: all checks passed"

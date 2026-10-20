#!/usr/bin/env bash
# Regenerate rung 0's known-person registry from capabilities/entities.
#
# Rung 0 (capabilities/comms/src/people_registry.rs) consults a list of the
# names this operator actually knows, because rung 1's person detector only
# fires after a salutation and therefore misses a bare first name. The list is
# every person in capabilities/entities with their aliases (src/names.rs), and goes
# stale the moment a person is added. It came from the vault's Atlas/People notes
# until 2026-10-20, when they moved into entities.
#
# This exists as a script rather than a shell fragment inside a LaunchAgent for
# the reason every other Sjel agent does the same: a plist that carries quoting,
# redirection and an && is a plist nobody can test. Run it by hand and it
# behaves identically to the scheduled run.
#
# The output is C2 data — real names — so it lands in the overlay, which
# gitignores /data/*, and never in this repository.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

# The binary capabilities/people-registry/service.toml declares as its build_output, which the
# service runner rebuilds before each run. A CARGO_TARGET_DIR copy is only the fallback: it is
# whatever a developer last built there and can predate the `names` command.
BIN="$SJEL_ROOT/target/release/entities-server"
[ -x "$BIN" ] || BIN="${CARGO_TARGET_DIR:-$SJEL_ROOT/target}/release/entities-server"
[ -x "$BIN" ] || { echo "entities-server not built: tools/service-runner.sh restart entities" >&2; exit 1; }

OUT_DIR="$SJEL_PERSONAL_ROOT/data/vault"
OUT="$OUT_DIR/people-registry.json"
mkdir -p "$OUT_DIR"

# Written to a temp file and moved into place. comms loads this once at startup,
# and a half-written file would read as State::Unreadable rather than as a
# registry. mv within one filesystem is atomic.
TMP="$OUT.tmp.$$"
trap 'rm -f "$TMP"' EXIT

"$BIN" names > "$TMP"

# A registry that suddenly holds nothing is a store that failed to load, not a
# store with no people. Overwriting a good file with that is the one outcome
# worth refusing outright.
count="$(grep -c '"' "$TMP" || true)"
if ! grep -q '"tokens"' "$TMP" || [ "$count" -lt 3 ]; then
  echo "refusing to install an empty or malformed registry" >&2
  exit 1
fi

mv "$TMP" "$OUT"
trap - EXIT
echo "people-registry refreshed: $(wc -c < "$OUT") bytes"

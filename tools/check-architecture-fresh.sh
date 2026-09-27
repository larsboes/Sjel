#!/bin/bash
# check-architecture-fresh.sh — the ARCHITECTURE.md freshness gate (CI: repo gates).
# Fails if ARCHITECTURE.md is stale relative to the manifests it is generated from.
# Never writes into the checkout: it regenerates into a scratch file and diffs, because
# a gate that fixes what it finds reports green over a change nobody reviewed.
# See README.md, "Generated architecture".
set -e

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

before="$SJEL_ROOT/ARCHITECTURE.md"
after="$(mktemp)"
trap 'rm -f "$after"' EXIT

# ARCHITECTURE_OUT: generate into the scratch file rather than in place.
ARCHITECTURE_OUT="$after" bash "$SJEL_ROOT/tools/generate-architecture.sh" >/dev/null

# The "Generated: <timestamp>" line always differs -- that's not staleness.
if diff -q <(grep -v '^> Generated:' "$before") <(grep -v '^> Generated:' "$after") >/dev/null; then
  echo "ARCHITECTURE.md is up to date."
  exit 0
fi

echo "ARCHITECTURE.md is stale. Run: tools/generate-architecture.sh" >&2
diff <(grep -v '^> Generated:' "$before") <(grep -v '^> Generated:' "$after") >&2 || true
exit 1

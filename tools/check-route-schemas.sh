#!/bin/bash
# Fails when a capability gains a write route that declares no request schema.
#
# `route_manifest::bodies_without_schemas` is the canonical check, run as a unit
# test by the capabilities that are clean. This is its repo-wide ratchet: the
# bodies already undeclared are baselined, so the ones that exist direct new
# work instead of blocking every capability at once, and any *increase* fails.
#
# The baseline may only go down. `improved:` lines say which capability to lower.
# Do not raise a number to land a route: derive a schema with
# `route_manifest::schema_of::<T>()` and attach it, or say in the commit why the
# body genuinely cannot have one.
#
# Usage: tools/check-route-schemas.sh
#        SJEL_ROUTE_SCHEMA_ROOT=<dir> SJEL_ROUTE_SCHEMA_BASELINE=<file>  (tests)
set -uo pipefail

ROOT="${SJEL_ROUTE_SCHEMA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)}"
BASELINE="${SJEL_ROUTE_SCHEMA_BASELINE:-$ROOT/tools/route-schema-baseline.txt}"

[ -f "$BASELINE" ] || { echo "check-route-schemas: no baseline at $BASELINE" >&2; exit 2; }

# The manifest is a `const ROUTES` block. Text matching, not parsing, for the
# same reason `undeclared_routes` reads source as text: it cannot see a route
# built from a runtime string, and no router in this repo builds one.
block_of() {
  awk '/const ROUTES/{flag=1} flag{print} flag && /^];/{exit}' "$1"
}

baseline_count() {
  awk -v c="$1" '$1==c {print $2; found=1} END{if(!found) print ""}' "$BASELINE"
}

fail=0
checked=0
while IFS= read -r f; do
  cap="$(printf '%s' "$f" | sed -E 's|.*/capabilities/([^/]+)/.*|\1|')"
  block="$(block_of "$f")"
  writes="$(printf '%s\n' "$block" | grep -oE '"(POST|PUT|PATCH)"' | wc -l | tr -d ' ')"
  schemas="$(printf '%s\n' "$block" | grep -c 'request_schema: Some')"
  undeclared=$((writes - schemas))
  checked=$((checked + 1))

  base="$(baseline_count "$cap")"
  if [ -z "$base" ]; then
    if [ "$undeclared" -gt 0 ]; then
      echo "NEW:        $cap declares $undeclared write body/bodies without a schema and is not in the baseline"
      fail=1
    else
      echo "new:        $cap declares every write body — add it to the baseline at 0"
    fi
  elif [ "$undeclared" -gt "$base" ]; then
    echo "REGRESSED:  $cap $base → $undeclared undeclared write bodies"
    fail=1
  elif [ "$undeclared" -lt "$base" ]; then
    echo "improved:   $cap $base → $undeclared — lower the baseline"
  fi
done < <(grep -rl 'const ROUTES' "$ROOT"/capabilities/*/src/*.rs 2>/dev/null | sort)

[ "$checked" -gt 0 ] || { echo "check-route-schemas: found no capability ROUTES manifests under $ROOT" >&2; exit 2; }

echo "check-route-schemas: $checked capability manifest(s) checked against $(basename "$BASELINE")"
if [ "$fail" -ne 0 ]; then
  echo "check-route-schemas: a write route lost its schema — attach request_schema, or lower the baseline with a reason." >&2
  exit 1
fi
echo "check-route-schemas: no capability gained an undeclared write body."

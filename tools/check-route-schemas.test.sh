#!/bin/bash
# Planted-root regression tests for tools/check-route-schemas.sh.
#
# The tool reads a checkout, so every case runs against a throwaway root tree
# named through SJEL_ROUTE_SCHEMA_ROOT, and never touches this repository.
set -uo pipefail

TOOL="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-route-schemas.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export SJEL_ROUTE_SCHEMA_ROOT="$SCRATCH"
export SJEL_ROUTE_SCHEMA_BASELINE="$SCRATCH/baseline.txt"

fails=0
run() { out="$("$TOOL" 2>&1)"; status=$?; }

expect() { # name expected_exit needle
  run
  if [ "$status" -ne "$2" ]; then
    echo "FAIL: $1: expected exit $2, got $status"
    fails=$((fails + 1))
  fi
  if ! printf '%s' "$out" | grep -qF "$3"; then
    echo "FAIL: $1: expected \"$3\" in output; got: $out"
    fails=$((fails + 1))
  fi
}

# A server with `undeclared` write routes that take a body and no schema, and
# `declared` write routes that attach one.
plant_server() { # path undeclared declared
  local path="$1" undeclared="$2" declared="$3" i body
  mkdir -p "$(dirname "$path")"
  {
    printf 'const ROUTES: &[route_manifest::Route] = &[\n'
    printf '    r("GET", "/health", "Liveness."),\n'
    i=0
    while [ "$i" -lt "$undeclared" ]; do
      printf '    r("POST", "/api/undeclared-%s", "Takes a body, declares none."),\n' "$i"
      i=$((i + 1))
    done
    i=0
    while [ "$i" -lt "$declared" ]; do
      printf '    route_manifest::Route {\n'
      printf '        method: "POST",\n        path: "/api/declared-%s",\n' "$i"
      printf '        summary: "A declared body.",\n'
      printf '        request_schema: Some(route_manifest::schema_of::<Thing>),\n'
      printf '    },\n'
      i=$((i + 1))
    done
    printf '];\n'
  } > "$path"
}

write_baseline() { printf '%s\n' "$@" > "$SJEL_ROUTE_SCHEMA_BASELINE"; }

# 1. Equal to the baseline is the passing state.
plant_server "$SCRATCH/capabilities/alpha/src/server.rs" 1 0
write_baseline "alpha 1"
expect "count equal to the baseline passes" 0 "no capability gained"

# 2. One more undeclared body fails, and names the capability.
plant_server "$SCRATCH/capabilities/alpha/src/server.rs" 2 0
expect "a new undeclared body fails" 1 "REGRESSED:  alpha 1 → 2"

# 3. A fall to the baseline is a pass and asks for the baseline to be lowered.
plant_server "$SCRATCH/capabilities/alpha/src/server.rs" 0 1
expect "a lower count passes and is reported" 0 "improved:   alpha 1 → 0"

# 4. A capability absent from the baseline owes a schema at once.
plant_server "$SCRATCH/capabilities/beta/src/server.rs" 1 0
expect "a new capability with an undeclared body fails" 1 "NEW:        beta declares 1"

# 5. A capability absent from the baseline that declares everything is a pass.
plant_server "$SCRATCH/capabilities/beta/src/server.rs" 0 1
expect "a new capability that declares every body passes" 0 "new:        beta declares every write body"

# 6. No manifests at all is a broken invocation, not a green build.
rm -rf "$SCRATCH/capabilities"
expect "no manifests is exit 2" 2 "found no capability ROUTES manifests"

if [ "$fails" -eq 0 ]; then
  echo "check-route-schemas: all checks passed"
  exit 0
fi
echo "check-route-schemas: $fails check(s) failed"
exit 1

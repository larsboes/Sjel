#!/bin/bash
# Regression tests for the battery guard: the profile's answer, the one-command exception, and
# the "undecidable" answer a stale binary gives. A stub CLI stands in, so no build runs.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/local-build"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fails=0

stub() { # stub <name> <exit> [stderr]
  local path="$SCRATCH/$1"
  printf '#!/bin/sh\n%s\nexit %s\n' "${3:+echo '$3' >&2}" "$2" > "$path"
  chmod +x "$path"
  printf '%s' "$path"
}

expect() { # expect <description> <wanted> <stub> <env...>
  local what="$1" want="$2" bin="$3"
  shift 3
  out="$(env SJEL_CLI_BIN="$bin" "$@" "$GUARD" 2>&1)"; status=$?
  if [ "$status" -ne "$want" ]; then
    echo "FAIL: $what — wanted exit $want, got $status:" >&2
    printf '%s\n' "$out" >&2
    fails=$((fails + 1))
  fi
}

expect "an admitted profile passes" 0 "$(stub allowed 0)"
expect "a deferred profile refuses" 3 "$(stub deferred 3 'deferred by runtime profile: On the go')"
expect "a binary too old to answer is undecidable, not a refusal" 2 "$(stub stale 1)"
expect "the one-command exception bypasses the profile" 0 "$(stub deferred 3)" SJEL_ALLOW_LOCAL_BUILD=1

if [ "$fails" -ne 0 ]; then
  echo "local-build.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "local-build.test.sh: all cases passed"

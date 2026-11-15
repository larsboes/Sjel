#!/bin/bash
# Stubbed admission and explicit battery-build defaults; no compiler runs.
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
  out="$(env -u SJEL_ROOT -u SJEL_ALLOW_LOCAL_BUILD SJEL_CLI_BIN="$bin" "$@" "$GUARD" 2>&1)"; status=$?
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

cat > "$SCRATCH/root-check" <<'EOF'
#!/bin/sh
[ "${SJEL_ROOT:-}" = "$LOCAL_BUILD_TEST_ROOT" ] && [ "$*" = 'runtime admit local-build' ]
EOF
chmod +x "$SCRATCH/root-check"
expect "a prebuilt CLI receives its checkout root without an inherited root" 0 "$SCRATCH/root-check" \
  LOCAL_BUILD_TEST_ROOT="$(cd "$(dirname "$GUARD")/.." && pwd)"

expect_run() {
  local what="$1" want="$2" expected="$3" bin="$4"
  shift 4
  out="$(env -u SJEL_ROOT -u SJEL_ALLOW_LOCAL_BUILD -u CARGO_BUILD_JOBS -u RUST_TEST_THREADS \
    SJEL_CLI_BIN="$bin" "$@" "$GUARD" run bash -c \
    'printf "budget=%s/%s/%s\\n" "${CARGO_BUILD_JOBS:-unset}" "${RUST_TEST_THREADS:-unset}" "${SJEL_ALLOW_LOCAL_BUILD:-unset}"; exit 7' 2>&1)"; status=$?
  if [ "$status" -ne "$want" ] || ! grep -qF "$expected" <<< "$out"; then
    echo "FAIL: $what — wanted exit $want and '$expected', got $status: $out" >&2
    fails=$((fails + 1))
  fi
}

expect_run "battery builds use conservative defaults and preserve child exit" 7 "budget=2/2/1" "$(stub battery 3)"
expect_run "lower limits remain lower" 7 "budget=1/1/1" "$(stub battery 3)" CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1
expect_run "higher environment limits are reduced" 7 "budget=2/2/1" "$(stub battery 3)" CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12
expect_run "Normal does not alter concurrency" 7 "budget=unset/unset/unset" "$(stub allowed 0)"
expect_run "Normal preserves explicit concurrency" 7 "budget=4/3/unset" "$(stub allowed 0)" CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=3
expect_run "unknown admission does not execute the child" 2 "cannot read the power profile" "$(stub stale 1)"
expect_run "invalid concurrency fails before execution" 2 "positive integer" "$(stub battery 3)" CARGO_BUILD_JOBS=auto
expect_run "zero test concurrency fails before execution" 2 "positive integer" "$(stub battery 3)" RUST_TEST_THREADS=0
out="$($GUARD run 2>&1)"; status=$?
if [ "$status" -ne 2 ] || ! grep -qF Usage <<< "$out"; then
  echo "FAIL: run without a command must report usage" >&2
  fails=$((fails + 1))
fi

if [ "$fails" -ne 0 ]; then
  echo "local-build.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "local-build.test.sh: all cases passed"

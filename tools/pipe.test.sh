#!/bin/bash
# Tests for tools/lib/pipe.sh (#42).
#
# The whole point is the LARGE-producer case. A short stream passes with `grep -q` and with
# `stream_matches` alike, so a test built on one would prove nothing and would have let this bug
# ship — which is exactly what happened: eight call sites carried it for as long as the container
# lists stayed small enough to finish writing first.
#
# Run under bash on purpose, even where the developer's interactive shell is zsh: the scripts this
# protects are `#!/bin/bash` with `set -euo pipefail`, and the SIGPIPE-vs-pipefail interaction is
# a property of that combination.
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB=""
for _c in "$_dir/lib" "$_dir/tools/lib"; do
  if [ -f "$_c/pipe.sh" ]; then LIB="$_c"; break; fi
done
[ -n "$LIB" ] || { echo "pipe: cannot find pipe.sh next to $_dir" >&2; exit 1; }
# shellcheck source=lib/pipe.sh
. "$LIB/pipe.sh"

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# Big enough that the producer is still writing when a matcher that quits early would quit.
BIG=300000

# --- the regression itself -------------------------------------------------
# Demonstrate the defect first, so this file also documents what it is protecting against. If this
# assertion ever stops holding, the bug is gone from bash itself and the rest of this is history.
( set -o pipefail; seq 1 "$BIG" | grep -q "^7$" ) 2>/dev/null
naive=$?
if [ "$naive" -eq 0 ]; then
  echo "NOTE: bash no longer propagates the producer's SIGPIPE here; the defect this guards is gone"
else
  # The naive form reports a FOUND value as a failure. That is the bug.
  [ "$naive" -ne 0 ] || fail "expected the naive grep -q form to misreport, it did not"
fi

# --- the fix ---------------------------------------------------------------
# Same question, same stream, correct answer — regardless of where the match sits.
( set -o pipefail; seq 1 "$BIG" | stream_matches "^7$" )
[ $? -eq 0 ] || fail "a match at the START of a large stream was reported as no match"

( set -o pipefail; seq 1 "$BIG" | stream_matches "^$((BIG - 1))\$" )
[ $? -eq 0 ] || fail "a match at the END of a large stream was reported as no match"

( set -o pipefail; seq 1 "$BIG" | stream_matches "^not-in-there$" )
[ $? -ne 0 ] || fail "a stream with no match was reported as matching"

# --- flags reach grep ------------------------------------------------------
# Every call site passes one: -x for whole-line container names, -E for the runtime status line.
( set -o pipefail; printf 'sjel-status\nsjel-status-extra\n' | stream_matches -x "sjel-status" )
[ $? -eq 0 ] || fail "-x did not match a whole line"
( set -o pipefail; printf 'sjel-status-extra\n' | stream_matches -x "sjel-status" )
[ $? -ne 0 ] || fail "-x matched a line it should not have"
( set -o pipefail; printf 'status  running\n' | stream_matches -E '^status[[:space:]]+running' )
[ $? -eq 0 ] || fail "-E did not match"

# --- an empty stream is not a match ---------------------------------------
( set -o pipefail; printf '' | stream_matches "anything" )
[ $? -ne 0 ] || fail "an empty stream was reported as matching"

# --- a failing producer still fails ---------------------------------------
# This is why the fix is `grep -c` rather than dropping pipefail: a runtime that cannot be asked
# must not be read as "the container is not there". pipefail still sees the producer's status.
( set -o pipefail; { echo present; exit 3; } | stream_matches "present" )
[ $? -ne 0 ] || fail "a producer that failed after emitting a match was reported as a clean match"

# --- no call site kept the old form ---------------------------------------
# The acceptance is "one idiom, applied at every site", so the absence is asserted rather than
# left to a reviewer's grep.
for f in "$_dir/service-runner.sh" "$_dir/restore.sh" "$_dir/tools/service-runner.sh" "$_dir/tools/restore.sh"; do
  [ -f "$f" ] || continue
  if grep -nE '\| *grep -[a-zA-Z]*q' "$f"; then
    fail "$(basename "$f") still pipes into grep -q (lines above)"
  fi
done

if [ "$fails" -gt 0 ]; then
  echo "pipe.sh: $fails check(s) failed"
  exit 1
fi
echo "pipe.sh: all checks passed"

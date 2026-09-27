#!/bin/bash
# Tests for tools/cargo-hermetic: the guards, watched refusing planted input.
#
# The tool's whole value is what it does NOT let happen, and none of that is visible in a
# green run. So every case here hands it something it must refuse, or an environment it must
# override, and reads the answer back. A guard nobody has watched fail is the sixth silent
# failure in PRD §13.1.
#
# Nothing here invokes cargo. `--print-env` exists as the seam: it resolves exactly what a
# run would resolve and then does nothing with it, so the environment can be asserted on
# without a five-minute workspace build inside a test suite.
set -uo pipefail

TOOL="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/cargo-hermetic"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd -P)"

fails=0
out=""
status=0

note_fail() {
  echo "FAIL: $1"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
}

run() { # run <args...> -> $out, $status
  out=$("$TOOL" "$@" 2>&1)
  status=$?
}

# --- assertions, all reading the LAST run ----------------------------------

expect_refused() { # expect_refused <description> <substring>
  [ "$status" -eq 2 ] || { note_fail "$1 — wanted exit 2, got $status"; return; }
  printf '%s' "$out" | grep -qF "$2" || note_fail "$1 — refusal does not say '$2'"
}

expect_ok() { # expect_ok <description>
  [ "$status" -eq 0 ] || note_fail "$1 — wanted exit 0, got $status"
}

env_value() { printf '%s\n' "$out" | sed -n "s/^$1=//p"; }

expect_not_under() { # expect_not_under <description> <VAR> <forbidden-root>
  local value
  value="$(env_value "$2")"
  [ -n "$value" ] || { note_fail "$1 — --print-env does not report $2"; return; }
  case "$value" in
    "$3"|"$3"/*) note_fail "$1 — $2 is $value, under $3" ;;
  esac
}

expect_ends_with() { # expect_ends_with <description> <VAR> <suffix>
  local value
  value="$(env_value "$2")"
  [ -n "$value" ] || { note_fail "$1 — --print-env does not report $2"; return; }
  case "$value" in
    *"$3") ;;
    *) note_fail "$1 — $2 is $value, which does not end with $3" ;;
  esac
}

# --- the release ban -------------------------------------------------------
#
# Four spellings, because cargo accepts four and a guard that catches one of them reads as
# protection while the other three go through.

run test --release;            expect_refused "--release is refused" "target/release"
run test -r;                   expect_refused "-r is refused" "target/release"
run build --profile release;   expect_refused "--profile release is refused" "target/release"
run build --profile=release;   expect_refused "--profile=release is refused" "target/release"

# The ban is on the release profile, not on the word. A debug invocation must survive it.
run --print-env test --workspace --locked
expect_ok "an ordinary debug invocation is allowed"

# --- the target directory --------------------------------------------------
#
# The fourth silent failure: a worktree's build wrote target/release/<bin> in the checkout
# the supervisor runs from, and killed the live service. Both checkouts are refused — the
# one this script sits in, and the main one it may be a worktree of.

out=$(CARGO_TARGET_DIR="$REPO/target" "$TOOL" test 2>&1); status=$?
expect_refused "a target dir inside this checkout is refused" "holds the binaries the supervisor runs"

out=$(CARGO_TARGET_DIR="$REPO/target/nested/deeper" "$TOOL" test 2>&1); status=$?
expect_refused "a target dir nested deep inside the checkout is refused too" "Point it at scratch"

# ...and refused BEFORE it is created. A guard that mkdir -p's its way to the verdict has
# already put the directory in the repository it was protecting.
if [ -e "$REPO/target/nested" ]; then
  out=""
  note_fail "the refusal created $REPO/target/nested on its way to refusing it"
  rm -rf "$REPO/target/nested"
fi

# A path that merely starts with the same characters is a different directory. Asserted on
# the reason rather than on the exit status, because this suite runs in both layouts: a
# plain clone accepts `<repo>-elsewhere`, and a worktree of the same repository refuses it
# for the OTHER root — the main checkout it sits under. What must never happen either way is
# this checkout's own path being named.
out=$(CARGO_TARGET_DIR="${REPO}-elsewhere/target" "$TOOL" --print-env 2>&1); status=$?
if printf '%s' "$out" | grep -qF "inside the checkout at $REPO."; then
  note_fail "a sibling path sharing a prefix with the checkout was treated as inside it"
fi

SCRATCH_TARGET="${TMPDIR:-/tmp}/axon-cargo-hermetic-test/target"
out=$(CARGO_TARGET_DIR="$SCRATCH_TARGET" "$TOOL" --print-env 2>&1); status=$?
expect_ok "a scratch target dir outside every checkout is honoured"
expect_ends_with "a warm target dir is reused rather than replaced" CARGO_TARGET_DIR \
  "axon-cargo-hermetic-test/target"

# --- the environment the run inherits --------------------------------------
#
# The third silent failure, in one case: a session exports the real vault as a projection
# root — the overlay's own config/shell exports the SJEL_* set — and a run that redirected
# only the database rewrote thirteen real notes. Every variable that can point at a vault is
# poisoned here with a path that must not survive.

POISON="/nowhere/axon-poison/Knowledge-Base"
out=$(SJEL_PERSONAL_ROOT="/nowhere/axon-poison/overlay" \
      SJEL_OVERLAY_ROOT="/nowhere/axon-poison/overlay" \
      SJEL_DB_PATH="/nowhere/axon-poison/overlay/data/axon/axon.db" \
      SJEL_COMMS_CONFIG="/nowhere/axon-poison/overlay/config/comms.json" \
      SJEL_TRIPS_OBSIDIAN_ROOT="$POISON" \
      SJEL_FINANCE_OBSIDIAN_ROOT="$POISON" \
      SJEL_FINANCE_DECISIONS_ROOT="$POISON" \
      SJEL_INTERIOR_OBSIDIAN_ROOT="$POISON" \
      "$TOOL" --print-env 2>&1); status=$?
expect_ok "--print-env answers under a poisoned environment"
for var in SJEL_PERSONAL_ROOT SJEL_OVERLAY_ROOT SJEL_DB_PATH SJEL_COMMS_CONFIG \
           SJEL_TRIPS_OBSIDIAN_ROOT SJEL_FINANCE_OBSIDIAN_ROOT \
           SJEL_FINANCE_DECISIONS_ROOT SJEL_INTERIOR_OBSIDIAN_ROOT; do
  expect_not_under "an inherited $var does not survive" "$var" "/nowhere/axon-poison"
done
expect_ends_with "the comms config is a file, not a directory" SJEL_COMMS_CONFIG ".json"
expect_ends_with "the database sits under the sandbox overlay" SJEL_DB_PATH "/overlay/data/axon/axon.db"

# The projection roots must not be the sandbox OVERLAY either. SJEL_FINANCE_DECISIONS_ROOT
# exists (PRD Q80) precisely because redirecting the overlay instead would redirect the
# config read and test a configuration nobody is running.
run --print-env
expect_ok "--print-env with no cargo arguments"
overlay="$(env_value SJEL_PERSONAL_ROOT)"
for var in SJEL_TRIPS_OBSIDIAN_ROOT SJEL_FINANCE_OBSIDIAN_ROOT SJEL_FINANCE_DECISIONS_ROOT \
           SJEL_INTERIOR_OBSIDIAN_ROOT; do
  if [ "$(env_value "$var")" = "$overlay" ]; then
    note_fail "$var points at the sandbox overlay rather than a projection root of its own"
  fi
done

# --- usage -----------------------------------------------------------------

run
expect_refused "a bare invocation is usage, not a silent cargo run" "needs a cargo subcommand"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "cargo-hermetic.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "cargo-hermetic.test.sh: all cases passed"

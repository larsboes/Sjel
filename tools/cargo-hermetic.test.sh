#!/bin/bash
# Regression probes for hermetic Cargo guards, cache reuse and fixture cleanup.
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

SCRATCH_TARGET="${TMPDIR:-/tmp}/sjel-cargo-hermetic-test/target"
out=$(CARGO_TARGET_DIR="$SCRATCH_TARGET" "$TOOL" --print-env 2>&1); status=$?
expect_ok "a scratch target dir outside every checkout is honoured"
expect_ends_with "a warm target dir is reused, one subdirectory per checkout" CARGO_TARGET_DIR \
  "sjel-cargo-hermetic-test/target/hermetic-$(printf %s "$(cd "$REPO" && pwd -P)" | shasum | cut -c1-12)"

NESTED_TARGET="$(env_value CARGO_TARGET_DIR)"
out=$(CARGO_TARGET_DIR="$NESTED_TARGET" "$TOOL" --print-env 2>&1); status=$?
expect_ok "a nested invocation is admitted"
[ "$(env_value CARGO_TARGET_DIR)" = "$NESTED_TARGET" ] || note_fail "a nested invocation forked another build cache"

# --- the environment the run inherits --------------------------------------
# Explicit Cargo flags take precedence, and nested Cargo invocations inherit the same target.
EXPLICIT_TARGET="$(cd "${TMPDIR:-/tmp}" && pwd -P)/sjel-explicit-target"
run --print-env test --target-dir "$EXPLICIT_TARGET"
expect_ok "an explicit target is admitted"
[ "$(env_value CARGO_TARGET_DIR)" = "$EXPLICIT_TARGET" ] || note_fail "nested cargo would use a different target"
run --print-env test --target-dir="$EXPLICIT_TARGET"
expect_ok "the equals spelling is admitted"
[ "$(env_value CARGO_TARGET_DIR)" = "$EXPLICIT_TARGET" ] || note_fail "the equals spelling lost its target"
run --print-env test --target-dir "$REPO/target"
expect_refused "an explicit checkout target is refused" "holds the binaries the supervisor runs"
run --print-env test --target-dir="$REPO/target"
expect_refused "an equals checkout target is refused" "holds the binaries the supervisor runs"
out=$(CARGO_TARGET_DIR="$REPO/target" "$TOOL" --print-env test --target-dir "$EXPLICIT_TARGET" 2>&1); status=$?
expect_ok "the explicit target overrides an inherited target"
run --print-env test --target-dir
expect_refused "a missing target value is refused" "needs a directory"
run --print-env test --target-dir=
expect_refused "an empty target value is refused" "needs a directory"
run --print-env test -- --target-dir "$REPO/target"
expect_ok "test executable arguments do not select Cargo's target"

#
# The third silent failure, in one case: a session exports the real vault as a projection
# root — the overlay's own config/shell exports the SJEL_* set — and a run that redirected
# only the database rewrote thirteen real notes. Every variable that can point at a vault is
# poisoned here with a path that must not survive.

POISON="/nowhere/sjel-poison/Knowledge-Base"
out=$(SJEL_PERSONAL_ROOT="/nowhere/sjel-poison/overlay" \
      XDG_STATE_HOME="/nowhere/sjel-poison/state" \
      SJEL_OVERLAY_ROOT="/nowhere/sjel-poison/overlay" \
      SJEL_DB_PATH="/nowhere/sjel-poison/overlay/data/sjel/sjel.db" \
      SJEL_COMMS_CONFIG="/nowhere/sjel-poison/overlay/config/comms.json" \
      SJEL_TRIPS_EXPORT_ROOT="$POISON" \
      SJEL_FINANCE_DECISIONS_ROOT="$POISON" \
      "$TOOL" --print-env 2>&1); status=$?
expect_ok "--print-env answers under a poisoned environment"
for var in XDG_STATE_HOME SJEL_PERSONAL_ROOT SJEL_OVERLAY_ROOT SJEL_DB_PATH SJEL_COMMS_CONFIG \
           SJEL_TRIPS_EXPORT_ROOT \
           SJEL_FINANCE_DECISIONS_ROOT; do
  expect_not_under "an inherited $var does not survive" "$var" "/nowhere/sjel-poison"
done
expect_ends_with "the comms config is a file, not a directory" SJEL_COMMS_CONFIG ".json"
expect_ends_with "the database sits under the sandbox overlay" SJEL_DB_PATH "/overlay/data/sjel/sjel.db"
expect_ends_with "temporary files belong to the test sandbox" TMPDIR "/tmp"
[ "$(env_value TMPDIR)" = "$(env_value TMP)" ] || note_fail "TMP escapes the sandbox"
[ "$(env_value TMPDIR)" = "$(env_value TEMP)" ] || note_fail "TEMP escapes the sandbox"

# The projection roots must not be the sandbox OVERLAY either. SJEL_FINANCE_DECISIONS_ROOT
# exists (PRD Q80) precisely because redirecting the overlay instead would redirect the
# config read and test a configuration nobody is running.
run --print-env
expect_ok "--print-env with no cargo arguments"
overlay="$(env_value SJEL_PERSONAL_ROOT)"
for var in SJEL_TRIPS_EXPORT_ROOT SJEL_FINANCE_DECISIONS_ROOT; do
  if [ "$(env_value "$var")" = "$overlay" ]; then
    note_fail "$var points at the sandbox overlay rather than a projection root of its own"
  fi
done

# --- usage -----------------------------------------------------------------

run
expect_refused "a bare invocation is usage, not a silent cargo run" "needs a cargo subcommand"

case_dir="$(mktemp -d "${TMPDIR:-/tmp}/sjel-hermetic-cases.XXXXXX")"
case_dir="$(cd "$case_dir" && pwd -P)"
trap 'rm -rf "$case_dir"' EXIT
mkdir -p "$case_dir/bin" "$case_dir/host-tmp"
cat > "$case_dir/bin/cargo" <<'CARGO'
#!/bin/bash
set -eu
printf '%s\n%s\n' "$TMPDIR" "$CARGO_TARGET_DIR" > "$HERMETIC_TEST_REPORT"
mkdir -p "$TMPDIR/comms-test-$$"
printf 'fixture\n' > "$TMPDIR/comms-test-$$/store.db"
[ "${1:-}" != "fail" ] || exit 19
CARGO
chmod +x "$case_dir/bin/cargo"

fake_run() {
  out=$(PATH="$case_dir/bin:$PATH" TMPDIR="$case_dir/host-tmp" \
    CARGO_TARGET_DIR="$case_dir/cache" HERMETIC_TEST_REPORT="$case_dir/report" \
    SJEL_ALLOW_LOCAL_BUILD=1 "$TOOL" "$@" 2>&1)
  status=$?
}

fake_run test
expect_ok "a planted test process succeeds"
fixture_tmp="$(sed -n '1p' "$case_dir/report")"
fixture_cache="$(sed -n '2p' "$case_dir/report")"
[ ! -e "$fixture_tmp" ] || note_fail "successful tests left temporary databases behind"
[ -d "$fixture_cache" ] || note_fail "fixture cleanup removed the reusable build cache"

fake_run fail
[ "$status" -eq 19 ] || note_fail "a failed test process lost its exit status"
fixture_tmp="$(sed -n '1p' "$case_dir/report")"
[ ! -e "$fixture_tmp" ] || note_fail "failed tests left temporary databases behind"

cat > "$case_dir/bin/rmdir" <<'RMDIR'
#!/bin/bash
case "$1" in
  "$HERMETIC_TEST_BLOCKED"/*) exit 1 ;;
esac
exec /bin/rmdir "$@"
RMDIR
chmod +x "$case_dir/bin/rmdir"

fallback_run() {
  out=$(PATH="$case_dir/bin:$PATH" TMPDIR="$case_dir/host-tmp" \
    CARGO_TARGET_DIR="$case_dir/blocked" HERMETIC_TEST_BLOCKED="$case_dir/blocked" \
    HERMETIC_TEST_REPORT="$case_dir/report" SJEL_ALLOW_LOCAL_BUILD=1 \
    "$TOOL" "$@" 2>&1)
  status=$?
}

fallback_run test
expect_ok "an inherited cache with denied cleanup uses the shared fallback"
fallback_cache="$(sed -n '2p' "$case_dir/report")"
case "$fallback_cache" in
  "$case_dir/host-tmp/sjel-cargo-hermetic/target/hermetic-"*) ;;
  *) note_fail "the fallback did not use the checkout-keyed shared cache" ;;
esac
fallback_run test
expect_ok "the shared fallback survives a second invocation"
[ "$(sed -n '2p' "$case_dir/report")" = "$fallback_cache" ] || note_fail "the fallback cache changed between invocations"
fallback_run test --target-dir "$case_dir/blocked"
expect_refused "an explicit unwritable cache is not silently redirected" "explicit target cannot create and remove"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "cargo-hermetic.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "cargo-hermetic.test.sh: all cases passed"

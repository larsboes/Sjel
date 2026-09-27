#!/bin/bash
# Planted-tree regression tests for check-store-transactions.sh.
#
# The gate walks `find . -name '*.rs'` from its working directory, so each case
# is a throwaway tree the test cds into. The red path is proven here rather than
# assumed: a gate nobody has watched fail is the sixth silent failure in PRD
# §13.1 — an instrument that answers the same for a known-bad input as for a
# known-good one is measuring something other than what was asked.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-store-transactions.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

tree() { # tree <case-name> -> echoes the fresh tree root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root"
  printf '%s' "$root"
}

run() { # run <tree-root> -> gate output on stdout+stderr, exit status in $status
  local root="$1"
  out=$(cd "$root" && "$CHECK" 2>&1)
  status=$?
}

expect_pass() { # expect_pass <description> <tree-root>
  run "$2"
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $1 should pass, got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

expect_fail_with() { # expect_fail_with <description> <tree-root> <substring>
  run "$2"
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$3"; then
    echo "FAIL: $1 should fail naming '$3', got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# --- the red path, which is the whole reason this file exists ---------------

root=$(tree deferred-in-capability)
mkdir -p "$root/capabilities/comms/src"
cat > "$root/capabilities/comms/src/store.rs" <<'RS'
fn set_status(&self) -> Result<(), Box<dyn Error>> {
    let mut conn = self.conn()?;
    let transaction = conn.transaction()?;
    transaction.commit()?;
    Ok(())
}
RS
expect_fail_with "a deferred transaction in a capability is refused" "$root" \
  "capabilities/comms/src/store.rs"

# --- the green path --------------------------------------------------------

root=$(tree immediate-in-capability)
mkdir -p "$root/capabilities/comms/src"
cat > "$root/capabilities/comms/src/store.rs" <<'RS'
fn set_status(&self) -> Result<(), Box<dyn Error>> {
    let mut conn = self.conn()?;
    let transaction = sjel_store::write_transaction(&mut conn)?;
    transaction.commit()?;
    Ok(())
}
RS
expect_pass "the helper is what a capability is supposed to call" "$root"

# `transaction_with_behavior(...)` must not match: the pattern requires the
# parenthesis to close immediately, and this is the form the gate is steering
# toward, not away from.
root=$(tree with-behavior-not-matched)
mkdir -p "$root/libs/other/src"
cat > "$root/libs/other/src/lib.rs" <<'RS'
let t = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
RS
expect_pass "transaction_with_behavior is not the deferred form" "$root"

# --- the exemption is a path, and it is exactly one path -------------------

# The owner's own file carries the deferred spelling and must not be reported.
# It needs a caller beside it: a tree holding nothing BUT the owner scans zero
# files, which the empty-sweep guard below refuses on purpose — the two cases
# would otherwise plant the same shape and expect opposite answers.
root=$(tree owner-exempt)
mkdir -p "$root/libs/sjel-store/src" "$root/capabilities/x/src"
cat > "$root/libs/sjel-store/src/lib.rs" <<'RS'
// The library that owns the primitive may spell it either way.
let t = conn.transaction()?;
RS
echo "fn ok() {}" > "$root/capabilities/x/src/lib.rs"
expect_pass "libs/sjel-store owns both spellings" "$root"

root=$(tree lookalike-not-exempt)
mkdir -p "$root/libs/sjel-store-extras/src"
cat > "$root/libs/sjel-store-extras/src/lib.rs" <<'RS'
let t = conn.transaction()?;
RS
expect_fail_with "a path that merely starts like the owner is not exempt" "$root" \
  "libs/sjel-store-extras/src/lib.rs"

# --- a nested checkout is not part of this tree ----------------------------
#
# The failure this guards was live on 2026-09-08: a fleet of agents left full
# copies of the repository under .claude/worktrees/, each carrying its own
# libs/sjel-store/src/lib.rs, and the gate reported eight failures against files
# that were not the tree it was asked about. CI never saw it, because CI checks
# out clean.

root=$(tree nested-checkout-pruned)
mkdir -p "$root/.claude/worktrees/wf-1/libs/sjel-store/src" "$root/capabilities/x/src"
echo "let t = conn.transaction()?;" > "$root/.claude/worktrees/wf-1/libs/sjel-store/src/lib.rs"
mkdir -p "$root/.claude/worktrees/wf-1/capabilities/y/src"
echo "let t = conn.transaction()?;" > "$root/.claude/worktrees/wf-1/capabilities/y/src/lib.rs"
echo "fn ok() {}" > "$root/capabilities/x/src/lib.rs"
expect_pass "a checkout nested under .claude/ is somebody else's tree" "$root"

root=$(tree node-modules-pruned)
mkdir -p "$root/dashboard/node_modules/pkg" "$root/capabilities/x/src"
echo "let t = conn.transaction()?;" > "$root/dashboard/node_modules/pkg/vendored.rs"
echo "fn ok() {}" > "$root/capabilities/x/src/lib.rs"
expect_pass "a dependency's vendored source is not this repository's code" "$root"

# --- the gate must not report green over an empty sweep --------------------
#
# The failure this guards is the one the sibling gate already hit: a broken
# `find` reports no matches, and no matches reads as "nothing wrong".

root=$(tree no-rust-at-all)
mkdir -p "$root/docs"
echo "# not rust" > "$root/docs/readme.md"
expect_fail_with "an empty sweep is a broken gate, not a clean tree" "$root" \
  "no *.rs files scanned"

# A tree whose only Rust file is inside the owner also scans nothing, and must
# fail for the same reason rather than passing on the owner's exemption.
root=$(tree only-owner-rust)
mkdir -p "$root/libs/sjel-store/src"
echo "let t = conn.transaction()?;" > "$root/libs/sjel-store/src/lib.rs"
expect_fail_with "a sweep that skips everything it found is still empty" "$root" \
  "no *.rs files scanned"

# --- target/ is build output, not source ----------------------------------

root=$(tree target-ignored)
mkdir -p "$root/target/debug/build/x/out" "$root/capabilities/x/src"
echo "let t = conn.transaction()?;" > "$root/target/debug/build/x/out/generated.rs"
echo "fn ok() {}" > "$root/capabilities/x/src/lib.rs"
expect_pass "generated code under target/ is not the tree's source" "$root"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "check-store-transactions.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "check-store-transactions.test.sh: all cases passed"

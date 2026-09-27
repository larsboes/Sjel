#!/bin/bash
# Proves tools/audit's exit contract and how it decides what a repository is.
# 0 clean · 1 a finding · 2 a scanner is not installed, and a finding outranks a missing
# scanner. Bash 3.2-safe.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
MOCK_BIN="$SCRATCH/bin"
GITLEAKS_ONLY_BIN="$SCRATCH/bin-gitleaks-only"
mkdir -p "$MOCK_BIN" "$GITLEAKS_ONLY_BIN"

# Repository detection must use Git plumbing: linked worktrees have a .git file,
# not a directory. Run the real audit script against isolated repositories with
# a deterministic gitleaks mock and an osv-scanner stub that always reports clean,
# so a non-zero exit can only have come from the half under test.
PRIMARY="$SCRATCH/primary"
LINKED="$SCRATCH/linked"
NONREPO="$SCRATCH/not-a-repo"
MISSING="$SCRATCH/not-reachable"
AUDIT_FIXTURE="$SCRATCH/audit-fixture"
GITLEAKS_LOG="$SCRATCH/gitleaks.log"
mkdir -p "$NONREPO" "$AUDIT_FIXTURE/tools/lib"

git init -q "$PRIMARY"
git -C "$PRIMARY" -c user.name=Axon -c user.email=axon@example.invalid \
  commit -q --allow-empty -m initial
git -C "$PRIMARY" worktree add -q -b audit-linked "$LINKED"

cp "$ROOT/tools/audit" "$AUDIT_FIXTURE/tools/audit"
cat > "$AUDIT_FIXTURE/tools/lib/paths.sh" <<'PATHS'
SJEL_ROOT="$SJEL_AUDIT_TEST_ROOT"
SJEL_PERSONAL_ROOT="${SJEL_AUDIT_TEST_OVERLAY:-}"
export SJEL_ROOT SJEL_PERSONAL_ROOT
PATHS

cat > "$MOCK_BIN/gitleaks" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$SJEL_AUDIT_GITLEAKS_LOG"
exit "${SJEL_AUDIT_GITLEAKS_RC:-0}"
MOCK
cat > "$MOCK_BIN/osv-scanner" <<'MOCK'
#!/bin/sh
exit 0
MOCK
cp "$MOCK_BIN/gitleaks" "$GITLEAKS_ONLY_BIN/gitleaks"
chmod +x "$MOCK_BIN/gitleaks" "$MOCK_BIN/osv-scanner" \
  "$GITLEAKS_ONLY_BIN/gitleaks" "$AUDIT_FIXTURE/tools/audit"

run_gitleaks_audit() {
  SJEL_AUDIT_GITLEAKS_LOG="$GITLEAKS_LOG" \
  SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  SJEL_AUDIT_TEST_OVERLAY="$1" \
  SJEL_AUDIT_GITLEAKS_RC="${2:-0}" \
  PATH="$MOCK_BIN:$PATH" \
    "$AUDIT_FIXTURE/tools/audit"
}

: > "$GITLEAKS_LOG"
run_gitleaks_audit "$LINKED" >"$SCRATCH/worktree.out" 2>&1 || {
  cat "$SCRATCH/worktree.out"
  echo "FAIL: linked worktree audit did not complete cleanly" >&2
  exit 1
}
grep -F -- "-s $PRIMARY " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: primary repository was not scanned" >&2
  exit 1
}
grep -F -- "-s $LINKED " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: linked worktree was not scanned" >&2
  exit 1
}
if grep -F "$LINKED" "$SCRATCH/worktree.out" >/dev/null; then
  echo "FAIL: audit output exposed the private overlay coordinate" >&2
  exit 1
fi
grep -F 'private overlay — clean' "$SCRATCH/worktree.out" >/dev/null || {
  echo "FAIL: linked worktree clean status was not explicit" >&2
  exit 1
}

: > "$GITLEAKS_LOG"
if run_gitleaks_audit "$NONREPO" >"$SCRATCH/nonrepo.out" 2>&1; then
  echo "FAIL: non-repository overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not a Git repository' "$SCRATCH/nonrepo.out" >/dev/null || {
  echo "FAIL: non-repository status was not explicit" >&2
  exit 1
}
if grep -F "$NONREPO" "$GITLEAKS_LOG" >/dev/null; then
  echo "FAIL: gitleaks was invoked for a non-repository" >&2
  exit 1
fi

if run_gitleaks_audit "$MISSING" >"$SCRATCH/missing.out" 2>&1; then
  echo "FAIL: unreachable overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not reachable' "$SCRATCH/missing.out" >/dev/null || {
  echo "FAIL: unreachable status was not explicit" >&2
  exit 1
}

run_gitleaks_audit "" >"$SCRATCH/unconfigured.out" 2>&1 || {
  echo "FAIL: an unconfigured optional overlay failed the Axon audit" >&2
  exit 1
}
grep -F 'private overlay — not configured' "$SCRATCH/unconfigured.out" >/dev/null || {
  echo "FAIL: unconfigured status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 1 >"$SCRATCH/leak.out" 2>&1; then
  echo "FAIL: mocked leak produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — leak(s) found' "$SCRATCH/leak.out" >/dev/null || {
  echo "FAIL: leak status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 2 >"$SCRATCH/error.out" 2>&1; then
  echo "FAIL: mocked scanner error produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — gitleaks errored (exit 2' "$SCRATCH/error.out" >/dev/null || {
  echo "FAIL: scanner-error status was not explicit" >&2
  exit 1
}

# --- the exit contract ----------------------------------------------------
#
# A scanner that is not installed is a setup error, not a finding. Until 2026-09-02 a 127
# fell into the finding branch, so this Mac reported fabricated security findings on every
# run — a gate that cries wolf over an absent binary is worse than no gate.

env PATH="/usr/bin:/bin" SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  "$AUDIT_FIXTURE/tools/audit" >"$SCRATCH/nobin.out" 2>&1
nobin_rc=$?
[ "$nobin_rc" -eq 2 ] || {
  cat "$SCRATCH/nobin.out"
  echo "FAIL: audit with no scanner on PATH must exit 2, got $nobin_rc" >&2
  exit 1
}
grep -F 'not installed' "$SCRATCH/nobin.out" >/dev/null || {
  echo "FAIL: missing scanner was not named" >&2
  exit 1
}
if grep -F 'finding(s)' "$SCRATCH/nobin.out" >/dev/null; then
  cat "$SCRATCH/nobin.out"
  echo "FAIL: a missing scanner was reported as a finding" >&2
  exit 1
fi

# Precedence: a real finding outranks a missing scanner, so a run with both exits 1.
# Otherwise a leak would be reported under the exit code that means "nothing was scanned".
: > "$GITLEAKS_LOG"
env PATH="$GITLEAKS_ONLY_BIN:/usr/bin:/bin" \
  SJEL_AUDIT_GITLEAKS_LOG="$GITLEAKS_LOG" \
  SJEL_AUDIT_GITLEAKS_RC=1 \
  SJEL_AUDIT_TEST_ROOT="$PRIMARY" \
  "$AUDIT_FIXTURE/tools/audit" >"$SCRATCH/both.out" 2>&1
both_rc=$?
[ "$both_rc" -eq 1 ] || {
  cat "$SCRATCH/both.out"
  echo "FAIL: a finding beside a missing scanner must exit 1, got $both_rc" >&2
  exit 1
}
grep -F 'not installed' "$SCRATCH/both.out" >/dev/null || {
  echo "FAIL: the missing scanner was not reported alongside the finding" >&2
  exit 1
}

echo "audit exit contract and repository detection tests: pass"

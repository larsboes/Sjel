#!/bin/bash
# Regression tests for tools/githooks/pre-push. The hook's whole job is which commits it scans
# and when it refuses to scan, so each case proves that decision rather than a message: the
# range handed to gitleaks, the exit code, and the deleted-ref and no-scanner paths.
#
# Nothing here reads the developer's repository: gitleaks is replaced by a fake on PATH that
# records its argv and exits with whatever the case asks for.
set -uo pipefail

HOOK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/githooks/pre-push"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

LOG="$SCRATCH/gitleaks.log"
BIN="$SCRATCH/bin"
mkdir -p "$BIN"

cat > "$BIN/gitleaks" <<'FAKE'
#!/bin/bash
printf '%s\n' "$*" >> "$GITHOOKS_TEST_LOG"
exit "${GITHOOKS_TEST_EXIT:-0}"
FAKE
chmod +x "$BIN/gitleaks"

fails=0
status=0

# run <path-dir-or-empty> <stdin> ; leaves the exit code in $status
run() {
  : > "$LOG"
  if [ -n "$1" ]; then path="$1:/usr/bin:/bin"; else path="/usr/bin:/bin"; fi
  printf '%s\n' "$2" | env PATH="$path" GITHOOKS_TEST_LOG="$LOG" \
    GITHOOKS_TEST_EXIT="${MOCK_EXIT:-0}" bash "$HOOK" 2>"$SCRATCH/stderr"
  status=$?
}

expect() { # expect <what> <needle> <file>
  if ! grep -qF "$2" "$3"; then
    echo "FAIL: $1 — expected to find '$2'"
    fails=$((fails + 1))
  fi
}

# --- a clean push ---------------------------------------------------------------------------
run "$BIN" "refs/heads/main aaaa1111 refs/heads/main bbbb2222"
[ "$status" -eq 0 ] || { echo "FAIL: a clean push should exit 0, got $status"; fails=$((fails + 1)); }
expect "the outgoing range is scoped to what no remote has" \
  "git --redact --no-banner --exit-code 1 --log-opts=aaaa1111 --not --remotes" "$LOG"

# --- a finding refuses the push -------------------------------------------------------------
MOCK_EXIT=1 run "$BIN" "refs/heads/main aaaa1111 refs/heads/main bbbb2222"
[ "$status" -eq 1 ] || { echo "FAIL: a finding should exit 1, got $status"; fails=$((fails + 1)); }
expect "the refusal says what to do" "fix or remove it, then push again" "$SCRATCH/stderr"

# --- a deleted ref has no commits to scan ---------------------------------------------------
run "$BIN" "refs/heads/gone 0000000000000000000000000000000000000000 refs/heads/gone bbbb2222"
[ "$status" -eq 0 ] || { echo "FAIL: a deleted ref should exit 0, got $status"; fails=$((fails + 1)); }
[ -s "$LOG" ] && { echo "FAIL: a deleted ref must not reach gitleaks"; fails=$((fails + 1)); }

# --- two refs from one commit are one scan --------------------------------------------------
run "$BIN" "refs/heads/main aaaa1111 refs/heads/main 0000
refs/tags/v1 aaaa1111 refs/tags/v1 0000"
[ "$(wc -l < "$LOG" | tr -d ' ')" = "1" ] \
  || { echo "FAIL: two refs at one commit should be one gitleaks call"; fails=$((fails + 1)); }

# --- a missing scanner warns and does not block ---------------------------------------------
run "" "refs/heads/main aaaa1111 refs/heads/main bbbb2222"
[ "$status" -eq 0 ] || { echo "FAIL: a missing gitleaks should not block a push, got $status"; fails=$((fails + 1)); }
expect "the missing scan is said out loud" "NOT secret-scanned" "$SCRATCH/stderr"

# --- the real gitleaks, when it is installed -------------------------------------------------
# The mock proves which arguments the hook builds; only the real binary proves the invocation is
# valid. That distinction is why this case exists: a bare `--exit-code` is accepted by the mock
# and rejected by gitleaks 8.30.1, which takes an INT for it, so every push would have failed on
# a parse error that had nothing to do with a secret. Skipped where gitleaks is absent, which is
# the case the hook already handles above.
if command -v gitleaks >/dev/null 2>&1; then
  mkdir -p "$SCRATCH/real"
  (
    cd "$SCRATCH/real" || exit 1
    git init -q .
    git config user.email test@example.invalid
    git config user.name test
    printf 'no secret here\n' > file.txt
    git add file.txt
    git commit -qm one
    printf 'refs/heads/main %s refs/heads/main %s\n' \
      "$(git rev-parse HEAD)" "$(git rev-parse HEAD)" | bash "$HOOK" >/dev/null 2>"$SCRATCH/real-stderr"
  )
  real_status=$?
  [ "$real_status" -eq 0 ] \
    || { echo "FAIL: the real gitleaks rejected the hook's invocation (exit $real_status)"; fails=$((fails + 1)); }

  # And the other direction, because "scanned and found nothing" also describes a hook that
  # scanned the wrong range. The shape matters: `ghp_` + 36 is the GitHub PAT rule, while the AWS
  # documentation example key is on gitleaks' own allowlist and passes — which is what the first
  # attempt at this case proved, at the price of a test that could never fail.
  (
    cd "$SCRATCH/real" || exit 1
    printf 'token = "ghp_%s"\n' "0123456789abcdefghijklmnopqrstuvwx" >> file.txt
    git add file.txt
    git commit -qm two
    printf 'refs/heads/main %s refs/heads/main %s\n' \
      "$(git rev-parse HEAD)" "$(git rev-parse HEAD)" | bash "$HOOK" >/dev/null 2>&1
  )
  leak_status=$?
  [ "$leak_status" -eq 1 ] \
    || { echo "FAIL: a planted token should refuse the push (exit $leak_status)"; fails=$((fails + 1)); }
fi

if [ "$fails" -eq 0 ]; then
  echo "✓ tools/githooks: all cases pass"
else
  echo "✗ tools/githooks: $fails case(s) failed"
  exit 1
fi

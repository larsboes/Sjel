#!/bin/bash
# End-to-end tests for tools/ci-local, over planted checkouts.
#
# The tool's job is to say the same thing about this tree that CI will say about the push.
# So the case that matters is the red one: a gate that fails must come back as a failure
# here, named, with the rest of the job still measured. A runner that answers the same for
# a known-bad input as for a known-good one is measuring something other than the gates —
# the sixth silent failure in PRD §13.1.
#
# Every case builds its own checkout under /tmp with its own .github/workflows/ci.yml, so
# nothing here needs a failing gate to exist in the real repository. That is what --root is
# for.
set -uo pipefail

TOOL="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/ci-local"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

checkout() { # checkout <case-name> -> echoes a fresh checkout root with a .github tree
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/.github/workflows" "$root/tools"
  printf '%s' "$root"
}

run() { # run <root> <args...> -> combined output in $out, exit status in $status
  local root="$1"
  shift
  out=$("$TOOL" --root "$root" "$@" 2>&1)
  status=$?
}

expect_status() { # expect_status <description> <wanted> <root> <args...>
  local what="$1" want="$2"
  shift 2
  run "$@"
  if [ "$status" -ne "$want" ]; then
    echo "FAIL: $what — wanted exit $want, got $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
    return 1
  fi
  return 0
}

expect_says() { # expect_says <description> <substring>   (reads the last run's $out)
  if ! printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1 — output does not name '$2':"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

two_step_workflow() { # two_step_workflow <root> <first-command> <second-command>
  cat > "$1/.github/workflows/ci.yml" <<EOF
name: CI
on: [push]
jobs:
  repo-gates:
    name: repo gates
    runs-on: ubuntu-latest
    steps:
      - name: Check out Sjel
        uses: actions/checkout@v5
      - name: first gate
        run: $2
      - name: second gate
        run: $3
EOF
}

# --- the green path: both gates pass ---------------------------------------

root=$(checkout all-green)
cat > "$root/tools/first.sh" <<'SH'
echo "first gate passed"
SH
cat > "$root/tools/second.sh" <<'SH'
echo "second gate passed"
SH
chmod +x "$root/tools/first.sh" "$root/tools/second.sh"
two_step_workflow "$root" "tools/first.sh" "tools/second.sh"
if expect_status "a job whose gates all pass" 0 "$root" run repo-gates; then
  expect_says "the green run reports its own step count" "2 steps passed"
  expect_says "a gate's own stdout reaches the operator" "first gate passed"
fi

# --- the red path, which is the whole reason this file exists ---------------
#
# Same tree, same runner, one planted failing gate. The verdict must invert, the failing
# step must be named, and the step AFTER it must still have run: stopping at the first
# failure hides how much of the repository is broken behind whichever gate sorts first.

root=$(checkout one-red)
cat > "$root/tools/first.sh" <<'SH'
echo "planted failure: this gate refuses" >&2
exit 1
SH
cat > "$root/tools/second.sh" <<'SH'
echo "second gate passed"
SH
chmod +x "$root/tools/first.sh" "$root/tools/second.sh"
two_step_workflow "$root" "tools/first.sh" "tools/second.sh"
if expect_status "a planted failing gate turns the run red" 1 "$root" run repo-gates; then
  expect_says "the failing step is named in the summary" "first gate"
  expect_says "the count separates failures from the total" "1 of 2 steps FAILED"
  expect_says "the gate's own reason is not swallowed" "planted failure"
  expect_says "the step after the failure still ran" "second gate passed"
fi

# A step that fails only through a pipe: `bash -e` alone would let this pass, and CI runs
# these with pipefail. The runner has to reproduce the shell CI uses, not a friendlier one.
root=$(checkout red-through-a-pipe)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
name: CI
on: [push]
jobs:
  repo-gates:
    steps:
      - name: piped gate
        run: |
          set -uo pipefail
          false | cat
EOF
expect_status "a failure on the left of a pipe is still a failure" 1 "$root" run repo-gates

# --- fail-closed refusals --------------------------------------------------

root=$(checkout refused-job)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
name: CI
on: [push]
jobs:
  ui-check:
    steps:
      - name: Minimal overlay for the config read
        run: echo "this would write into the operator's overlay"
EOF
if expect_status "a job that would write into the real overlay is refused" 2 "$root" run ui-check; then
  expect_says "the refusal says what the damage would be" "machine.toml"
fi

root=$(checkout unclassified-job)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
name: CI
on: [push]
jobs:
  brand-new-job:
    steps:
      - name: whatever this is
        run: echo hello
EOF
if expect_status "a job nobody has classified is refused, not run" 2 "$root" run brand-new-job; then
  expect_says "the refusal says what to do about it" "not classified"
fi

# A job made only of `uses:` steps does no work. Reporting it green would be the
# empty-sweep failure tools/check-store-transactions.sh guards against on its own input.
root=$(checkout no-run-steps)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
name: CI
on: [push]
jobs:
  repo-gates:
    steps:
      - name: Check out Sjel
        uses: actions/checkout@v5
EOF
if expect_status "a job with nothing to run is refused, not passed" 2 "$root" run repo-gates; then
  expect_says "the empty job says why it is not a pass" "nothing would be checked"
fi

root=$(checkout no-such-job)
two_step_workflow "$root" "true" "true"
expect_status "a job name that is not in ci.yml is an error" 2 "$root" run cargo-tests

root=$(checkout no-workflow)
rm -rf "$root/.github"
expect_status "a checkout with no ci.yml is an error, not an empty pass" 2 "$root" run repo-gates

# --- against the real workflow ---------------------------------------------
#
# `list` reads .github/workflows/ci.yml and never runs anything, so it is safe to point at
# the real checkout. This is what proves the parse works on the file that matters, rather
# than only on the fixtures above.

out=$("$TOOL" --root "$REPO" list 2>&1)
status=$?
if [ "$status" -ne 0 ]; then
  echo "FAIL: list against the real checkout — exit $status:"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
else
  expect_says "the real repo-gates job is listed as runnable here" "repo-gates"
  expect_says "the real bun-tests job is listed as runnable here" "bun-tests"
  expect_says "the cargo job is listed as refused" "cargo-hermetic"
fi

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "ci-local.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "ci-local.test.sh: all cases passed"

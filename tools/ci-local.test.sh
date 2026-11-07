#!/bin/bash
# Replay and pre-push behavior against disposable checkouts.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
TOOL="$REPO/tools/ci-local"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fails=0

checkout() {
  local root="$SCRATCH/$1"
  mkdir -p "$root/.github/workflows" "$root/tools"
  printf '%s' "$root"
}

run() {
  local root="$1"
  shift
  out=$("$TOOL" --root "$root" "$@" 2>&1)
  status=$?
}

expect_status() {
  local what="$1" want="$2"
  shift 2
  run "$@"
  if [ "$status" -ne "$want" ]; then
    echo "FAIL: $what — wanted exit $want, got $status:"
    printf '%s\n' "$out"
    fails=$((fails + 1))
    return 1
  fi
}

expect_says() {
  if ! printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1 — output does not name '$2':"
    printf '%s\n' "$out"
    fails=$((fails + 1))
  fi
}

two_step_workflow() {
  cat > "$1/.github/workflows/ci.yml" <<EOF
name: CI
on: [push]
jobs:
  repo-gates:
    steps:
      - uses: actions/checkout@v5
      - name: first gate
        run: $2
      - name: second gate
        run: $3
EOF
}

root=$(checkout all-green)
two_step_workflow "$root" "echo first gate passed" "echo second gate passed"
if expect_status "all gates pass" 0 "$root" run repo-gates; then
  expect_says "correct step count" "2 steps passed"
  expect_says "step stdout is preserved" "first gate passed"
fi

root=$(checkout one-red)
two_step_workflow "$root" "echo planted failure >&2; exit 1" "echo second gate passed"
if expect_status "a failing gate blocks" 1 "$root" run repo-gates; then
  expect_says "failed step is named" "first gate"
  expect_says "failure count" "1 of 2 steps FAILED"
  expect_says "failure reason is preserved" "planted failure"
  expect_says "later steps still run" "second gate passed"
fi

root=$(checkout red-through-a-pipe)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  repo-gates:
    steps:
      - run: |
          set -uo pipefail
          false | cat
EOF
expect_status "pipefail is preserved" 1 "$root" run repo-gates

root=$(checkout working-directory)
mkdir "$root/package"
printf 'package fixture\n' > "$root/package/input"
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  bun-tests:
    steps:
      - run: test -f input
        working-directory: package
EOF
expect_status "working directory is honored" 0 "$root" run bun-tests

root=$(checkout refused-job)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  ui-check:
    steps:
      - run: echo unsafe
EOF
if expect_status "destructive jobs refuse" 2 "$root" run ui-check; then
  expect_says "refusal names the damage" "machine.toml"
fi

root=$(checkout unclassified-job)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  brand-new-job:
    steps:
      - run: echo hello
EOF
if expect_status "unknown jobs refuse" 2 "$root" run brand-new-job; then
  expect_says "classification is required" "not classified"
fi

root=$(checkout no-run-steps)
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  repo-gates:
    steps:
      - uses: actions/checkout@v5
EOF
if expect_status "empty work refuses" 2 "$root" run repo-gates; then
  expect_says "empty job explanation" "nothing would be checked"
fi

root=$(checkout no-such-job)
two_step_workflow "$root" "true" "true"
expect_status "unknown job name errors" 2 "$root" run cargo-tests
root=$(checkout no-workflow)
rm -rf "$root/.github"
expect_status "missing workflow errors" 2 "$root" run repo-gates

root=$(checkout full-adapters)
cat > "$root/tools/ci-run.ts" <<'EOF'
console.log(process.argv.slice(2).join(" "));
if (!process.argv.includes("--full")) process.exit(1);
EOF
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  repo-gates:
    needs: changes
    if: needs.changes.outputs.gates == 'true'
    steps:
      - name: Read verification plan
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh run download "$GITHUB_RUN_ID" --name verification-plan --dir "$RUNNER_TEMP"
      - run: bun tools/ci-run.ts gates
  bun-tests:
    needs: changes
    if: needs.changes.outputs.bun == 'true'
    steps:
      - name: Read verification plan
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh run download "$GITHUB_RUN_ID" --name verification-plan --dir "$RUNNER_TEMP"
      - run: bun tools/ci-run.ts bun-tests
EOF
if expect_status "owned conditional wrappers replay full" 0 "$root" run repo-gates bun-tests; then
  expect_says "gates force full" "gates --full"
  expect_says "tests force full" "bun-tests --full"
fi
cat > "$root/.github/workflows/ci.yml" <<'EOF'
jobs:
  repo-gates:
    needs: changes
    if: github.ref == 'refs/heads/main'
    steps:
      - run: bun tools/ci-run.ts gates
EOF
expect_status "unknown wrapper conditions still refuse" 2 "$root" run repo-gates

root=$(checkout pre-push)
git -C "$root" init -q
mkdir "$root/bin"
cat > "$root/bin/gitleaks" <<'EOF'
#!/bin/sh
printf 'gitleaks %s\n' "$*" >> calls
exit "${LEAK_EXIT:-0}"
EOF
cat > "$root/bin/bun" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$root/tools/check-publication-hygiene.sh" <<'EOF'
#!/bin/sh
printf 'publication\n' >> calls
exit "${PUBLICATION_EXIT:-0}"
EOF
cat > "$root/tools/ci-local" <<'EOF'
#!/bin/sh
printf 'ci-local %s\n' "$*" >> calls
exit 0
EOF
chmod +x "$root/bin/gitleaks" "$root/bin/bun" "$root/tools/check-publication-hygiene.sh" "$root/tools/ci-local"
hook() {
  (cd "$root" && printf '%s\n' 'refs/heads/main 1111111111111111111111111111111111111111 refs/heads/main 0000000000000000000000000000000000000000' |
    PATH="$root/bin:$PATH" LEAK_EXIT="$1" PUBLICATION_EXIT="$2" SJEL_SKIP_PUSH_CHECKS="$3" bash "$REPO/tools/githooks/pre-push") > "$root/output" 2>&1
  status=$?
  out=$(< "$root/calls")
}
hook 0 0 0
[ "$status" -eq 0 ] || { echo "FAIL: pre-push success"; fails=$((fails + 1)); }
expect_says "gitleaks stays redacted and scoped" "gitleaks git --redact --no-banner --exit-code 1 --log-opts=1111111111111111111111111111111111111111 --not --remotes"
expect_says "publication safeguard runs" "publication"
expect_says "hook uses only fast" "ci-local fast"
: > "$root/calls"
hook 1 0 0
[ "$status" -eq 1 ] || { echo "FAIL: gitleaks failure must block push"; fails=$((fails + 1)); }
if printf '%s' "$out" | grep -qF 'ci-local'; then echo "FAIL: checks ran after secret failure"; fails=$((fails + 1)); fi
: > "$root/calls"
hook 0 1 1
[ "$status" -eq 1 ] || { echo "FAIL: skip must not bypass publication failure"; fails=$((fails + 1)); }
: > "$root/calls"
hook 0 0 1
[ "$status" -eq 0 ] || { echo "FAIL: explicit skip"; fails=$((fails + 1)); }
expect_says "skip retains publication" "publication"
if printf '%s' "$out" | grep -qF 'ci-local'; then echo "FAIL: skip still ran fast"; fails=$((fails + 1)); fi

out=$("$TOOL" --root "$REPO" list --json 2>&1)
status=$?
if [ "$status" -ne 0 ]; then
  echo "FAIL: real workflow list: $out"
  fails=$((fails + 1))
elif ! printf '%s' "$out" | bun -e '
  const jobs = await Bun.stdin.json();
  for (const id of ["repo-gates", "bun-tests"]) {
    const job = jobs.find(j => j.id === id);
    if (!job || job.refusal !== null || !job.steps.length || job.steps.some(s => s.script.includes("gh run download"))) process.exit(1);
  }
  if (!jobs.find(j => j.id === "cargo-tests")?.refusal?.includes("cargo-hermetic")) process.exit(1);
'; then
  echo "FAIL: real gates/Bun adapters must run, cargo replay must refuse: $out"
  fails=$((fails + 1))
fi

if [ "$fails" -ne 0 ]; then
  echo "ci-local.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "ci-local.test.sh: all cases passed"

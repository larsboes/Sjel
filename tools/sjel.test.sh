#!/bin/bash
# tools/sjel.test.sh — public CLI contract: discoverable without the sjel skill.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SJEL="$ROOT/sjel"

fail() { echo "FAIL: $*" >&2; exit 1; }
contains() { case "$1" in *"$2"*) ;; *) fail "expected '$2' in: $1" ;; esac; }

[ -x "$SJEL" ] || fail "sjel is not executable"
[ ! -e "$ROOT/scripts/axapi" ] || fail "legacy scripts/axapi remains"

# This file is the CLI contract, and only that. An assertion about where a Pack lives read
# green for two months against a tree the test never saw, so it was dropped rather than
# inverted; where the Pack lives is Packs/harness/README.md's fact.

out="$("$SJEL" help)"
contains "$out" "capability list"
contains "$out" "pack deploy"
# tools is in the list because it is indexed. The index itself is asserted by the unit tests
# in tools/sjel-cli/src/search.rs, which `cargo test --workspace` runs.
contains "$out" "search <words...>              Search commands, tools, capabilities, Packs and code"
contains "$out" "storage <browse|report|discover|installed|reclaim|target>"
contains "$out" "gates"
contains "$out" "test"
contains "$out" "cargo <args...>"

# The three pre-push verbs. Each dispatches to a tool that owns the work, so what is
# asserted here is the contract of the CLI: the verb exists, its help says what it runs,
# and the tool it delegates to is on disk. Whether the gates pass is ci-local.test.sh's
# question, and whether cargo is fenced is cargo-hermetic.test.sh's.
out="$("$SJEL" help gates)"
contains "$out" "repo-gates job"
contains "$out" "bun-tests job"
contains "$out" "sjel cargo test"
[ -x "$ROOT/tools/ci-local" ] || fail "tools/ci-local is not executable"

out="$("$SJEL" help cargo)"
contains "$out" "--release refused"
contains "$out" "--print-env"
[ -x "$ROOT/tools/cargo-hermetic" ] || fail "tools/cargo-hermetic is not executable"

# `sjel test` and `sjel gates` must name jobs that exist in the workflow they claim to
# replay. A verb pointing at a job CI does not have would fail only when somebody ran it.
for job in repo-gates bun-tests; do
  grep -q "^  $job:" "$ROOT/.github/workflows/ci.yml" ||
    fail "sjel dispatches to CI job '$job', which .github/workflows/ci.yml does not declare"
  _sjel_main="$ROOT/crates/sjel-cli/src/main.rs"
  [ -f "$_sjel_main" ] || _sjel_main="$ROOT/tools/sjel-cli/src/main.rs"
  grep -q "\"run\", \"$job\"" "$_sjel_main" ||
    fail "sjel no longer dispatches to CI job '$job'"
done

out="$("$SJEL" help capability)"
contains "$out" "ingest <url>"
# The four states are the contract `sjel capability health` publishes: a reader has to be
# able to learn that `off` is not a fault without running it on a machine where something
# is off. The unit tests in tools/sjel-cli/src/capability.rs assert the rules themselves.
contains "$out" "down     should be answering and did not"
contains "$out" "off      this machine does not autostart it"
contains "$out" "unknown  declares neither"

out="$("$SJEL" help pack)"
contains "$out" "opencode"

# storage dispatches to a launcher, not to a binary path: the launcher is what resolves the
# overlay and builds the release binary on demand. Asserted as a file rather than by running
# it, because running it on a cold checkout is a cargo build.
out="$("$SJEL" help storage)"
contains "$out" "target [--json]"
contains "$out" "installed [--idle DAYS]"
[ -x "$ROOT/tools/storage/storage" ] || fail "tools/storage/storage is not executable"

echo "sjel CLI contract passed"

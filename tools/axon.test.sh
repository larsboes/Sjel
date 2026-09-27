#!/bin/bash
# tools/axon.test.sh — public CLI contract: discoverable without the Axon skill.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AXON="$ROOT/axon"

fail() { echo "FAIL: $*" >&2; exit 1; }
contains() { case "$1" in *"$2"*) ;; *) fail "expected '$2' in: $1" ;; esac; }

[ -x "$AXON" ] || fail "axon is not executable"
[ ! -e "$ROOT/scripts/axapi" ] || fail "legacy scripts/axapi remains"

# `Packs/axon/pack.toml` was asserted absent here from #102 (the CLI port retired the
# Pack) until 2026-08-25. #137 gave the Axon Pack a dedicated deployer and put the
# manifest back, and this line should have gone with it. It did not, and the assertion
# stayed green for two months because the sh_test declared only `axon` and
# `tools/axon-context` as data: the sandbox never materialized the file the test was
# looking for, so `[ ! -e ... ]` was true about the sandbox and false about the repo.
# PRD Q44 (2026-08-25) retired Bazel, this test started reading the real checkout, and
# it went red on the first run. Dropped rather than inverted — the CLI contract below
# is what this file is for; where the Pack lives is Packs/harness/README.md's fact.

out="$("$AXON" help)"
contains "$out" "capability list"
contains "$out" "pack deploy"
# tools is in the list because it is indexed. `axon search` itself cannot run here — it
# calls tools/capability.sh registry, which hard-fails without a machine.toml — so the index
# is asserted by tools/tool-index.test.sh against the library both use.
contains "$out" "search <words...>              Search commands, tools, capabilities, and Packs"
[ -r "$ROOT/tools/lib/tool-index.sh" ] || fail "tools/lib/tool-index.sh is missing"
contains "$out" "storage <report|apply|target|prune>"
contains "$out" "gates"
contains "$out" "test"
contains "$out" "cargo <args...>"

# The three pre-push verbs. Each dispatches to a tool that owns the work, so what is
# asserted here is the contract of the CLI: the verb exists, its help says what it runs,
# and the tool it delegates to is on disk. Whether the gates pass is ci-local.test.sh's
# question, and whether cargo is fenced is cargo-hermetic.test.sh's.
out="$("$AXON" help gates)"
contains "$out" "repo-gates job"
contains "$out" "bun-tests job"
contains "$out" "sjel cargo test"
[ -x "$ROOT/tools/ci-local" ] || fail "tools/ci-local is not executable"

out="$("$AXON" help cargo)"
contains "$out" "--release refused"
contains "$out" "--print-env"
[ -x "$ROOT/tools/cargo-hermetic" ] || fail "tools/cargo-hermetic is not executable"

# `axon test` and `axon gates` must name jobs that exist in the workflow they claim to
# replay. A verb pointing at a job CI does not have would fail only when somebody ran it.
for job in repo-gates bun-tests; do
  grep -q "^  $job:" "$ROOT/.github/workflows/ci.yml" ||
    fail "axon dispatches to CI job '$job', which .github/workflows/ci.yml does not declare"
  grep -q "run $job" "$AXON" ||
    fail "axon no longer dispatches to CI job '$job'"
done

out="$("$AXON" help capability)"
contains "$out" "ingest <url>"
# The four states are the contract `axon capability health` publishes: a reader has to be
# able to learn that `off` is not a fault without running it on a machine where something
# is off. tools/capability-probe.test.sh asserts the rules themselves.
contains "$out" "down     should be answering and did not"
contains "$out" "off      this machine does not autostart it"
contains "$out" "unknown  declares neither"
[ -r "$ROOT/tools/lib/capability-probe.sh" ] || fail "tools/lib/capability-probe.sh is missing"

out="$("$AXON" help pack)"
contains "$out" "opencode"

# storage dispatches to a launcher, not to a binary path: the launcher is what resolves the
# overlay and builds the release binary on demand. Asserted as a file rather than by running
# it, because running it on a cold checkout is a cargo build.
out="$("$AXON" help storage)"
contains "$out" "target [--json]"
[ -x "$ROOT/tools/storage/storage" ] || fail "tools/storage/storage is not executable"

echo "axon CLI contract passed"

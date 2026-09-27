#!/bin/bash
# End-to-end tests for tools/self's two refusals and its drift report.
#
# tools/self.test.ts covers the pure rollup. This covers the two things that only exist as
# behaviour: `generate` refusing a stale code graph, and `check` showing WHAT differs rather
# than only that something does. Both were watched failing before they were written down —
# `generate` used to record `graph.stale: [...]` and exit 0, which cost a session a wrong
# first diagnosis on 2026-09-07 (commit 55c71a0).
#
# Nothing here writes into the checkout. The stale case runs the refusal path, which by
# definition writes nothing, and the test asserts self.json is byte-identical afterwards
# rather than trusting that. The graph it plants goes through SJEL_SELF_GRAPH, because
# graphify-out/ on a working machine holds a graph somebody built.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
SELF="$ROOT/tools/self"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
out=""
status=0

note_fail() {
  echo "FAIL: $1"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
}

# `--` because the diff lines this asserts on begin with `-`, which grep would read as a flag.
says() { printf '%s' "$out" | grep -qF -- "$1"; }

# --- generate refuses a graph that describes a tree this is not ------------
#
# Two nodes for files that do not exist under a known internal root. One of them is the
# real path from 2026-09-07: dashboard/src/lib/feed/list-cursor.ts, deleted by the cursor
# merge, still held by the graph, and written into self.json as a field rather than treated
# as a reason not to write.

cat > "$SCRATCH/stale-graph.json" <<'JSON'
{"nodes": [
  {"id": "a", "source_file": "capabilities/comms/src/config.rs"},
  {"id": "b", "source_file": "dashboard/src/lib/feed/list-cursor.ts"},
  {"id": "c", "source_file": "capabilities/trips/src/a-module-that-was-deleted.rs"}
]}
JSON

# A copy, compared with cmp, rather than a hash: `shasum` is macOS's spelling and
# `sha1sum` is the GNU one, and this suite runs in both places.
cp "$ROOT/self.json" "$SCRATCH/self.json.before"
out=$(SJEL_SELF_GRAPH="$SCRATCH/stale-graph.json" "$SELF" generate 2>&1); status=$?
[ "$status" -eq 1 ] || note_fail "a stale graph must make generate refuse, got exit $status"
says "2 path(s) this tree does not have" || note_fail "the refusal does not count the stale paths"
says "dashboard/src/lib/feed/list-cursor.ts" || note_fail "the refusal does not name a stale path"
says "graphify update ." || note_fail "the refusal does not say how to fix it"
cmp -s "$SCRATCH/self.json.before" "$ROOT/self.json" ||
  note_fail "generate refused and wrote self.json anyway"

# The other direction — a graph with nothing stale in it must NOT trip this refusal — is
# asserted in tools/self.test.ts against generateWouldBakeStaleGraph, and deliberately not
# here: the only way to watch it from outside is to let `generate` succeed, and a successful
# generate writes the tracked self.json out of whatever fixture this test planted.

# --- check shows the diff --------------------------------------------------
#
# `--against` compares this tree with an artifact that is not the committed one, so the
# stale path can be watched producing a real diff without editing a tracked file.

DRIFTED="$SCRATCH/drifted.json" COMMITTED="$ROOT/self.json" bun -e '
const model = await Bun.file(process.env.COMMITTED).json();
model.units[0].kind = "a-kind-no-unit-has";
await Bun.write(process.env.DRIFTED, JSON.stringify(model, null, 2) + "\n");
'

out=$("$SELF" check --against "$SCRATCH/drifted.json" 2>&1); status=$?
[ "$status" -eq 1 ] || note_fail "a drifted artifact must fail check, got exit $status"
says "is stale" || note_fail "the verdict is missing"
says "-      \"kind\": \"a-kind-no-unit-has\"" ||
  note_fail "the diff does not show the value that is wrong in the committed artifact"
says "self.json (this tree)" || note_fail "the diff does not label which side is which"

# The committed artifact still passes, so the case above measured drift and not a broken
# comparison. Without this, a check that failed on everything would look identical.
out=$("$SELF" check 2>&1); status=$?
[ "$status" -eq 0 ] || note_fail "the committed self.json should pass check, got exit $status"
says "is current" || note_fail "the passing verdict is missing"

# A path that does not exist is an error, not a silent pass over nothing to compare.
out=$("$SELF" check --against "$SCRATCH/no-such-file.json" 2>&1); status=$?
[ "$status" -eq 1 ] || note_fail "check --against a missing file must fail, got exit $status"
says "cannot read" || note_fail "the missing-file error does not say what it could not read"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "self.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "self.test.sh: all cases passed"

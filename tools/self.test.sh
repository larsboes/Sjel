#!/bin/bash
# End-to-end tests for tools/self's generate and its drift report.
#
# The pure rollup is covered by `cargo test -p sjel-cli` (self_model::model). This covers the two
# things that only exist as behaviour: what `generate` writes on a machine with no code graph, and
# `check` showing WHAT differs rather than only that something does.
#
# The graphless-generate case replaced two refusals on 2026-09-29. `generate` used to refuse when
# graphify-out/ was absent, because writing would drop the per-unit `code` counts the committed
# artifact carried (#35) — and `check` narrowed its comparison to the tracked-file layers in
# exactly that situation, so the drift it reported was the one drift nobody could repair. CI is
# that machine, and main sat red behind it while eleven armed Dependabot pull requests queued.
# The counts are fused on read now, so what `generate` writes is the same on every machine. This
# asserts that, and that the artifact carries no code layer left to drop.
#
# Nothing here writes into the checkout. The successful generate is pointed at a scratch path
# with --out, and the test asserts the tracked self.json is byte-identical afterwards rather
# than trusting that.
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

# --- generate works with no code graph, and commits no code layer ----------
#
# A copy, compared with cmp, rather than a hash: `shasum` is macOS's spelling and
# `sha1sum` is the GNU one, and this suite runs in both places.
cp "$ROOT/self.json" "$SCRATCH/self.json.before"

out=$(SJEL_SELF_GRAPH="$SCRATCH/no-graph-here.json" "$SELF" generate --out "$SCRATCH/generated.json" 2>&1)
status=$?
[ "$status" -eq 0 ] || note_fail "generate must succeed without a code graph, got exit $status"
[ -s "$SCRATCH/generated.json" ] || note_fail "generate --out wrote nothing"
cmp -s "$SCRATCH/self.json.before" "$ROOT/self.json" ||
  note_fail "generate --out wrote the tracked self.json anyway"

# The property the whole change is about: the artifact must be reproducible from tracked files, so
# it must not carry a layer that only a graphful machine can produce.
if grep -q '"code"' "$SCRATCH/generated.json"; then
  out="$(grep -n '"code"' "$SCRATCH/generated.json" | head -3)"
  note_fail "the generated artifact carries per-unit code counts"
fi
if grep -q '"graph"' "$SCRATCH/generated.json"; then
  out="$(grep -n '"graph"' "$SCRATCH/generated.json" | head -3)"
  note_fail "the generated artifact carries a graph block"
fi
# The known-good half. Without it, "no code key" would also pass on a file with nothing in it.
units="$(grep -c '"kind"' "$SCRATCH/generated.json")"
[ "$units" -gt 10 ] ||
  note_fail "the generated artifact carries almost no units ($units lines mentioning kind)"

# --- check shows the diff --------------------------------------------------
#
# `--against` compares this tree with an artifact that is not the committed one, so drift can be
# watched producing a real diff without editing a tracked file.

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
# comparison. Without this, a check that failed on everything would look identical. Run with the
# graph pointed at nothing, because the claim is that this verdict does not depend on one.
out=$(SJEL_SELF_GRAPH="$SCRATCH/no-graph-here.json" "$SELF" check 2>&1); status=$?
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

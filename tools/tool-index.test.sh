#!/bin/bash
# Tests for tools/lib/tool-index.sh — the tools/ half of `axon search`.
#
# Driven against planted checkouts rather than against the real tools/ directory, so a case
# describes exactly one shape and stays true when a tool is added or renamed. The exception
# is the last block, which asks the real tools/ one question that no fixture can answer: does
# the index actually cover this repository's own machinery.
#
# `axon search` itself cannot run in CI — it calls tools/capability.sh registry, which
# hard-fails without a machine.toml — which is why the rules live in a library at all.
set -uo pipefail

LIB="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib/tool-index.sh"
REAL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

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

# shellcheck source=lib/tool-index.sh
SJEL_ROOT="$SCRATCH/fixture"
mkdir -p "$SJEL_ROOT/tools"
. "$LIB"

find_in() { # find_in <query> -> $out, $status
  out="$(search_tools "$1")"
  status=$?
}

says() { printf '%s' "$out" | grep -qF -- "$1"; }

# --- the fixture checkout --------------------------------------------------

cat > "$SJEL_ROOT/tools/widget.sh" <<'SH'
#!/bin/bash
# tools/widget.sh — polishes a widget until it shines.
SH

# A launcher and the file it execs. One tool, and the launcher is what a reader should run.
cat > "$SJEL_ROOT/tools/gadget" <<'SH'
#!/usr/bin/env bash
# Thin launcher: gadget's logic lives in gadget.ts.
SH
cat > "$SJEL_ROOT/tools/gadget.ts" <<'TS'
// tools/gadget.ts — counts every gadget on this machine and says which are unowned.
TS

# An older header that names no path. Its first line is the honest summary.
cat > "$SJEL_ROOT/tools/legacy.sh" <<'SH'
#!/bin/bash
# Restore a thing, carefully, without writing into live state.
SH

# Not tools: a test, a fixture and a document. None is a thing to run for a task.
cat > "$SJEL_ROOT/tools/widget.test.sh" <<'SH'
#!/bin/bash
# tools/widget.test.sh — planted cases for the widget polisher.
SH
cat > "$SJEL_ROOT/tools/widget.env.example" <<'SH'
# tools/widget.env.example — a widget's settings, with every value blank.
SH
cat > "$SJEL_ROOT/tools/README.md" <<'MD'
# tools/README.md — what lives here.
MD

# --- matching --------------------------------------------------------------

find_in polish
[ "$status" -eq 0 ] || note_fail "a word from a tool's own header should match"
says "tools/widget.sh" || note_fail "the matching tool is not named"
says "polishes a widget until it shines." || note_fail "the summary is not the tool's own line"

find_in WIDGET
[ "$status" -eq 0 ] || note_fail "matching is case-insensitive"

find_in widg
[ "$status" -eq 0 ] || note_fail "a partial name should match"

# The launcher, matched on words that appear only in the file it execs. Without that the
# index answers a question about `tools/gadget` with `tools/gadget.ts`, which is not the
# file to run.
find_in unowned
[ "$status" -eq 0 ] || note_fail "a launcher should match its implementation's header"
says "tools/gadget " || note_fail "the launcher is not what was named"
if says "tools/gadget.ts"; then
  note_fail "the launcher and its implementation are listed as two tools"
fi
says "counts every gadget" || note_fail "the launcher's summary does not come from its implementation"

find_in "Restore a thing"
[ "$status" -eq 0 ] || note_fail "a header that names no path still gets indexed"
says "Restore a thing, carefully" || note_fail "the fallback summary is not the first comment line"

# --- what is deliberately not a tool ---------------------------------------

find_in planted
[ "$status" -ne 0 ] || note_fail "a *.test.sh is not a tool to run for a task"

find_in "with every value blank"
[ "$status" -ne 0 ] || note_fail "an .example fixture is not a tool"

find_in "what lives here"
[ "$status" -ne 0 ] || note_fail "a document is not a tool"

# --- the miss, which is the whole point ------------------------------------
#
# `axon search` printed four empty headings and exited 0. A search that cannot say no is a
# search whose silence the caller has to guess at.

find_in nothing-in-this-tree-says-this
[ "$status" -eq 0 ] && note_fail "a query that matches nothing must not report a hit"
[ -z "$out" ] || note_fail "a miss must print no rows"

# --- against the real tools/ -----------------------------------------------
#
# One question a fixture cannot answer: is the real directory actually covered. Asserted as
# a floor on the count, not as a list of names — a list here would be the stale second copy
# this index exists to avoid.

SJEL_ROOT="$REAL_ROOT"
rows="$(search_tools "" | wc -l | tr -d ' ')"
scripts="$(find "$REAL_ROOT/tools" -maxdepth 1 -type f \
  ! -name '*.test.sh' ! -name '*.test.ts' ! -name '*.example' ! -name '*.example.*' \
  ! -name '*.md' ! -name '*.json' ! -name '*.toml' | wc -l | tr -d ' ')"
if [ "$rows" -lt 40 ]; then
  out=""
  note_fail "the empty query indexes only $rows tools of the $scripts in tools/ — the index is not reading the directory"
fi

out="$(search_tools backup)"
printf '%s' "$out" | grep -qF "tools/backup.sh" ||
  { note_fail "the real index does not find tools/backup.sh for 'backup'"; }

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "tool-index.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "tool-index.test.sh: all cases passed"

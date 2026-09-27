#!/bin/bash
# check-doc-self-references.sh — the self-reference gate (CI: repo gates).
# A doc must never cite its own repo-relative path as if it were a pointer to
# somewhere else ("see `capabilities/x/README.md`" written inside that very
# file). The class is real: the 2026-07-28 decisions/ dissolution's link
# rewriter left four such self-references (sjel-status, punctuality — two
# each), all reading like cross-references and all going nowhere.
#
# Pure file-based check over the tracked docs, same contract as the sibling
# gates: no git, no network. Only paths with at least two segments
# are checked: a root-level file's repo-relative path is just its basename,
# which legitimately appears in prose ("ARCHITECTURE.md is generated").
#
# Two properties this gate has to hold, both regression-tested in
# tools/check-doc-self-references.test.sh:
#   - every materialized doc is actually read. Iterating an unquoted `find`
#     substitution word-split any path containing a space, grep then errored to
#     stderr on the fragments, and the run still exited 0 — the exact
#     silent-green failure the gate exists to prevent.
#   - a citation only counts when the path stands alone. A fixed-string match
#     is a substring match, so a doc at a/b.md that legitimately mentions
#     a/b.md.bak, or a sibling's docs/a/b.md, was reported as citing itself.
set -e

fail=0
found=0
while IFS= read -r f; do
  rel="${f#./}"
  case "$rel" in
    */*) ;;                      # two+ segments: checkable
    *) continue ;;               # root-level: basename-only, skip (see header)
  esac
  found=$((found + 1))

  # A missing or unreadable entry means the data list and the tree disagree.
  # Say so instead of letting grep's exit 2 read as "no match found".
  if [ ! -r "$f" ]; then
    echo "FAIL [$rel]: declared in data but not readable — the data list is stale" >&2
    fail=1
    continue
  fi

  # ERE-escape the path, then require a non-path character (or a line edge) on
  # both sides, so only a standalone citation matches.
  # `]` leads and `[` trails on purpose: a `[` followed by `.`, `:` or `=` opens a
  # collating symbol, which is why the obvious `[][.*...]` ordering is a syntax
  # error on BSD sed.
  esc=$(printf '%s' "$rel" | sed 's/[]^$\\.*+?(){}|[]/\\&/g')
  pattern="(^|[^[:alnum:]_./-])${esc}($|[^[:alnum:]_./-])"

  if grep -qE "$pattern" "$f"; then
    echo "FAIL [$rel]: cites its own path — a self-reference pretending to be a pointer:" >&2
    grep -nE "$pattern" "$f" | head -3 >&2
    fail=1
  fi
done < <(find . -name '*.md' | sort)

if [ "$found" -eq 0 ]; then
  echo "FAIL: no *.md files materialized — the data list is broken" >&2
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "doc self-reference check FAILED." >&2
  exit 1
fi

echo "doc self-reference check passed ($found docs, none cites its own repo-relative path)."

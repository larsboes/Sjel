#!/bin/bash
# check-generator-inputs-tracked.sh — every file tools/generate-architecture.sh reads off
# disk must be tracked by git (CI: repo gates).
#
# The generator globs capabilities/*/README.md, capabilities/*/service.toml, Packs/*/pack.toml
# and libs/*/README.md. An UNTRACKED manifest feeds real data into ARCHITECTURE.md, and then
# nobody else can reproduce it: the freshness gate passes locally because it globs the same
# working tree, and CI clones and gets a different answer. That is the local-green/CI-red
# shape, and it has bitten this repo before (the 2026-07-16 note about untracked capabilities
# appearing in a committed ARCHITECTURE.md).
#
# Split out of tools/check-bazel-package-labels.sh on 2026-08-25 (PRD Q44). The half that went
# was a declared-label list, which had nothing left to compare once the build stopped being
# package-scoped. Trackedness is a property of the git checkout, so it survived unchanged.
#
# Usage: tools/check-generator-inputs-tracked.sh
# Exit 0 = every generator input is tracked, 1 = at least one is not.
set -e

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

if ! git -C "$SJEL_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
  echo "FAIL: $SJEL_ROOT is not a git checkout, so trackedness cannot be answered" >&2
  exit 1
fi

fail=0
checked=0
for f in "$SJEL_ROOT"/capabilities/*/README.md "$SJEL_ROOT"/capabilities/*/service.toml \
         "$SJEL_ROOT"/Packs/*/pack.toml "$SJEL_ROOT"/libs/*/README.md; do
  [ -f "$f" ] || continue
  checked=$((checked + 1))
  _rel="${f#"$SJEL_ROOT"/}"
  if [ -n "$(git -C "$SJEL_ROOT" ls-files --others --exclude-standard -- "$_rel")" ]; then
    echo "FAIL: $_rel is untracked but the architecture generator reads it — ARCHITECTURE.md would carry data nobody else can reproduce (local green, CI red)" >&2
    fail=1
  fi
done

# A glob that matched nothing means the input set moved and this gate is comparing air.
if [ "$checked" -eq 0 ]; then
  echo "FAIL: no generator input found at all — the globs in this file no longer match the tree" >&2
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "generator-input trackedness check FAILED." >&2
  exit 1
fi

echo "generator-input trackedness check passed ($checked inputs, all tracked)."

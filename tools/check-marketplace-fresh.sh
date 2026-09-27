#!/bin/bash
# check-marketplace-fresh.sh — the marketplace.json / plugin.json freshness gate (CI: repo gates).
# Fails if the generated Claude Code plugin marketplace is stale relative to the
# pack.toml files it is generated from, or carries a stale plugin.json for a pack
# that no longer exists or is no longer generic (see CONTRIBUTING.md, "Generated architecture").
#
# Never writes into the checkout: it regenerates into a scratch root and diffs,
# because a gate that fixes what it finds reports green over a change nobody reviewed.
#
# Usage: tools/check-marketplace-fresh.sh
set -e

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

MARKETPLACE_OUT_ROOT="$scratch" bun "$SJEL_ROOT/tools/generate-marketplace.ts" >/dev/null

fail=0

after_list="$(cd "$scratch" && find .claude-plugin Packs -type f 2>/dev/null | sort)"
before_list="$(cd "$SJEL_ROOT" && find .claude-plugin Packs -type f -path '*.claude-plugin*' 2>/dev/null | sort)"

while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  if [ ! -f "$SJEL_ROOT/$rel" ]; then
    echo "missing: $rel is generated but not tracked" >&2
    fail=1
  elif ! diff -q "$SJEL_ROOT/$rel" "$scratch/$rel" >/dev/null 2>&1; then
    echo "stale: $rel" >&2
    fail=1
  fi
done <<<"$after_list"

while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  if [ ! -f "$scratch/$rel" ]; then
    echo "orphaned: $rel is tracked but no longer generated" >&2
    fail=1
  fi
done <<<"$before_list"

if [ "$fail" -ne 0 ]; then
  echo "marketplace is stale. Run: tools/generate-marketplace.ts" >&2
  exit 1
fi

echo "marketplace.json and plugin.json files are up to date."

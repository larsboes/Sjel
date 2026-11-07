#!/bin/bash
# Enforce script-free installs and per-package adoption/scanner settings.
set -eu

ROOT="${SJEL_BUN_INSTALL_POLICY_ROOT:-.}"
fail=0

require() { # require <path> <literal>
  local path="$1" literal="$2"
  if ! grep -Fq -- "$literal" "$ROOT/$path"; then
    echo "FAIL: $path must contain $literal" >&2
    fail=1
  fi
}

if grep -Fq 'bun tools/ci-run.ts ui-check' "$ROOT/.github/workflows/ci.yml"; then
  require ".github/workflows/ci.yml" "bun tools/ci-run.ts bun-tests"
  require "tools/ci-run.ts" '["bun", "install", "--frozen-lockfile", "--ignore-scripts"]'
  require "tools/ci-run.ts" '["bun", "pm", "scan"]'
else
  require ".github/workflows/ci.yml" "bun install --frozen-lockfile --ignore-scripts"
  require ".github/workflows/ci.yml" "bun pm scan"
fi
if [ -f "$ROOT/crates/comms/clipper/README.md" ]; then
  require "crates/comms/clipper/README.md" "bun install --frozen-lockfile --ignore-scripts"
else
  require "capabilities/comms/clipper/README.md" "bun install --frozen-lockfile --ignore-scripts"
fi

trees=""
while IFS= read -r pkg; do
  [ -n "$pkg" ] || continue
  dir="$(dirname "$pkg")"
  [ -f "$dir/bun.lock" ] || continue
  rel="${dir#$ROOT/}"
  [ -n "$rel" ] || rel="."
  trees="$trees
$rel"
done <<PKGS
$(find "$ROOT" \
  \( -name node_modules -o -name .git -o -name target -o -name .claude -o -name 'bazel-*' \
     -o -path '*/Packs/*/pi-packages/*' \) -prune \
  -o -name package.json -print 2>/dev/null)
PKGS

if [ -z "$trees" ]; then
  echo "FAIL: no tree with a package.json and a bun.lock was found under $ROOT — the derived set is empty, so this check proved nothing." >&2
  fail=1
fi

while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  require "$rel/bunfig.toml" "minimumReleaseAge = 86400"
  require "$rel/bunfig.toml" "@socketsecurity/bun-security-scanner"
done <<TREES
$trees
TREES

if [ "$fail" -ne 0 ]; then
  echo "bun install policy FAILED — lifecycle hooks must stay disabled, and every resolving tree must keep the npm-only hold and its scanner." >&2
  exit 1
fi

echo "bun install policy passed (CI and the clipper README disable lifecycle hooks; every resolving tree carries the 24h hold and the scanner)."

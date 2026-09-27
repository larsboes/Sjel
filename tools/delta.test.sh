#!/bin/bash
# Test for tools/lib/delta.sh — the version-to-version manifest delta. Builds a throwaway git
# repo with two commits whose manifests differ, points SJEL_ROOT at it, and asserts
# print_manifest_delta names each kind of change (capability add/remove, upstream add + verdict
# change, toolchain add).
#
# Run it with: tools/delta.test.sh
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export SJEL_ROOT="$SCRATCH"

git -C "$SCRATCH" init -q
git -C "$SCRATCH" config user.email t@example.com
git -C "$SCRATCH" config user.name test

# --- commit 1: capabilities alpha+beta; upstreams foo(adopt)+bar; toolchain git ---
mkdir -p "$SCRATCH/capabilities/alpha" "$SCRATCH/capabilities/beta"
echo x > "$SCRATCH/capabilities/alpha/README.md"
echo x > "$SCRATCH/capabilities/beta/README.md"
cat > "$SCRATCH/upstreams.toml" <<'EOF'
[foo]
verdict = "adopt"
[bar]
verdict = "inspiration"
EOF
cat > "$SCRATCH/toolchain.toml" <<'EOF'
[git]
required = "yes"
EOF
git -C "$SCRATCH" add -A; git -C "$SCRATCH" commit -qm v1
FROM="$(git -C "$SCRATCH" rev-parse HEAD)"

# --- commit 2: -beta +gamma; foo verdict adopt->overlay, +baz; toolchain +jq ---
rm -rf "$SCRATCH/capabilities/beta"
mkdir -p "$SCRATCH/capabilities/gamma"; echo x > "$SCRATCH/capabilities/gamma/README.md"
cat > "$SCRATCH/upstreams.toml" <<'EOF'
[foo]
verdict = "overlay"
[bar]
verdict = "inspiration"
[baz]
verdict = "adopt"
EOF
cat > "$SCRATCH/toolchain.toml" <<'EOF'
[git]
required = "yes"
[jq]
required = "yes"
EOF
git -C "$SCRATCH" add -A; git -C "$SCRATCH" commit -qm v2
TO="$(git -C "$SCRATCH" rev-parse HEAD)"

# shellcheck source=lib/delta.sh
. "$_dir/lib/delta.sh"
OUT="$(print_manifest_delta "$FROM" "$TO")"

fails=0
want() {  # want <description> <substring that must appear>
  if printf '%s' "$OUT" | grep -qF -- "$2"; then :; else
    echo "FAIL: $1 (missing: $2)"; fails=$((fails + 1))
  fi
}
absent() {  # absent <description> <substring that must NOT appear>
  if printf '%s' "$OUT" | grep -qF -- "$2"; then
    echo "FAIL: $1 (unexpected: $2)"; fails=$((fails + 1))
  fi
}

want   "capability gamma added"   "+ gamma"
want   "capability beta removed"  "- beta"
absent "alpha unchanged, not listed" "alpha"
want   "upstream baz added"       "+ baz"
want   "foo verdict changed"      "~ foo: adopt → overlay"
absent "bar verdict unchanged"    "~ bar"
want   "toolchain jq added"       "+ jq"

if [ "$fails" -gt 0 ]; then
  echo "delta.sh: $fails check(s) failed"
  echo "--- output was ---"; printf '%s\n' "$OUT"
  exit 1
fi
echo "delta.sh: all checks passed"

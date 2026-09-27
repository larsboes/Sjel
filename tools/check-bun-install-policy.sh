#!/bin/bash
# check-bun-install-policy.sh — the frontend install boundary must never execute lifecycle hooks,
# and every tree that resolves dependencies must carry the npm-only adoption hold.
#
# ChainDrop used an npm preinstall hook, before the dependency tree had finished installing. Axon's
# frontend packages are fully resolved by committed Bun lockfiles, and nothing that installs them
# needs lifecycle hooks. Keep that invariant in every place that performs or teaches a frontend
# dependency install.
#
# tools/bazel/bun/deps.bzl was the fourth such place until 2026-08-25, when PRD Q44 retired the
# hermetic bun toolchain along with the rest of Bazel. pages.yml and the soundscape README took
# its place in this list because they install and teach the same thing — pages.yml had been
# doing so unchecked.
#
# THE SECOND INVARIANT, added 2026-09-22 (Q109). Two settings belong in every tree that resolves
# dependencies, and BOTH are per tree rather than repository-wide because Bun does not inherit a
# parent directory's bunfig.toml — measured on 1.4.2 on 2026-09-22, an install run from dashboard/
# ignores ../bunfig.toml. The hold is a real control only where resolution happens:
#
#   [install] minimumReleaseAge = 86400            the npm-only 24h adoption hold
#   [install.security] scanner = "@socketsecurity/bun-security-scanner"
#                                                  the malicious-publish check, read at install
#
# So the set of trees is DERIVED from the committed lockfiles rather than listed, for the reason
# bunfig.toml gives about test discovery: the moment this file names directories, a new tree's
# coverage depends on somebody remembering to come here. A tree is "resolves dependencies" if it
# has a committed package.json beside a bun.lock. Vendored trees are carved out — a copy of
# another project is not this repository's configuration to set, the same carve-out the root
# bunfig's [test] pathIgnorePatterns makes and for the same reason.
#
# What is NOT derived and cannot be: whether a scanner configured in bunfig is actually installed.
# Bun enforces that itself — a configured scanner missing from node_modules is a hard
# SecurityScannerNotInDependencies failure that stops the install (measured 2026-09-22), so the
# dependency cannot drift away from the config silently. This script checks the config line.
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

require ".github/workflows/ci.yml" "bun install --frozen-lockfile --ignore-scripts"
require ".github/workflows/ci.yml" "bun pm scan"
require ".github/workflows/pages.yml" "bun install --frozen-lockfile --ignore-scripts"
require "dashboard/README.md" "bun install --frozen-lockfile --ignore-scripts"
require "capabilities/soundscape/README.md" "bun install --frozen-lockfile --ignore-scripts"

# Trees that resolve dependencies: a package.json with a committed bun.lock beside it.
#
# The prune list is derived trees, not an allow-list of our directories: node_modules and .git
# are not this repository, target/ is cargo's output, .claude/worktrees/ is a scratch checkout of
# this repository (a red gate there is a red gate nobody can act on — the reason the root bunfig
# carries the bazel-* carve-out), and bazel-* is the retired Bazel root symlinks the root bunfig
# already ignores for tests. Upstream's `-path` prune is what carries the vendored carve-out,
# because bash 3.2.57 — the stock macOS shell this repository targets (README.md#portable-shell)
# — cannot PARSE a `case` statement inside a command substitution containing a pipeline: measured
# 2026-09-22, `/bin/bash -c 'X="$(printf a | while read l; do case "$l" in a) ;; esac; done)"'`
# is a syntax error on 3.2.57 and parses on bash 5. Writing the carve-out as a `case` in the
# loop below would therefore have been a script that runs on the machine that edited it and
# fails to parse on the machine that ships it.
#
# The accumulation is newline-separated and consumed by a `while read` on a heredoc, never a
# `for` over an unquoted expansion: a checkout under a path with a space in it is otherwise two
# trees that do not exist. Heredocs keep both loops in THIS shell, so the vendored-branch's
# `continue` and the `fail` flag below are the same variable the rest of this script reads.
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

echo "bun install policy passed (both workflows and both operator READMEs disable lifecycle hooks; every resolving tree carries the 24h hold and the scanner)."

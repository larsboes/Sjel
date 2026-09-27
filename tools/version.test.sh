#!/bin/bash
# Regression test for tools/lib/version.sh. The case that matters is the one that was
# live and wrong on 2026-07-28: arrow-rs published 58.4.0 after 59.1.0, and the checker
# called the older release "newer".
set -uo pipefail

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib"
# shellcheck source=lib/version.sh
. "$_lib/version.sh"

fails=0
check() { # check <description> <expected: yes|no> <command...>
  local desc="$1" expect="$2"; shift 2
  if "$@"; then got=yes; else got=no; fi
  if [ "$got" != "$expect" ]; then
    echo "FAIL: $desc (expected $expect, got $got)"
    fails=$((fails + 1))
  fi
}

# The live regression: a maintenance release published later than a higher minor.
check "59.1.0 > 58.4.0"                  yes ver_gt 59.1.0 58.4.0
check "58.4.0 is NOT > 59.1.0"           no  ver_gt 58.4.0 59.1.0

# Ordinary ordering, including the double-digit trap a lexical sort gets wrong.
check "1.10.0 > 1.9.0 (not lexical)"     yes ver_gt 1.10.0 1.9.0
check "0.71.10 > 0.71.9"                 yes ver_gt 0.71.10 0.71.9
check "equal is not greater"             no  ver_gt 1.2.3 1.2.3
check "patch bump"                       yes ver_gt 1.2.4 1.2.3
check "differing depth: 2.0 vs 2.0.1"    no  ver_gt 2.0 2.0.1

# What must NOT be ordered. These are the shapes a version string actually takes in the
# wild -- a git sha, a tool-prefixed tag, a dated image tag, a base-suffixed image tag --
# and ver_numeric is what stops a caller inventing a comparison for them. They were real
# upstreams.toml pins until 2026-09-02 (Q77 deleted that field); they are kept here
# because toolchain.toml `min_version` floors still flow through this comparator, and a
# floor read off `apfel --version` or `tailscale version` meets the same shapes.
check "plain version is orderable"       yes ver_numeric 59.1.0
check "single component is orderable"    yes ver_numeric 17
check "git sha is not"                   no  ver_numeric f7c4aef
check "cli-v2026.6.0 is not"             no  ver_numeric cli-v2026.6.0
check "ProjectName-v1.1.0 is not"          no  ver_numeric ProjectName-v1.1.0
check "trixie-20260713-slim is not"      no  ver_numeric trixie-20260713-slim
check "17.9-alpine is not"               no  ver_numeric 17.9-alpine
check "empty is not"                     no  ver_numeric ""

# norm_ver strips exactly one leading v, and nothing else.
check "v-prefix stripped"                yes [ "$(norm_ver v8.30.1)" = "8.30.1" ]
check "bare version untouched"           yes [ "$(norm_ver 8.30.1)" = "8.30.1" ]
check "v inside a tag untouched"         yes [ "$(norm_ver cli-v2026.6.0)" = "cli-v2026.6.0" ]

# --- release-line identity ------------------------------------------------
# The falsifier for the hijack this guards against: a tag that is not on the release line must
# not become the reported version. A synthetic repo rather than this checkout, so the assertion
# holds on a machine that happens to carry no tags, or the wrong ones.
_fixture="$(mktemp -d)"
trap 'rm -rf "$_fixture"' EXIT
_g() { git -C "$_fixture" -c user.email=t@example.invalid -c user.name=t -c commit.gpgsign=false "$@" >/dev/null 2>&1; }

_g init -q
printf '[release]\ntag_glob = "v[0-9]*"\n' > "$_fixture/axon.toml"
_g add axon.toml
_g commit -m "release fixture"
_g tag v0.1.0
printf 'second\n' > "$_fixture/later.txt"
_g add later.txt
_g commit -m "after the release"
# The kind of tag that caused this: a marker parked on the same line, not a release.
_g tag archive/dev-pre-public

SJEL_ROOT="$_fixture"; export SJEL_ROOT

# `check` runs a command, and `case` is a keyword — so the pattern tests get real predicates.
starts_with() { case "$2" in "$1"*) return 0 ;; *) return 1 ;; esac; }
contains()    { case "$2" in *"$1"*) return 0 ;; *) return 1 ;; esac; }

check "glob comes from axon.toml"        yes [ "$(release_tag_glob)" = "v[0-9]*" ]
check "describe_release finds v0.1.0"    yes starts_with v0.1.0 "$(describe_release)"
check "a non-release tag never wins"     no  contains archive "$(describe_release)"
# The control: an unrestricted describe DOES pick the marker up, which is the whole point.
check "bare describe is hijacked"        yes starts_with archive/ "$(git -C "$_fixture" describe --tags)"
# Explicit revision: --dirty is illegal with a rev, so this exercises the other branch.
check "describe_release takes a rev"     yes [ "$(describe_release v0.1.0)" = "v0.1.0" ]

if [ "$fails" -gt 0 ]; then
  echo "version.sh: $fails check(s) failed"
  exit 1
fi
echo "version.sh: all checks passed"

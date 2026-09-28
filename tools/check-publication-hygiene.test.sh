#!/bin/bash
# Planted-index regression tests for check-publication-hygiene.sh. The check reads
# Git's index, so each case uses a throwaway repository and never needs private data.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-publication-hygiene.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export SJEL_PUBLICATION_ROOT="$SCRATCH"

git -C "$SCRATCH" init -q
printf '%s\n' 'container example: /home/agent/config' > "$SCRATCH/safe.txt"
git -C "$SCRATCH" add safe.txt

fails=0
expect_pass() {
  if "$CHECK" >/dev/null 2>&1; then :; else
    echo "FAIL: $1 should pass"; fails=$((fails + 1))
  fi
}
expect_fail_with() {
  out="$($CHECK 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1 should fail and name $2"; fails=$((fails + 1))
  fi
}

expect_pass "portable container home"

mkdir -p "$SCRATCH/pkg/__pycache__"
printf '%s\n' 'compiled' > "$SCRATCH/pkg/__pycache__/module.pyc"
git -C "$SCRATCH" add -f pkg/__pycache__/module.pyc
expect_fail_with "tracked bytecode" "pkg/__pycache__/module.pyc"

git -C "$SCRATCH" rm -q --cached pkg/__pycache__/module.pyc
rm -f "$SCRATCH/pkg/__pycache__/module.pyc"
printf '%s\n' '@/''Users/private-user/Developer/project/module.py' > "$SCRATCH/leak.bin"
git -C "$SCRATCH" add leak.bin
expect_fail_with "workstation home in blob metadata" "leak.bin"

printf '%s\n' 'portable metadata' > "$SCRATCH/leak.bin"
git -C "$SCRATCH" add leak.bin
expect_pass "cleaned index"

# The dashboard's own lib/home/ is a directory in this repository, not a home directory.
# Home's decision ladder lives there, so every import specifier below it matched the second
# home marker and turned this gate red on a branch that leaked nothing.
printf '%s\n' 'import mail from "$lib/''home/kinds/mail.ts";' > "$SCRATCH/import.txt"
git -C "$SCRATCH" add import.txt
expect_pass "an import specifier under the repository's own lib/home/"

# ...and the two shapes that exemption must not cost, both of which begin after an
# identifier character: a committed diff's a/ and b/ prefixes, and a relative path.
printf '%s\n' 'diff prefix: a/''Users/private-user/Developer/project/' > "$SCRATCH/diffish.txt"
git -C "$SCRATCH" add diffish.txt
expect_fail_with "a workstation path behind a diff prefix" "diffish.txt"

git -C "$SCRATCH" rm -q --cached diffish.txt
rm -f "$SCRATCH/diffish.txt"
printf '%s\n' 'relative: ../../''Users/private-user/Developer/project/' > "$SCRATCH/relative.txt"
git -C "$SCRATCH" add relative.txt
expect_fail_with "a workstation path written relative" "relative.txt"

git -C "$SCRATCH" rm -q --cached relative.txt
rm -f "$SCRATCH/relative.txt"

printf '%s\n' 'private overlay: axon-family' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_fail_with "named deployment marker" "instance.txt"

# A compound identifier that merely starts with a marker is not a deployment being named.
# `axon-personal-cents` is a journal tag in capabilities/finance and exposes nothing; it failed
# this gate on every push to main until the marker gained a trailing-character guard.
printf '%s\n' 'let tag = "axon-personal-cents";' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_pass "a compound identifier beginning with a marker"

# ...but the marker itself, in a path or on its own, still fails.
printf '%s\n' 'see ~/Developer/sjel-personal/config' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_fail_with "a marker used as a real path" "instance.txt"

printf '%s\n' 'axon-personal' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_fail_with "a marker alone on a line" "instance.txt"

printf '%s\n' 'selected deployment overlay' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_pass "generic deployment language"

if [ "$fails" -gt 0 ]; then
  echo "publication hygiene: $fails check(s) failed"
  exit 1
fi
echo "publication hygiene: all checks passed"

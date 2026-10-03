#!/bin/bash
# Planted-index regression tests for tools/isa-hygiene.sh. The tool reads Git's index, so each
# case uses a throwaway repository and never needs private data.
set -uo pipefail

TOOL="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/isa-hygiene.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export SJEL_ISA_ROOT="$SCRATCH"
# Injected rather than inherited, so these cases do not depend on whose account the tool
# happens to default to.
export SJEL_PRIVATE_USERS="private-user"

git -C "$SCRATCH" init -q
mkdir -p "$SCRATCH/capabilities/media"

# Every case starts from a leak-free index, so a finding can only come from the line it planted.
safe() {
  printf '%s\n' 'safe: no machine identifiers here' > "$SCRATCH/ISA.md"
  printf '%s\n' 'safe: a relative path capabilities/media/src' > "$SCRATCH/capabilities/media/ISA.md"
  rm -rf "$SCRATCH/sub" "$SCRATCH/README.md"
  git -C "$SCRATCH" add -A >/dev/null 2>&1
}
plant() {
  printf '%s\n' "$2" > "$SCRATCH/$1"
  git -C "$SCRATCH" add "$1"
}

fails=0
run() { out="$("$TOOL" 2>&1)"; status=$?; }

expect_exit_zero() {
  run
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $1: expected exit 0, got $status"; fails=$((fails + 1))
  fi
}
expect_names() {
  run
  expect_exit_zero "$1 (exit)"
  if ! printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1: expected \"$2\" in output"; fails=$((fails + 1))
  fi
}
expect_silent_about() {
  run
  expect_exit_zero "$1 (exit)"
  if printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1: did not expect \"$2\" in output"; fails=$((fails + 1))
  fi
}

safe
expect_silent_about "a clean index reports nothing" "/Volumes/"
expect_names "a clean index says so" "no machine-private strings found"

plant "capabilities/media/ISA.md" 'measured on /Volumes/Extreme/Media/Library'
expect_names "a volume path is named" "/Volumes/Extreme"
expect_names "the volume path is labelled" "volume path"

safe
plant "ISA.md" 'the live systems.local.toml holds the backup target'
expect_names "a hostname is named" "systems.local"
expect_names "the hostname is labelled" "hostname"

safe
plant "ISA.md" 'path /Users/private-user/Developer/project'
expect_names "this installation's home path is named" "/Users/private-user/"
expect_names "the home path is labelled" "home path"

safe
plant "ISA.md" 'an address 10.0.0.7 and a loopback 127.0.0.1'
expect_names "a private address is named" "10.0.0.7"
expect_silent_about "loopback is not a machine" "127.0.0.1"

safe
plant "ISA.md" 'the overlay is sjel-personal'
expect_names "an instance marker is named" "sjel-personal"

safe
plant "ISA.md" 'axon.local.toml is the overlay pointer; args.local is a known false positive'
expect_silent_about "documented .local names are not hostnames" "hostname"
expect_names "an allowlisted-only line reports clean" "no machine-private strings found"

safe
printf '%s\n' 'an export at /Volumes/Extreme/Media' > "$SCRATCH/README.md"
git -C "$SCRATCH" add README.md
expect_silent_about "a non-ISA file is out of scope" "/Volumes/Extreme"

safe
mkdir -p "$SCRATCH/sub"
printf '%s\n' 'a leak at /Volumes/Extreme/Media' > "$SCRATCH/sub/ISA.md"
expect_silent_about "an untracked ISA is out of scope" "/Volumes/Extreme"
git -C "$SCRATCH" add sub/ISA.md
expect_names "the same ISA once staged is in scope" "/Volumes/Extreme"

if [ "$fails" -gt 0 ]; then
  echo "isa-hygiene: $fails check(s) failed"
  exit 1
fi
echo "isa-hygiene: all checks passed"

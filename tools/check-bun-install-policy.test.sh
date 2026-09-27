#!/bin/bash
# Planted-tree regression tests for the install policy: lifecycle scripts stay off, and every
# tree that resolves dependencies keeps the npm-only hold and its scanner (Q109).
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-bun-install-policy.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fails=0

# The compliant parts of a planted tree: the two workflows and the two operator READMEs the
# script requires, plus ONE tree that resolves dependencies — a package.json beside a bun.lock
# beside a bunfig with both settings. A tree that is missing any of those three is the thing
# each planted failure below removes; keeping the default complete is what makes the difference
# between the cases the missing file rather than the missing directory.
plant() { # plant <name> <ci-install> [<mode>]
  local root="$SCRATCH/$1" mode="${3:-full}"
  mkdir -p "$root/.github/workflows" "$root/dashboard" "$root/capabilities/soundscape"
  printf '%s\n' "$2" > "$root/.github/workflows/ci.yml"
  printf 'bun pm scan\n' >> "$root/.github/workflows/ci.yml"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/.github/workflows/pages.yml"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/dashboard/README.md"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/capabilities/soundscape/README.md"

  printf '{"name":"planted-ui"}\n' > "$root/dashboard/package.json"
  printf '{}\n' > "$root/dashboard/bun.lock"
  case "$mode" in
    no-bunfig) : ;; # a tree that resolves dependencies and declares nothing
    hold-only)
      printf '[install]\nminimumReleaseAge = 86400\n' > "$root/dashboard/bunfig.toml" ;;
    *)
      printf '[install]\nminimumReleaseAge = 86400\n\n[install.security]\nscanner = "@socketsecurity/bun-security-scanner"\n' \
        > "$root/dashboard/bunfig.toml" ;;
  esac

  # A vendored tree WITH a lockfile and NO bunfig, and a directory with a bunfig and NO lockfile.
  # Neither resolves this repository's dependencies, so neither may be demanded of: the first is
  # another project's configuration, the second is not a tree at all (it would also catch a
  # dashboard/ tree that lost its lockfile as a missing-bunfig failure, which is the wrong name
  # for the fault).
  mkdir -p "$root/Packs/planted/pi-packages/vendored" "$root/not-a-tree"
  printf '{"name":"vendored"}\n' > "$root/Packs/planted/pi-packages/vendored/package.json"
  printf '{}\n' > "$root/Packs/planted/pi-packages/vendored/bun.lock"
  printf '{"name":"unresolved"}\n' > "$root/not-a-tree/package.json"
  printf '[install]\nminimumReleaseAge = 86400\n' > "$root/not-a-tree/bunfig.toml"
  printf '%s' "$root"
}

expect() { # expect <name> <status> <root>
  local out status
  out="$(SJEL_BUN_INSTALL_POLICY_ROOT="$3" "$CHECK" 2>&1)"; status=$?
  if [ "$status" -ne "$2" ]; then
    echo "FAIL: $1 expected exit $2, got $status:" >&2
    printf '%s\n' "$out" >&2
    fails=$((fails + 1))
  fi
}

expect "safe installs pass" 0 "$(plant safe 'bun install --frozen-lockfile --ignore-scripts')"
expect "CI install without hook protection fails" 1 "$(plant unsafe 'bun install --frozen-lockfile')"

# The hold and the scanner are two settings, and a tree can lose either one alone. Both planted
# cases keep everything else — workflows, READMEs, lockfile — so the exit status can only be
# about the setting that went missing.
expect "a resolving tree with no bunfig fails" 1 \
  "$(plant no-hold 'bun install --frozen-lockfile --ignore-scripts' no-bunfig)"
expect "a tree that keeps the hold but loses the scanner fails" 1 \
  "$(plant no-scanner 'bun install --frozen-lockfile --ignore-scripts' hold-only)"

if [ "$fails" -ne 0 ]; then
  echo "bun install policy test: $fails check(s) failed" >&2
  exit 1
fi
echo "bun install policy test: all checks passed"

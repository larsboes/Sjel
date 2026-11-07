#!/bin/bash
# Planted-tree regression tests for dependency install safeguards.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-bun-install-policy.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fails=0

plant() { # plant <name> <ci-install> [<mode>]
  local root="$SCRATCH/$1" mode="${3:-full}"
  mkdir -p "$root/.github/workflows" "$root/ui" "$root/crates/comms/clipper"
  printf '%s\n' "$2" > "$root/.github/workflows/ci.yml"
  printf 'bun pm scan\n' >> "$root/.github/workflows/ci.yml"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/crates/comms/clipper/README.md"

  printf '{"name":"planted-ui"}\n' > "$root/ui/package.json"
  printf '{}\n' > "$root/ui/bun.lock"
  case "$mode" in
    no-bunfig) : ;; # a tree that resolves dependencies and declares nothing
    hold-only)
      printf '[install]\nminimumReleaseAge = 86400\n' > "$root/ui/bunfig.toml" ;;
    *)
      printf '[install]\nminimumReleaseAge = 86400\n\n[install.security]\nscanner = "@socketsecurity/bun-security-scanner"\n' \
        > "$root/ui/bunfig.toml" ;;
  esac

  # A vendored tree WITH a lockfile and NO bunfig, and a directory with a bunfig and NO lockfile.
  # Neither resolves this repository's dependencies, so neither may be demanded of: the first is
  # another project's configuration, the second is not a tree at all (it would also catch a
  # ui/ tree that lost its lockfile as a missing-bunfig failure, which is the wrong name
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

expect "a resolving tree with no bunfig fails" 1 \
  "$(plant no-hold 'bun install --frozen-lockfile --ignore-scripts' no-bunfig)"
expect "a tree that keeps the hold but loses the scanner fails" 1 \
  "$(plant no-scanner 'bun install --frozen-lockfile --ignore-scripts' hold-only)"

runner_plant() {
  local root
  root="$(plant "$1" 'bun tools/ci-run.ts ui-check')"
  printf 'bun tools/ci-run.ts bun-tests\n' >> "$root/.github/workflows/ci.yml"
  mkdir -p "$root/tools"
  printf '%s\n' "$2" '["bun", "pm", "scan"]' > "$root/tools/ci-run.ts"
  printf '%s' "$root"
}

expect "shared runner preserves install protections" 0 \
  "$(runner_plant runner-safe '["bun", "install", "--frozen-lockfile", "--ignore-scripts"]')"
expect "shared runner without hook protection fails" 1 \
  "$(runner_plant runner-unsafe '["bun", "install", "--frozen-lockfile"]')"
runner="$(runner_plant runner-no-scan '["bun", "install", "--frozen-lockfile", "--ignore-scripts"]')"
printf '%s\n' '["bun", "install", "--frozen-lockfile", "--ignore-scripts"]' > "$runner/tools/ci-run.ts"
expect "shared runner without a dependency scan fails" 1 "$runner"

if [ "$fails" -ne 0 ]; then
  echo "bun install policy test: $fails check(s) failed" >&2
  exit 1
fi
echo "bun install policy test: all checks passed"

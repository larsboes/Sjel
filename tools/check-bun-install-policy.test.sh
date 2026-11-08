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

  printf '{"name":"planted-ui","devDependencies":{"@socketsecurity/bun-security-scanner":"^1.1.3"}}\n' > "$root/ui/package.json"
  printf '{}\n' > "$root/ui/bun.lock"
  case "$mode" in
    no-bunfig) : ;; # a tree that resolves dependencies and declares nothing
    hold-only)
      printf '[install]\nminimumReleaseAge = 86400\n' > "$root/ui/bunfig.toml" ;;
    *)
      printf '[install]\nminimumReleaseAge = 86400\n\n[install.security]\nscanner = "@socketsecurity/bun-security-scanner"\n' \
        > "$root/ui/bunfig.toml" ;;
  esac

  # Packages without a lockfile are not installed by the repository test runner.
  mkdir -p "$root/Packs/planted/pi-packages/vendored" "$root/not-a-tree"
  printf '{"name":"vendored"}\n' > "$root/Packs/planted/pi-packages/vendored/package.json"
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

missing="$(plant undeclared-scanner 'bun install --frozen-lockfile --ignore-scripts')"
printf '{"name":"planted-ui"}\n' > "$missing/ui/package.json"
expect "a configured scanner without a dependency fails" 1 "$missing"

vendored="$(plant vendored-lock 'bun install --frozen-lockfile --ignore-scripts')"
printf '{}\n' > "$vendored/Packs/planted/pi-packages/vendored/bun.lock"
expect "a locked vendored package also requires the install policy" 1 "$vendored"

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

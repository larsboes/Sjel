#!/bin/bash
# local-build.sh — the power profile's answer about local compilation, Clippy and tests.
#
# One decision, in one place: crates/runtime owns the profile and sjel-cli exposes it as
# `runtime admit local-build`. This file only finds that binary and translates its exit code.
# It never builds it — a guard that compiles to decide whether compiling is allowed would be
# the drain it exists to prevent.
#
#   local_build_check   0 admitted · 3 deferred · 2 undecidable (no built CLI, or an error)
#
# SJEL_ALLOW_LOCAL_BUILD=1 admits one command. It is read here and never saved: a standing
# exception belongs in the profile, which is why the runtime has no category for builds.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).

SJEL_LOCAL_BUILD_LIB="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"

# A release binary, if this checkout or the machine already has one. A stale binary that lacks
# the subcommand exits 1; that is undecidable, not a refusal, so the caller says so plainly.
local_build_cli() {
  local candidate root
  root="$(cd "$SJEL_LOCAL_BUILD_LIB/../.." && pwd)"
  for candidate in \
    "${SJEL_CLI_BIN:-}" \
    "${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/release/sjel-cli}" \
    "$root/target/release/sjel-cli" \
    "$HOME/.cargo-target/release/sjel-cli"; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}

local_build_check() {
  if [ "${SJEL_ALLOW_LOCAL_BUILD:-}" = 1 ]; then
    echo "local-build: SJEL_ALLOW_LOCAL_BUILD=1 — the power profile is bypassed for this command" >&2
    return 0
  fi
  local bin code
  if ! bin="$(local_build_cli)"; then
    echo "local-build: no built sjel-cli, so the power profile cannot be read — heavy work is NOT battery-protected" >&2
    return 2
  fi
  "$bin" runtime admit local-build
  code=$?
  case "$code" in
    0|3) return "$code" ;;
    # A binary older than this guard lacks the subcommand. Undecidable, and said out loud.
    *) echo "local-build: $bin cannot read the power profile (exit $code) — rebuild sjel-cli; heavy work is NOT battery-protected" >&2; return 2 ;;
  esac
}

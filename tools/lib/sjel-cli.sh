#!/bin/bash
# The launcher half of every tools/ script whose logic moved into the sjel-cli crate
# (tools/sjel-cli). Set SJEL_ROOT (tools/lib/paths.sh does) and source tools/lib/toml.sh,
# then call
#
#   sjel_cli_exec <subcommand> "$@"    # a ported tools/ script
#   sjel_cli_exec "$@"                 # the sjel launcher: its arguments are the binary's
#
# It builds the release binary when it is missing or older than the crate's sources, then execs
# it. The same on-demand build tools/storage/storage and tools/service-runner.sh do, written once
# here because sjel-cli is one binary behind many launchers.
#
# SJEL_CLI_BIN names a prebuilt binary and skips the build. A test that copies a launcher into a
# scratch root needs it: that root has no crate to build from.
#
# cargo is required on every Sjel host (toolchain.toml [cargo], decided 2026-10-02 when the
# tooling started moving to Rust). Without it this prints the install line and exits 1, so a
# fresh tools/install.sh run still reports what to install first.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).

sjel_cli_exec() {
  local sub
  sub="$(basename "$0")"
  local bin="${SJEL_CLI_BIN:-}"
  if [ -z "$bin" ]; then
    local crate
    crate="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../sjel-cli" 2>/dev/null && pwd)" || crate=""
    if [ -z "$crate" ] || [ ! -f "$crate/Cargo.toml" ]; then
      echo "$sub: no tools/sjel-cli crate beside this launcher and SJEL_CLI_BIN is unset" >&2
      exit 2
    fi
    bin="${CARGO_TARGET_DIR:-$SJEL_ROOT/target}/release/sjel-cli"
    if [ ! -x "$bin" ] || [ -n "$(find "$crate/src" "$crate/Cargo.toml" -newer "$bin" -print -quit 2>/dev/null)" ]; then
      # A stale binary that still runs beats no answer. Daemons call these launchers too: the
      # launchd watchdogs run tools/service-runner.sh, sjel-status runs tools/capability.sh.
      # Their PATH may hold no cargo, and a crate mid-edit may not compile. Either way the last
      # good binary answers and the rebuild is reported, rather than a supervisor or a
      # registry read failing because somebody saved a source file.
      if ! command -v cargo >/dev/null 2>&1; then
        if [ -x "$bin" ]; then
          echo "$sub: sjel-cli is older than its sources and cargo is not on PATH — running the existing binary" >&2
        else
          local hint=""
          case "$(uname -s)" in
            Darwin) hint="$(toml_get_in cargo install_macos "$SJEL_ROOT/toolchain.toml")" ;;
            *)      hint="$(toml_get_in cargo install_linux "$SJEL_ROOT/toolchain.toml")" ;;
          esac
          echo "$sub: cargo is not installed, and Sjel's tooling is built with it." >&2
          echo "     install: $hint" >&2
          exit 1
        fi
      else
        echo "$sub: building sjel-cli (release)..." >&2
        if ! cargo build --locked --release -p sjel-cli --manifest-path "$crate/../../Cargo.toml" >&2; then
          [ -x "$bin" ] || exit 2
          echo "$sub: the sjel-cli build failed — running the existing, older binary" >&2
        fi
      fi
    fi
  fi
  exec "$bin" "$@"
}

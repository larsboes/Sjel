#!/bin/bash
# tools/capability.sh — manage which capabilities are enabled on this machine.
#
# Launcher only. The logic moved to tools/sjel-cli/src/registry.rs on 2026-10-02; this path
# stays because service-runner.sh, sjel-status, dashboard/vite.config.ts, doctor and self all
# call it. `tools/capability.sh -h` prints the usage.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).
set -u

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"       # SJEL_ROOT, SJEL_MACHINE_TOML, SJEL_OVERLAY_ROOT, SJEL_*CAPS_DIR
# shellcheck source=tools/lib/sjel-cli.sh
. "$_here/lib/sjel-cli.sh"
sjel_cli_exec capability.sh "$@"

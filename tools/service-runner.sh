#!/bin/bash
# tools/service-runner.sh — the shared service-manifest interpreter (schemas/service.toml.example).
#
# Launcher only. The logic moved to tools/sjel-cli/src/runner.rs and persist.rs on 2026-10-02.
# This path stays because the launchd and systemd units it installs, tools/watchdog.sh,
# backup.sh, container-refresh.sh, doctor and sjel-status all run it by this path.
# `tools/service-runner.sh` with no arguments prints the usage.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).
set -u

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"       # SJEL_ROOT, SJEL_MACHINE_TOML, SJEL_OVERLAY_ROOT, SJEL_*CAPS_DIR
# shellcheck source=tools/lib/sjel-cli.sh
. "$_here/lib/sjel-cli.sh"
sjel_cli_exec service-runner.sh "$@"

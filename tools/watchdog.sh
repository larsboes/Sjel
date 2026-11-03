#!/bin/bash
# Generic watchdog: `watchdog.sh <capability>` keeps calling
# `service-runner.sh start <capability>`. It exists for kind = "process"
# capabilities. A container capability never gets one: docker and podman restart
# it themselves (`--restart unless-stopped`) and service-runner.sh's
# persistence_applicable says so rather than installing a unit that would fight
# the runtime. apple-container had no restart policy and was the one runtime that
# made this file apply to a container; it retired 2026-09-02 (Q75).
set -euo pipefail
CAP="${1:?usage: watchdog.sh <capability>}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Tells service-runner.sh this start comes from the job itself, so it starts the process here
# rather than handing the start back to launchd.
export SJEL_WATCHDOG=1
while true; do
  # stdout is noise (the runtime echoes the container name whenever it actually starts
  # one); stderr is the only thing anyone ever needs, so it lands in the plist's
  # StandardErrorPath instead of /dev/null. `|| true` stays -- the loop has to outlive
  # any single failure -- but the reason no longer disappears along with it. A healthy
  # loop still writes nothing at all: service-runner.sh no-ops on an already-running
  # capability rather than erroring, which is what makes this affordable.
  if ! err="$("$SCRIPT_DIR/service-runner.sh" start "$CAP" 2>&1 >/dev/null)"; then
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') watchdog[$CAP]: ${err:-start failed, no output}" >&2
  fi
  sleep 30
done

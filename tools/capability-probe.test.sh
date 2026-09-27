#!/bin/bash
# Tests for tools/lib/capability-probe.sh — where the CLI polls a capability, and what the
# answer means.
#
# Every case here is a wrong answer `axon capability health` gave on a real machine on
# 2026-09-08, held as a fixture so it cannot come back. None of them could be caught by
# running the CLI: they need capabilities that are running, capabilities that are refusing,
# and a capability on another host, and CI has none of the three.
#
# The rules are copied from capabilities/sjel-status/src/status/registry.rs, which owns this
# question for the dashboard. Where the two disagreed, the CLI was wrong.
set -uo pipefail

# shellcheck source=lib/capability-probe.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib/capability-probe.sh"

fails=0

is() { # is <description> <expected> <actual>
  if [ "$2" != "$3" ]; then
    echo "FAIL: $1"
    echo "    expected: '$2'"
    echo "    got:      '$3'"
    fails=$((fails + 1))
  fi
}

# --- which path is polled --------------------------------------------------

is "readiness wins where a capability declares it" \
  "http://127.0.0.1:8083/ready" \
  "$(capability_probe_url capability 8083 "" /health /ready)"

# The compatibility half of Axon#126: a capability with no ready_path behaves exactly as it
# did. Without this the fix would have silently stopped probing half the registry.
is "health is the fallback, not a second choice to skip" \
  "http://127.0.0.1:3000/health" \
  "$(capability_probe_url capability 3000 "" /health "")"

is "no path at all is nothing to poll, not a URL to guess" \
  "" \
  "$(capability_probe_url capability 8080 "" "" "")"

is "a capability with no port has no loopback surface" \
  "" \
  "$(capability_probe_url capability "" "" /health "")"

# --- the external case, which was invisible --------------------------------
#
# vaultwarden declares health_path = "/alive" and resolves an endpoint on another host. Its
# port is blank because a port is a fact about the host that binds it. The CLI selected rows
# on `.port != ""`, so this capability was absent from list and health entirely.

is "an external capability is polled at its endpoint" \
  "https://homepi.example.ts.net/alive" \
  "$(capability_probe_url external "" https://homepi.example.ts.net /alive "")"

is "an external capability with no resolved endpoint has nothing to poll" \
  "" \
  "$(capability_probe_url external "" "" /alive "")"

# A port on an external row would be another host's fact. Even if one leaked through, the
# endpoint is what must be dialled — never loopback, which is a different machine's service.
is "an external capability is never dialled on loopback" \
  "https://homepi.example.ts.net/alive" \
  "$(capability_probe_url external 8080 https://homepi.example.ts.net /alive "")"

# --- base URLs -------------------------------------------------------------

is "a local base URL is loopback and the published port" \
  "http://127.0.0.1:8086" "$(capability_base_url capability 8086 "")"
is "an external base URL is the resolved endpoint" \
  "https://homepi.example.ts.net" "$(capability_base_url external "" https://homepi.example.ts.net)"
is "a capability with neither has no base URL" \
  "" "$(capability_base_url capability "" "")"

# --- what a silent port means ----------------------------------------------
#
# The whole reason for three states. `dashboard` declares port 47117 and
# autostart = "false"; that port is the hot-reload dev server, and the shell it serves has
# been served by sjel-status since 2026-08-29. Calling it "down" was wrong twice, and it set
# a non-zero exit for a machine that was entirely healthy.

is "a 200 is up whatever the manifest declares" up "$(capability_state 200 false)"
is "a capability that declares autostart and is silent is down" down "$(capability_state 000 true)"
is "a capability that declares autostart=false and is silent is off" off "$(capability_state 000 false)"
is "a 500 from something that should be running is down" down "$(capability_state 500 true)"
is "a 404 is not up" off "$(capability_state 404 false)"

# A local capability with no `autostart` line is off, which is what the rest of the
# repository already means by an absent field: tools/capability.sh's `_is_autostart` and
# `_emit_line` both read absent as "false", so the supervisor does not keep it running
# either. It is a real reduction in what this verb can catch — five capabilities answering
# 200 today declare no autostart — and it is written down in the library rather than
# discovered later.
is "an absent autostart is not a claim that it should be running" off "$(capability_state 000 "")"

# The external row, which the rule above must NOT swallow. tools/capability.sh blanks
# `autostart` for `scope = external` because a manifest describes how its OWNER runs the
# capability and this machine has no authority over another host — so vaultwarden arrives
# here as "" while its own service.toml declares `autostart = "true"`. Read as a
# declaration, an unreachable peer came back `off ... not autostarted; start it with
# tools/service-runner.sh start vaultwarden` and exit 0: the wrong state, the wrong exit,
# and an instruction to start another host's service with this machine's supervisor.
is "an external capability that does not answer is down, whatever the blanked field says" \
  down "$(capability_state 000 "" external)"
is "an external capability that answers is up" up "$(capability_state 200 "" external)"
# And the blanking is not something this test invented: it is the registry's own rule.
is "an external row is down even if an autostart value did leak through" \
  down "$(capability_state 000 false external)"

# --- the row separator -----------------------------------------------------
#
# @tsv plus `read` was the bug that printed `http://127.0.0.1:8082true`: tab is IFS
# whitespace, so a run of tabs collapses and every field after an empty one shifts left. This
# is that row — sjel-status, which declares no ready_path — read back with the separator the
# library chose.

row="sjel-status${CAPABILITY_FS}capability${CAPABILITY_FS}8082${CAPABILITY_FS}${CAPABILITY_FS}/health${CAPABILITY_FS}${CAPABILITY_FS}true"
IFS="$CAPABILITY_FS" read -r f_name f_scope f_port f_endpoint f_health f_ready f_auto <<EOF
$row
EOF
is "an empty endpoint does not shift the row" "" "$f_endpoint"
is "the health path survives an empty field before it" "/health" "$f_health"
is "an empty ready path stays empty" "" "$f_ready"
is "autostart is read as autostart, not as a path" "true" "$f_auto"
is "and the whole row still resolves to the right probe" \
  "http://127.0.0.1:8082/health" \
  "$(capability_probe_url "$f_scope" "$f_port" "$f_endpoint" "$f_health" "$f_ready")"

# The known-bad half: the same row through a tab, which is what was there. If this ever
# stops being wrong, the separator no longer matters and this file can lose the section.
tab_row="$(printf 'sjel-status\tcapability\t8082\t\t/health\t\ttrue')"
IFS="$(printf '\t')" read -r t_name t_scope t_port t_endpoint t_health t_ready t_auto <<EOF
$tab_row
EOF
if [ "$t_health" = "/health" ]; then
  echo "FAIL: the tab separator no longer collapses empty fields — this suite's premise is stale"
  fails=$((fails + 1))
fi

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "capability-probe.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "capability-probe.test.sh: all cases passed"

# tools/lib/capability-probe.sh — how the CLI decides where to poll a capability and what
# the answer means. Sourced by `axon`; tested by tools/capability-probe.test.sh.
#
# It is a library rather than four functions inside `axon` for one reason: the rules here
# were wrong for two years in ways nobody could see, and none of them can be tested through
# the CLI on a machine that has no services running. CI has none.
#
# Where to poll is copied from capabilities/sjel-status/src/status/registry.rs, which
# already owns that question for the dashboard, and where the two disagreed the CLI was the
# one that was wrong. What the answer MEANS is not copied and cannot be: that surface has
# `up: Option<bool>` and no `off` at all, so the third state below is this CLI's own and is
# argued for where it is defined rather than cited to a file that does not hold it.
#
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).

# Unit Separator, not a tab.
#
# Bash treats tab as IFS *whitespace*, so a run of tabs collapses into one and every empty
# field shifts the rest of the row left. With `jq @tsv` and a capability that declares no
# ready path, `read -r name scope port endpoint health_path ready_path autostart` put
# `autostart` where `health_path` belonged and the CLI printed
# `http://127.0.0.1:8082true`. US is not IFS whitespace, so an empty field stays one field.
# No manifest value can contain it.
CAPABILITY_FS=$'\037'

# Where to poll one capability, and on what.
#
# Mirrors `probe_url` and `readiness_url` in
# capabilities/sjel-status/src/status/registry.rs. Two properties came from there, and the
# CLI had neither:
#
#   Readiness first. Until 2026-08-07 sjel-status polled `health_path` everywhere, and five
#   database-backed capabilities answered it from a stateless handler that could not see
#   their database — reporting themselves up through an outage that failed every query
#   behind them (Axon#126). A capability that declares no `ready_path` is unchanged.
#
#   An external capability has no port here. The registry blanks it, because a port number
#   is a fact about the host that binds it, and hands over `endpoint` instead. The CLI
#   filtered its rows on `.port != ""`, so vaultwarden — which declares
#   `health_path = "/alive"` and resolves an endpoint — was not merely unprobed, it was
#   absent from `axon capability list` and `axon capability health` entirely.
#
# Prints nothing when there is nothing to poll. The caller reports that as unknown, which is
# what sjel-status does too: "a capability without such a surface is reported as unknown
# rather than silently down".
capability_probe_url() {  # <scope> <port> <endpoint> <health_path> <ready_path>
  local scope="$1" port="$2" endpoint="$3" health_path="$4" path="${5:-}"
  [ -n "$path" ] || path="$health_path"
  [ -n "$path" ] || return 0
  if [ "$scope" = external ]; then
    [ -n "$endpoint" ] || return 0
    printf '%s%s' "$endpoint" "$path"
  else
    [ -n "$port" ] || return 0
    printf 'http://127.0.0.1:%s%s' "$port" "$path"
  fi
}

# The base URL a caller should dial: loopback for a capability this machine runs, the
# resolved endpoint for one it only consumes. Empty when neither is known.
capability_base_url() {  # <scope> <port> <endpoint>
  if [ "$1" = external ]; then
    printf '%s' "$3"
  elif [ -n "$2" ]; then
    printf 'http://127.0.0.1:%s' "$2"
  fi
}

# up / off / down — three states, because "down" was carrying two meanings.
#
# `autostart` is what separates the last two, and sjel-status already reasons this way:
# `idle_timeout_secs` refuses to reap anything with `autostart = "true"` because "a
# capability the machine is supposed to keep running is never idle by definition". Turn that
# around and a capability the machine is NOT supposed to keep running is not faulty when it
# is not running. It is off.
#
# The case that forced it: `dashboard` declares port 47117 and `autostart = "false"`. That
# port is the hot-reload dev server, started by hand for an editing session; the shell it
# serves has been served by sjel-status on its own port since 2026-08-29. So
# `axon capability health` printed `down dashboard http://127.0.0.1:47117/` while the
# dashboard answered 200, and exited non-zero for it. That line sent at least two sessions to
# a dead URL.
#
# An EXTERNAL capability is never off. Its `autostart` is empty here whatever its manifest
# says, because `tools/capability.sh` blanks every field that would be a claim of authority
# over another host — "a manifest says how its OWNER runs the capability". Reading that blank
# as a declaration inverted the answer: vaultwarden's own service.toml declares
# `autostart = "true"`, and against a planted registry row that does not answer, this printed
# `off vaultwarden ... not autostarted; start it with tools/service-runner.sh start
# vaultwarden` and exited 0 (measured 2026-09-08). Wrong three times over — the state, the
# exit status, and an instruction to start another host's service with this machine's
# supervisor. A withheld authority is not a declaration that the capability is optional, and
# sjel-status agrees: `CapabilityView.up` is an Option<bool> with no `off` to land in.
#
# A LOCAL capability with no `autostart` line stays off, and that is deliberate rather than
# an oversight: `tools/capability.sh`'s own `_is_autostart` and `_emit_line` both read an
# absent field as "false", so the supervisor does not keep such a capability running either.
# The cost is real and is written down rather than hidden — scouting, transit, trips, places
# and vault declare no autostart, all five answer 200 today, and none of them can make this
# verb non-zero. Closing that means declaring `autostart` in five manifests; it is not a
# thing to guess at here.
capability_state() {  # <http-code> <autostart> [scope] -> up|off|down
  if [ "$1" = "200" ]; then
    echo up
  elif [ "${3:-}" = external ]; then
    echo down
  elif [ "$2" = "true" ]; then
    echo down
  else
    echo off
  fi
}

# `--connect-timeout` and `--max-time`, because one of these probes now leaves the machine:
# an external capability is reached over the tailnet, and a peer that is asleep would
# otherwise hang the whole command on curl's default.
capability_probe_code() {  # <url> -> the HTTP status, or 000
  curl -s -o /dev/null -w '%{http_code}' --connect-timeout 2 --max-time 5 "$1" 2>/dev/null || true
}

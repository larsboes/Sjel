#!/bin/bash
# tools/capability.sh — manage which capabilities are enabled on this machine.
# The `capabilities = [...]` line in <overlay>/config/machine.toml is the single
# source of truth, and this is the one tool that writes it (enable/disable). It
# lives in the overlay rather than in axon.toml because the enabled set is a fact
# about one machine, and axon.toml is tracked and shared — see
# schemas/machine.toml.example. Every read goes
# through tools/lib/toml.sh; requires= dependencies are resolved transitively and
# cycle-safely on enable, and guarded on disable so a still-needed capability
# can't be stranded.
#
# No service is ever started here — after enable, the suggested
# `tools/service-runner.sh start <name>` is printed for you to run, never run
# automatically.
#
#   tools/capability.sh list             # every capability + enabled/disabled + requires
#   tools/capability.sh enable <name>    # enable <name> and everything it requires
#   tools/capability.sh disable <name>   # disable <name> (blocked if a dependent needs it)
#   tools/capability.sh registry         # the enabled set as JSON, in dependency order
#   tools/capability.sh -h               # this help
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SJEL_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"

case "${1:-}" in
  -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
esac

# paths.sh (not toml.sh directly) — it resolves this machine's overlay and exports
# SJEL_MACHINE_TOML, the file the enabled set lives in.
source "$TOOLS_DIR/lib/paths.sh"
# The enabled set answers "what does this machine RUN". A capability it CONSUMES from another
# overlay's deployment is a different claim in a different field; external-ref.sh reads it, and
# the registry is how it reaches every consumer that is not shell.
source "$TOOLS_DIR/lib/external-ref.sh"

CAPS_DIR="$SJEL_ROOT/capabilities"

if [ ! -f "$SJEL_MACHINE_TOML" ]; then
  echo "capability.sh: no machine.toml at $SJEL_MACHINE_TOML — run tools/install.sh first." >&2
  exit 1
fi

# --- shared reads (all via toml.sh) --------------------------------------

_enabled_names() {  # space-separated, in machine.toml order
  toml_array capabilities "$SJEL_MACHINE_TOML" | tr '\n' ' '
}

_cap_requires() {  # <name> -> space-separated direct requires (empty if none/no manifest)
  local mf
  mf="$(axon_manifest_for "$1")" || return 0
  [ -n "$mf" ] || return 0
  toml_array requires "$mf" | tr '\n' ' '
}

_has_service() {  # <name> -> exit 0 if either root declares it
  axon_manifest_for "$1" >/dev/null 2>&1
}

_is_autostart() {  # <name> -> exit 0 when the manifest declares autostart = "true"
  local mf
  mf="$(axon_manifest_for "$1" 2>/dev/null)" || return 1
  [ -n "$mf" ] || return 1
  [ "$(toml_get autostart "$mf")" = "true" ]
}

_is_data() {  # <name> -> exit 0 when the manifest declares state rather than a process
  local mf
  mf="$(axon_manifest_for "$1" 2>/dev/null)" || return 1
  [ -n "$mf" ] || return 1
  [ "$(toml_get kind "$mf")" = "data" ]
}

_has_schedule() {  # <name> -> exit 0 when the manifest declares a periodic schedule
  local mf
  mf="$(axon_manifest_for "$1" 2>/dev/null)" || return 1
  [ -n "$mf" ] || return 1
  [ -n "$(toml_get schedule "$mf")" ]
}

# A capability directory, from whichever root holds it. Separate from _manifest_for
# because a capability is allowed to exist before it declares a service: `enable` and
# dependency resolution ask "is this a real capability", not "does it run".
_cap_dir_for() {  # <name> -> its directory, empty if neither root has it
  if [ -d "$CAPS_DIR/$1" ]; then
    echo "$CAPS_DIR/$1"
  elif [ -d "$SJEL_OVERLAY_CAPS_DIR/$1" ]; then
    echo "$SJEL_OVERLAY_CAPS_DIR/$1"
  fi
}

# Both capability roots, deduplicated. An overlay-owned capability is enabled, started
# and reported exactly like a public one; only the tracked generators ignore it.
_cap_dirs() {  # names of every <root>/capabilities/<name>/ directory, one per line
  local d root
  for root in "$CAPS_DIR" "$SJEL_OVERLAY_CAPS_DIR"; do
    [ -d "$root" ] || continue
    for d in "$root"/*/; do
      [ -d "$d" ] || continue
      basename "$d"
    done
  done | sort -u
}

# A spine component may carry a service.toml at the repo root (today: dashboard/,
# README.md#three-architectural-nouns). Discovered by glob rather than listed, so the list can never go
# stale: a top-level service.toml IS the declaration. Spine services are always in the
# registry and never in machine.toml's `capabilities` — the spine exists on every
# machine by definition, which is exactly README.md#three-architectural-nouns's membership test.
_spine_names() {  # names of every <root>/<name>/service.toml, one per line
  local f
  for f in "$SJEL_ROOT"/*/service.toml; do
    [ -f "$f" ] || continue
    basename "$(dirname "$f")"
  done
}

# Empty output means "no manifest", which is a normal answer here. A duplicate
# declaration is not: axon_manifest_for has already named both paths on stderr, and
# continuing would mean picking one, so this exits instead. `|| rc=$?` keeps `set -e`
# from taking the decision away first.
_manifest_for() {  # <name> -> path of its service.toml, empty if it has none
  local mf="" rc=0
  mf="$(axon_manifest_for "$1")" || rc=$?
  if [ "$rc" -eq 2 ]; then exit 2; fi
  if [ -n "$mf" ]; then echo "$mf"; fi
  return 0
}

# --- the single machine.toml write point ---------------------------------

_write_capabilities() {  # <space-separated ordered names> -> rewrite the one line
  local count
  count="$(grep -cE '^capabilities[[:space:]]*=' "$SJEL_MACHINE_TOML" || true)"
  # More than one line is a corrupted file: a best-effort write would rewrite both
  # and quietly lose whichever the reader was not using. Refuse.
  if [ "$count" -gt 1 ]; then
    echo "capability.sh: found $count 'capabilities = [...]' lines in $SJEL_MACHINE_TOML — fix that file by hand first." >&2
    exit 1
  fi
  local names="$1" joined="" n
  for n in $names; do
    if [ -z "$joined" ]; then joined="\"$n\""; else joined="$joined, \"$n\""; fi
  done
  local newline="capabilities = [$joined]"
  if [ "$count" -eq 0 ]; then
    # A machine.toml written before this field existed, or by an installer run that
    # had nothing to enable yet. Seed it with its explanatory comment rather than
    # demanding a hand-edit first.
    {
      echo
      echo "# Capabilities enabled on THIS machine — written by tools/capability.sh"
      echo "# (enable/disable resolve service.toml \`requires =\` transitively); hand-editing"
      echo "# is legal, and tools/doctor re-checks that the set stays dependency-closed."
      echo "# Single-line array per tools/lib/toml.sh's contract."
      echo "$newline"
    } >> "$SJEL_MACHINE_TOML"
    return 0
  fi
  # `-i.bak` + rm is the portable form that behaves identically under BSD sed
  # (macOS) and GNU sed (Linux); same idiom tools/lib/toml.sh:toml_set uses.
  sed -i.bak -E "s|^capabilities[[:space:]]*=.*|$newline|" "$SJEL_MACHINE_TOML"
  rm -f "$SJEL_MACHINE_TOML.bak"
}

# --- transitive requires resolution (bash-3.2, cycle-safe) ---------------

# No associative arrays here on purpose (bash 3.2 / macOS stock bash has none):
# RESOLVED is a space-separated, dependency-first list; VISITING is the current
# DFS stack, which doubles as the cycle guard.
RESOLVED=""
VISITING=""

_resolve() {  # <name> -> append <name> + its transitive requires to RESOLVED, deps first
  local name="$1"
  case " $RESOLVED " in *" $name "*) return 0 ;; esac  # already settled
  case " $VISITING " in *" $name "*) return 0 ;; esac  # on the stack -> cycle, stop
  if [ -z "$(_cap_dir_for "$name")" ]; then
    echo "capability.sh: required capability '$name' has no capabilities/$name/ directory in Axon or the overlay" >&2
    exit 1
  fi
  VISITING="$VISITING $name"
  local dep
  for dep in $(_cap_requires "$name"); do
    _resolve "$dep"
  done
  case " $RESOLVED " in *" $name "*) ;; *) RESOLVED="$RESOLVED $name" ;; esac
}

# --- subcommands ----------------------------------------------------------

cmd_list() {
  local enabled; enabled=" $(_enabled_names) "  # space-padded for membership tests
  local name status reqs
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    status="disabled"
    case "$enabled" in *" $name "*) status="enabled" ;; esac
    reqs="$(_cap_requires "$name")"
    reqs="${reqs% }"  # trim the single trailing space tr left behind
    if [ -n "$reqs" ]; then
      printf '  %-18s [%s]  requires: %s\n' "$name" "$status" "$reqs"
    else
      printf '  %-18s [%s]\n' "$name" "$status"
    fi
  done <<EOF
$(_cap_dirs)
EOF
}

cmd_enable() {  # <name>
  local name="$1"
  if [ -z "$(_cap_dir_for "$name")" ]; then
    echo "capability.sh: no such capability '$name'" >&2
    echo "valid options:" >&2
    _cap_dirs | sed 's/^/  /' >&2
    exit 1
  fi

  RESOLVED=""; VISITING=""
  _resolve "$name"
  echo "Resolution chain (dependencies first): ${RESOLVED# }"

  local enabled; enabled=" $(_enabled_names) "
  local newly="" n
  for n in $RESOLVED; do
    case "$enabled" in
      *" $n "*) echo "  = $n (already enabled)" ;;
      *)        newly="$newly $n"; echo "  + $n (enabling)" ;;
    esac
  done

  if [ -z "$newly" ]; then
    echo "Nothing to do — already enabled."
    exit 0
  fi

  # Append the newly-enabled names to the existing order; the existing order is
  # preserved untouched so an enable/disable round-trip is byte-identical.
  _write_capabilities "$(_enabled_names)$newly"
  echo "machine.toml: capabilities updated."

  # v0 never starts services — just surface the next step for each newly-enabled
  # capability that actually has a container manifest.
  local suggested=""
  for n in $newly; do
    _has_service "$n" && suggested="$suggested $n"
  done
  if [ -n "$suggested" ]; then
    echo
    echo "Next step (not run automatically — start each when ready):"
    for n in $suggested; do
      # A scheduled capability has no useful `start` to suggest: starting it by hand runs one
      # tick, which is not what enabling it was for. The unit IS the way it runs.
      if _has_schedule "$n"; then
        echo "  tools/service-runner.sh install-persistence $n   # declares a schedule — the timer IS how it runs"
        continue
      fi
      # kind=data has nothing to start at all: it declares a file and how it is backed up
      # (capabilities/store). Suggesting `start` here would print a command the runner
      # refuses by name, which teaches a reader that the suggestion is noise.
      if _is_data "$n"; then
        echo "  tools/backup.sh $n   # declares data, not a process — nothing to start"
        continue
      fi
      echo "  tools/service-runner.sh start $n"
      # A capability that declares autostart is claiming it should always be running, and
      # `start` alone does not survive a reboot. Nothing used to say so, which is how a
      # capability could run all day and be gone the next morning (#9). Named here rather than
      # done here: installing persistence loads a launchd/systemd unit, and this tool starts
      # nothing. tools/doctor reports whichever of these were never run.
      if _is_autostart "$n"; then
        echo "  tools/service-runner.sh install-persistence $n   # declares autostart — survives a reboot only with this"
      fi
    done
  fi
}

# --- registry (the one place manifest facts leave the shell) --------------
#
# Emits the enabled set as JSON so non-shell consumers never learn to parse TOML:
# tools/lib/toml.sh stays the only parser (README.md#one-manifest-per-concern), and everyone else reads
# this. Three consumers today — service-runner.sh's up/down fan-out, sjel-status
# (which dials each health URL and starts capabilities on demand), and
# dashboard/vite.config.ts (which builds its proxy table from it). That third reader
# is the trigger sjel-status's own source comment named: the port literals it used to
# duplicate now live in one manifest each.
#
# Order is dependency-first, straight out of the same _resolve the enable path uses —
# so `up` starts a capability after whatever it requires (sparpreis-watch after transit), and
# `down` walks the list backwards.

_json_str() {  # <value> -> a JSON string. Refuses anything that would need escaping.
  case "$1" in
    *[\"\\]*)
      echo "capability.sh: manifest value needs JSON escaping, which this emitter deliberately does not do: $1" >&2
      exit 1
      ;;
  esac
  printf '"%s"' "$1"
}

_json_array_from() {  # stdin: one element per line -> ["a", "b"]
  local out="" v
  while IFS= read -r v; do
    [ -n "$v" ] || continue
    if [ -z "$out" ]; then out="$(_json_str "$v")"; else out="$out, $(_json_str "$v")"; fi
  done
  printf '[%s]' "$out"
}

REGISTRY_FIRST=1

_emit_service() {  # <name> <manifest> <scope> [endpoint]
  local name="$1" mf="$2" scope="$3" endpoint="${4:-}" kind
  kind="$(toml_get kind "$mf")"
  # "container" is the manifest default, but only where there is a manifest. An externally
  # provided capability may have none here at all — its manifest lives in whichever repo owns
  # its host — and calling that a container would be inventing a fact about someone else's
  # deployment.
  if [ -z "$kind" ]; then
    if [ "$mf" = /dev/null ]; then kind="external"; else kind="container"; fi
  fi
  [ "$REGISTRY_FIRST" -eq 1 ] || printf ',\n  '
  REGISTRY_FIRST=0
  printf '{"name": %s, "kind": %s, "scope": %s' \
    "$(_json_str "$name")" "$(_json_str "$kind")" "$(_json_str "$scope")"
  local key value
  # backup_* ride the same loop rather than a block of their own: they are manifest facts
  # leaving the shell, which is the one thing this function is for. `backup_target` is the
  # presence signal (backup.sh refuses a run without one), `backup_sqlite` is what decides
  # whether a run holds the capability down, and the two day fields are what timely MEANS
  # for this data. A consumer that shows a backup surface needs all four and has no other
  # legal way to get them — sjel-status is forbidden from parsing TOML (README.md#one-manifest-per-concern).
  #
  # `backup_sqlite_online` is the fifth and the counter-example that keeps the fourth
  # honest: capabilities/store's file is copied while it is open, so a run holds nothing
  # down. A consumer reading only `backup_sqlite` would render "this stops the service"
  # over a contract that does not.
  for key in port health_path ready_path panel_port panel_path autostart schedule proxy_api_only idle_timeout \
             backup_target backup_sqlite backup_sqlite_online backup_advise_days backup_stale_days \
             freshness_advise_hours freshness_stale_hours; do
    value="$(toml_get "$key" "$mf")"
    # A manifest says how its OWNER runs the capability. Project that onto a machine which only
    # consumes it and every field becomes a claim of authority it does not have: `autostart`
    # would install a watchdog for a process on another host, `backup_target` would put a backup
    # button on someone else's database, `port` would send a proxy to a local port with nothing
    # behind it. A client inherits `health_path` and `ready_path` and nothing else — the two
    # fields that describe how to ASK rather than how to ACT. Blanked here rather than filtered
    # per consumer, because a consumer that forgets is a consumer that reaches across the tailnet
    # and stops a vault.
    case "$scope:$key" in
      external:health_path|external:ready_path) ;;
      external:*) value="" ;;
    esac
    printf ', "%s": %s' "$key" "$(_json_str "$value")"
  done
  # The base URL a consumer should dial INSTEAD of loopback, and empty for every capability
  # this machine runs itself — which is most of them, and which is why this is a field rather
  # than a rule. A consumer seeing an empty endpoint builds 127.0.0.1:<port> exactly as before;
  # a consumer seeing a non-empty one must not, because the port is a fact about another host.
  printf ', "endpoint": %s' "$(_json_str "$endpoint")"
  if [ "$scope" = external ]; then
    # Same rule as the loop above. `requires` orders startup on the machine that starts things,
    # and this one starts nothing; `proxy_extra` routes a dev-server proxy at a local port.
    printf ', "proxy_extra": [], "requires": []}'
  else
    printf ', "proxy_extra": %s' "$(toml_array proxy_extra "$mf" | _json_array_from)"
    printf ', "requires": %s}' "$(toml_array requires "$mf" | _json_array_from)"
  fi
}

# The shell's own view of the same registry: whitespace-separated fields, because
# bash reading its own JSON with sed would be a regex parser nobody asked for. Same
# loop, same order, same source — only the rendering differs.
_emit_line() {  # <name> <manifest> <scope>
  local name="$1" mf="$2" scope="$3" kind autostart
  kind="$(toml_get kind "$mf")";           [ -n "$kind" ] || kind="container"
  autostart="$(toml_get autostart "$mf")"; [ -n "$autostart" ] || autostart="false"
  printf '%s %s %s %s\n' "$name" "$kind" "$scope" "$autostart"
}

_emit() {  # <json|lines> <name> <manifest> <scope>
  local fmt="$1"; shift
  if [ "$fmt" = lines ]; then _emit_line "$@"; else _emit_service "$@"; fi
}

cmd_registry() {  # [--lines]
  local fmt="json"
  [ "${1:-}" = "--lines" ] && fmt="lines"

  RESOLVED=""; VISITING=""
  local n
  for n in $(_enabled_names); do _resolve "$n"; done

  REGISTRY_FIRST=1
  if [ "$fmt" = json ]; then printf '[\n  '; fi
  # scope names the root a capability came from, so consumers do not have to re-derive
  # it from a path. tools/self.ts uses it to keep overlay capabilities out of self.json:
  # that file is tracked and public, and a capability name is a fact about a private
  # deployment. An implicit "is it in the repo" test would work today and rot silently.
  local _mf _scope
  for n in $RESOLVED; do
    _has_service "$n" || continue
    _mf="$(_manifest_for "$n")"
    case "$_mf" in
      "$SJEL_OVERLAY_CAPS_DIR"/*) _scope="overlay-capability" ;;
      *)                          _scope="capability" ;;
    esac
    _emit "$fmt" "$n" "$_mf" "$_scope"
  done

  # Capabilities this machine consumes but does not run (retired-tracker#169). JSON only, on
  # purpose: `--lines` drives the up/down/status fan-out, and a row there would mean the runner
  # walking over a service on another host — the exact authority a read-only reference exists
  # NOT to grant. Consumers that aggregate rather than act read the JSON, and get `scope`
  # "external" plus a resolved endpoint.
  #
  # An unresolvable reference is fatal here rather than skipped. This function is the one place
  # manifest facts leave the shell, so a silent omission would reach every consumer as "that
  # capability is not configured on this machine", which is precisely the wrong answer.
  local _endpoint _rc
  if [ "$fmt" = json ]; then
    while IFS= read -r n; do
      [ -n "$n" ] || continue
      _rc=0
      _endpoint="$(capability_endpoint "$n")" || _rc=$?
      if [ "$_rc" -ne 0 ]; then
        echo "capability.sh: cannot resolve the external provider for '$n' — see above." >&2
        exit 1
      fi
      _mf="$(_manifest_for "$n")"
      [ -n "$_mf" ] || _mf=/dev/null
      _emit "$fmt" "$n" "$_mf" external "$_endpoint"
    done <<EOF
$(external_capabilities)
EOF
  fi

  # Spine last: it is the shell that consumes the capabilities, so bringing it up
  # after them means its first discovery call already sees the truth.
  while IFS= read -r n; do
    [ -n "$n" ] || continue
    _emit "$fmt" "$n" "$SJEL_ROOT/$n/service.toml" spine
  done <<EOF
$(_spine_names)
EOF
  if [ "$fmt" = json ]; then printf '\n]\n'; fi
}

cmd_disable() {  # <name>
  local name="$1"
  local enabled; enabled=" $(_enabled_names) "

  case "$enabled" in
    *" $name "*) ;;
    *) echo "capability.sh: '$name' is not enabled — nothing to do."; exit 0 ;;
  esac

  # Block if any OTHER enabled capability directly requires this one — removing
  # it would strand that dependent. Direct requires suffices: a transitive
  # dependent reaches <name> only through an enabled intermediate that itself
  # directly requires it, and that intermediate is caught here.
  local dependents="" e dep
  for e in $(_enabled_names); do
    [ "$e" = "$name" ] && continue
    for dep in $(_cap_requires "$e"); do
      [ "$dep" = "$name" ] && dependents="$dependents $e"
    done
  done
  if [ -n "$dependents" ]; then
    echo "capability.sh: cannot disable '$name' — still required by:${dependents}" >&2
    echo "disable the dependent(s) first." >&2
    exit 1
  fi

  local kept="" n
  for n in $(_enabled_names); do
    [ "$n" = "$name" ] || kept="$kept $n"
  done
  _write_capabilities "$kept"
  echo "Disabled '$name'. machine.toml: capabilities updated."

  # Persistence disposition, stated rather than assumed (#9). A leftover unit is NOT inert:
  # watchdog.sh calls `service-runner.sh start <cap>` every 30s and consults nothing about the
  # enabled set, so persistence left behind here walks the capability straight back up — a
  # disable that silently does not disable. Reported, not performed: unloading a launchd or
  # systemd unit is a machine-level side effect, and this tool starts and stops nothing.
  # tools/doctor repeats it for as long as it is true, so ignoring this line does not bury it.
  local state
  state="$("$TOOLS_DIR/service-runner.sh" persistence-status "$name" 2>/dev/null | cut -f2)" || state=""
  case "$state" in
    installed|installed-not-loaded|stale)
      echo
      echo "Persistence is still installed for '$name'. Its watchdog will keep starting it every 30s."
      echo "  tools/service-runner.sh remove-persistence $name"
      ;;
  esac
}

case "${1:-list}" in
  list)     cmd_list ;;
  enable)   [ -n "${2:-}" ] || { echo "usage: capability.sh enable <name>" >&2; exit 1; }; cmd_enable "$2" ;;
  disable)  [ -n "${2:-}" ] || { echo "usage: capability.sh disable <name>" >&2; exit 1; }; cmd_disable "$2" ;;
  registry) cmd_registry "${2:-}" ;;
  *)        echo "usage: capability.sh list | enable <name> | disable <name> | registry [--lines]" >&2; exit 1 ;;
esac

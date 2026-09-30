#!/bin/bash
# The shared "service manifest" interpreter (schemas/service.toml.example).
# Capabilities declare WHAT they need; this is the ONLY place that knows HOW to
# satisfy it on this machine. Three kinds, one interpreter: `kind = "container"`
# (the default) hands it to the container runtime, `kind = "process"` execs a host
# process directly -- a compiled server, or a dev server whose whole point is that
# it reloads while it runs. A new capability of either kind is a new service.toml,
# not a new script.
#
# `kind = "data"` is the third and runs nothing at all: a file this machine owns, whose
# manifest exists so tools/backup.sh has one owner to read a contract from. Every
# lifecycle verb refuses it by name, and the whole-machine fan-out skips it.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SJEL_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"
source "$TOOLS_DIR/lib/paths.sh"
source "$TOOLS_DIR/lib/platform.sh"
source "$TOOLS_DIR/lib/toml.sh"
# Canonical publish-set form, so a declaration and a running container can be compared
# at all (each runtime reports its bindings in its own shape).
source "$TOOLS_DIR/lib/runargs.sh"
# `does this stream contain X` without the answer depending on where the match sits (#42).
source "$TOOLS_DIR/lib/pipe.sh"
# The `schedule` duration parser, shared with tools/check-service-tomls.sh so the gate that
# accepts a manifest and the runner that installs it can never disagree about what it means.
source "$TOOLS_DIR/lib/schedule.sh"
# Which capabilities this machine consumes rather than runs — the one thing this runner must
# refuse to act on (retired-tracker#169).
source "$TOOLS_DIR/lib/external-ref.sh"

usage() {
  echo "usage: service-runner.sh <start|stop|idle-stop|restart|resume|recreate|status> <capability>" >&2
  echo "       service-runner.sh <install-persistence|remove-persistence|persistence-status> <capability>" >&2
  echo "       service-runner.sh persistence                     # persistence state for the whole enabled set" >&2
  echo "       service-runner.sh recreate <capability>          # rebuild the container from the current declaration" >&2
  echo "       service-runner.sh stop <capability> [--no-hold]   # --no-hold: do not keep it down" >&2
  echo "       service-runner.sh up [--all]     # start the autostart set (--all: everything enabled)" >&2
  echo "       service-runner.sh down           # stop everything enabled, dependents first; no hold" >&2
  echo "       service-runner.sh status         # one line per enabled service" >&2
  exit 1
}
CMD="${1:-}"; CAP="${2:-}"; FLAG="${3:-}"
case "$FLAG" in ''|--no-hold) ;; *) echo "service-runner.sh: unknown flag '$FLAG'" >&2; usage ;; esac
[ -n "$CMD" ] || usage

# SJEL_CONTAINER_RUNTIME is manifest vocabulary that happens to be the command name for
# both runtimes left. Resolve the binary lazily, on the first container operation, so a
# machine running only process capabilities never needs a container runtime installed at
# all -- and an unresolvable runtime still says so by name. It used to surface as bash's
# `command not found` from inside start_service -- which is how the launchd-PATH defect
# stayed invisible for two weeks (see install_persistence). That defect is why the lookup
# still runs: on macOS the docker CLI is OrbStack's, at ~/.orbstack/bin/docker, which a
# login shell has on PATH and a bare launchd or cron environment does not.
RUNTIME_BIN=""; RUNTIME_PATH=""
resolve_runtime() {
  [ -z "$RUNTIME_PATH" ] || return 0
  case "$SJEL_CONTAINER_RUNTIME" in
    docker|podman)   RUNTIME_BIN="$SJEL_CONTAINER_RUNTIME" ;;
    *)
      echo "service-runner.sh: unsupported container_runtime '$SJEL_CONTAINER_RUNTIME' (axon-overlay/config/machine.toml)" >&2
      exit 1
      ;;
  esac
  RUNTIME_PATH="$(command -v "$RUNTIME_BIN" 2>/dev/null || true)"
  [ -n "$RUNTIME_PATH" ] || {
    echo "service-runner.sh: container_runtime '$SJEL_CONTAINER_RUNTIME' needs '$RUNTIME_BIN' on PATH, not found (PATH=$PATH)" >&2
    exit 1
  }
}

# --- fan-out over the enabled set ----------------------------------------
# No capability argument means "this whole machine". The order is capability.sh's
# registry order -- dependency-first, from the one resolver that already exists, never
# a second copy of it here. `down` walks it backwards so nothing is stopped while
# something that requires it is still up.
registry_lines() { "$TOOLS_DIR/capability.sh" registry --lines; }

fan_out() {  # <start|stop|status> [--all] [flag passed to each invocation]
  local op="$1" all="${2:-}" extra="${3:-}" names="" name kind scope autostart
  while read -r name kind scope autostart _; do
    [ -n "$name" ] || continue
    # kind=data is a file this machine owns, not a process (capabilities/store). Skipped
    # here rather than refused per verb: `down` and `status` walk every row regardless of
    # autostart, and a whole-machine verb reporting a failure for a row that can never
    # have one would make its exit status stop meaning anything.
    if [ "$kind" = data ]; then continue; fi
    if [ "$op" = start ] && [ "$all" != "--all" ] && [ "$autostart" != "true" ]; then
      continue
    fi
    names="$names $name"
  done <<EOF
$(registry_lines)
EOF
  [ -n "$names" ] || { echo "nothing to do (no matching services in the registry)"; return 0; }

  if [ "$op" = stop ]; then
    local reversed=""
    for name in $names; do reversed="$name $reversed"; done
    names="$reversed"
  fi
  local rc=0
  for name in $names; do
    # `|| rc=1` on purpose: one capability refusing to stop must not hide the state of the
    # rest. The failure is part of an || list, so `set -e` does not fire here and the walk
    # completes before the non-zero status is returned.
    "$0" "$op" "$name" ${extra:+"$extra"} || rc=1
  done
  return $rc
}

case "$CMD" in
  up)
    if [ -n "$CAP" ] && [ "$CAP" != "--all" ]; then usage; fi
    fan_out start "$CAP"; exit $?
    ;;
  down)
    if [ -n "$CAP" ]; then usage; fi
    # --no-hold, so `down` then `up` is not a dead end. `stop <cap>` keeps a capability down on
    # purpose -- it exists so a tool can work on that capability's data while nothing has it open,
    # and tools/backup.sh relies on exactly that, taking the hold through `stop <cap>` and lifting
    # it through `resume <cap>`. `down` is the operator's "take it all down" verb and has no such
    # window to protect: holding there made every service answer the following `up` with "is held
    # for maintenance, not starting", so the pair the usage advertises no-opped on its second half.
    fan_out stop "" --no-hold; exit $?
    ;;
  status)
    if [ -z "$CAP" ]; then fan_out status; exit $?; fi
    ;;
  persistence)
    # The whole enabled set at once, because "which capabilities will not come back after a
    # reboot" is a question about the machine, not about one capability (#9). Per-capability
    # detail stays available as `persistence-status <cap>`.
    if [ -n "$CAP" ]; then usage; fi
    fan_out persistence-status; exit $?
    ;;
esac
[ -n "$CAP" ] || usage

# --- maintenance hold -----------------------------------------------------
# `stop` takes a capability down AND keeps it down, so a tool can work on its data
# while nothing has it open (tools/backup.sh's cold SQLite copy); `resume` lifts the
# hold and brings it back. Without this the watchdog would race the container back up
# mid-copy 30s later. The lock lives here because this file is the only one that knows
# HOW a capability runs -- no other tool needs to learn the convention.
#
# Fail-safe in the direction that matters: the lock sits in boot-cleared /tmp and
# expires, so a crashed holder can never leave a password manager permanently
# un-restartable. Ignoring a stale hold is reported on stderr, which is exactly where
# the watchdog now writes -- so it surfaces instead of rotting.
#
# Hardcoded /tmp, NOT $TMPDIR: the holder runs in a login shell (per-user
# /var/folders/... tmpdir) while the watchdog runs under launchd, and the two must agree
# on one path or the hold silently protects nothing. Same convention as the watchdog's
# own /tmp/axon-<cap>-watchdog.{log,err}.
MAINT_LOCK="/tmp/axon-$CAP.maintenance"
MAINT_MAX_AGE=1800

maintenance_hold_active() {
  [ -f "$MAINT_LOCK" ] || return 1
  local mtime now age
  # GNU first, BSD second, and the order is load-bearing. `stat -f` on Linux means "display FILE
  # SYSTEM status" and SUCCEEDS, printing a multi-line report starting with `File: "..."` -- so
  # the old chain (-f then -c) never reached the Linux branch, fed that report to the arithmetic
  # below, and died with `File: unbound variable` under set -u. Every hold-consulting command was
  # therefore broken on Linux, including the resume tools/backup.sh depends on (#31).
  mtime="$(stat -c %Y "$MAINT_LOCK" 2>/dev/null || stat -f %m "$MAINT_LOCK" 2>/dev/null || true)"
  # A hold that cannot be dated is not a hold that has expired. Keep it and say so: dropping it
  # would let the watchdog race whatever opened the window.
  case "$mtime" in
    ''|*[!0-9]*)
      echo "service-runner.sh: cannot read the age of '$MAINT_LOCK' — treating the hold as active" >&2
      return 0
      ;;
  esac
  now="$(date +%s)"
  age=$(( now - mtime ))
  if [ "$age" -gt "$MAINT_MAX_AGE" ]; then
    echo "service-runner.sh: ignoring stale maintenance hold on '$CAP' (${age}s old, max ${MAINT_MAX_AGE}s): $MAINT_LOCK" >&2
    rm -f "$MAINT_LOCK"
    return 1
  fi
  return 0
}

# A capability's manifest lives with the capability, in public Axon or in the active
# overlay; the spine's own shell (dashboard/, CONTRIBUTING.md#three-architectural-nouns)
# carries one at the repo root instead, because it is not a capability and never appears
# in machine.toml's enabled set. paths.sh owns the resolution order and the
# declared-twice refusal, so this stays the only place that turns a name into a manifest.
MANIFEST=""
_mf_rc=0
MANIFEST="$(axon_manifest_for "$CAP")" || _mf_rc=$?
if [ "$_mf_rc" -eq 2 ]; then
  exit 2   # axon_manifest_for already named both paths
fi
if [ -z "$MANIFEST" ]; then
  echo "service-runner.sh: no service.toml for '$CAP' (looked in $SJEL_CAPS_DIR/$CAP/, $SJEL_OVERLAY_CAPS_DIR/$CAP/ and $SJEL_ROOT/$CAP/)" >&2
  exit 1
fi
unset _mf_rc

# A capability this machine CONSUMES from another overlay's deployment (retired-tracker#169) has a
# manifest here — that is where its contracts live — but no process here to act on. Refused by
# name, since the fan-out can never reach one: `capability.sh registry --lines` leaves external
# rows out precisely so a whole-machine `up` never walks across a tailnet. Without this the
# manifest would be read, a local container or binary looked for, and its absence reported as a
# broken capability rather than as someone else's, running fine.
if [ -n "$(capability_provider "$CAP")" ]; then
  echo "service-runner.sh: '$CAP' is provided by another deployment — [capability.$CAP] provided_by in $SJEL_MACHINE_TOML." >&2
  echo "  This machine may read its health; its lifecycle belongs to whoever owns its host." >&2
  exit 1
fi

# Relative paths in a manifest resolve against the root that manifest came from, so an
# overlay capability's workdir and build output stay inside the overlay. A manifest that
# genuinely needs an Axon path — an overlay capability driven by a shared tool, say —
# writes ${SJEL_ROOT} and gets it expanded below.
case "$MANIFEST" in
  "$SJEL_OVERLAY_CAPS_DIR"/*) CAP_ROOT="$SJEL_OVERLAY_ROOT" ;;
  *)                          CAP_ROOT="$SJEL_ROOT" ;;
esac

NAME="$(toml_get name "$MANIFEST")"
KIND="$(toml_get kind "$MANIFEST")"
[ -n "$KIND" ] || KIND="container"
AUTOSTART="$(toml_get autostart "$MANIFEST")"
# A periodic job: "run this every N", as opposed to autostart's "keep this up". Read here beside
# autostart because the two are read together everywhere below — they are the two halves of one
# question (how does this capability get started when nobody types a command), and a manifest
# answering both at once is a contradiction this file refuses rather than picks a winner for.
SCHEDULE="$(toml_get schedule "$MANIFEST")"
# `full_disk_access = true`: on macOS a scheduled run starts through the signed
# axon-fda-launcher, which holds the Full Disk Access grant, instead of as /bin/bash, which
# has none. Measured 2026-09-25: without it the backup could not read the vault or list its
# own iCloud Drive destination. tools/fda-launcher/src/main.rs says why it spawns, not execs.
FULL_DISK_ACCESS="$(toml_get full_disk_access "$MANIFEST")"

# The one local model runtime this machine has. `libs/inference` reads it as
# SJEL_INFERENCE_BACKEND and moves every role whose declared backend is loopback onto it,
# taking that role's `on_backend` model id with it — a backend id on its own would ask
# Ollama for an MLX model name. An Intel or Pi host that has only Ollama says so once here
# and no capability config changes.
#
# Home: `[inference] backend` in the overlay's machine.toml, the same file and the same
# single-line contract `[capability.<name>] port` below already uses, because it is the same
# shape of fact — true for one machine, unable to live in a tracked shared file. Machine-global
# rather than per-capability, so it is read once at this level and inherited by both the
# supervised and the scheduled branch; a capability that does no inference never sees it used.
# Passed through unvalidated on purpose: whether the id names a declared backend is a question
# about inference.json, and libs/inference already answers it by name.
if [ -f "$SJEL_MACHINE_TOML" ]; then
  _inference_backend="$(toml_get_in inference backend "$SJEL_MACHINE_TOML")"
  if [ -n "$_inference_backend" ]; then
    export SJEL_INFERENCE_BACKEND="$_inference_backend"
  fi
  unset _inference_backend
fi

# Which hosts this operator owns (PRD Q39, 2026-08-25). `libs/inference` reads it as
# SJEL_INFERENCE_TRUSTED_PEERS and trusts a backend whose `provided_by` names one of these
# exactly as it trusts loopback — no data class withheld, and NOT classified as a cloud
# provider needing a reviewed policy.
#
# Resolved here rather than in Rust because the declaration lives in the overlay's
# systems.local.toml and tools/lib/external-ref.sh is the one resolver for that file. Q39
# says extend it, not invent a second one; this export is how the answer crosses over, the
# same way [inference] backend does two lines above.
#
# Empty is the normal state on a single-host deployment and produces exactly the behaviour
# that shipped before Q39: an unset variable declares no peers, and every backend that is
# not loopback stays a cloud endpoint.
_trusted_peers="$(trusted_peers_env)"
if [ -n "$_trusted_peers" ]; then
  export SJEL_INFERENCE_TRUSTED_PEERS="$_trusted_peers"
fi
unset _trusted_peers

container_init() {  # every container-only manifest field, read only when it applies
IMAGE="$(toml_get image "$MANIFEST")"
TAG="$(toml_get tag "$MANIFEST")"
ENV_FILE_REL="$(toml_get env_file "$MANIFEST")"
ENV_FILE="$SJEL_PERSONAL_ROOT/$ENV_FILE_REL"
NETWORK_MODE="$(toml_get network_mode "$MANIFEST")"

PORTS=()
while IFS= read -r line; do [ -n "$line" ] && PORTS+=("$line"); done < <(toml_array ports "$MANIFEST")
VOLUMES=()
while IFS= read -r line; do [ -n "$line" ] && VOLUMES+=("$line"); done < <(toml_array volumes "$MANIFEST")
CAP_ADD=()
while IFS= read -r line; do [ -n "$line" ] && CAP_ADD+=("$line"); done < <(toml_array cap_add "$MANIFEST")

# `ports` is the one manifest field that describes the HOST rather than the capability:
# the shipped vaultwarden manifest publishes 0.0.0.0 for the family Pi, where
# `tailscale serve` terminates TLS in front of it and needs the container reachable on
# the host's tailnet address. The Mac has no such front, so it overrides to loopback
# (2026-07-30) — a vault on the house LAN was the bug that motivated this. service.toml is
# tracked and shared, so that difference cannot live there — same reasoning and the same
# home as every other machine-local field: section [capability.<name>] in
# <overlay>/config/machine.toml (schemas/machine.toml.example).
# Absent file, section or key leaves the manifest value standing, so a machine that
# overrides nothing behaves exactly as before.
if [ -f "$SJEL_MACHINE_TOML" ]; then
  PORT_OVERRIDE=()
  while IFS= read -r line; do [ -n "$line" ] && PORT_OVERRIDE+=("$line"); done \
    < <(toml_array_in "capability.$CAP" ports "$SJEL_MACHINE_TOML")
  if [ ${#PORT_OVERRIDE[@]} -gt 0 ]; then
    PORTS=("${PORT_OVERRIDE[@]}")
  fi
fi

CONTAINER_ARGS=(--name "$NAME")
# Every optional array below expands as ${A[@]+"${A[@]}"}, never as "${A[@]}": bash 3.2
# under `set -u` treats an empty array's plain expansion as an unbound variable and dies.
# The red path is the machine that does NOT declare the field, so the naive form works
# everywhere it is tested and breaks on the first capability that omits one.
#
# cap_add: Linux capabilities the image needs beyond the runtime's default set — pihole's
# NET_ADMIN, and nothing else so far. Bare names, no CAP_ prefix: docker's canonical form,
# which podman takes too.
for c in ${CAP_ADD[@]+"${CAP_ADD[@]}"}; do CONTAINER_ARGS+=(--cap-add "$c"); done
if [ -n "$NETWORK_MODE" ]; then
  # host (or other) networking: join the host network namespace. `ports` are NOT
  # published — host mode binds the container's own ports directly. See
  # schemas/service.toml.example (required by home-assistant for LAN mDNS discovery).
  CONTAINER_ARGS+=(--network "$NETWORK_MODE")
else
  for p in ${PORTS[@]+"${PORTS[@]}"}; do CONTAINER_ARGS+=(-p "$p"); done
fi
# The `managed_volume = "true"` field and the runtime volume it created stood here until
# 2026-09-02 (Q75). It existed for exactly one runtime property — apple-container's
# virtiofs bind mounts refused guest-side chown/chmod — and docker and podman share the host
# filesystem with real Unix ownership, so with apple-container gone the field had nothing
# left to do. Its only consumer, postgres, retired on 2026-08-27 (PRD Q45). The PARSE is
# deleted along with the branch, deliberately: a field still read and then ignored would put
# a capability's data somewhere other than where its manifest says, with nothing reporting it.
for v in ${VOLUMES[@]+"${VOLUMES[@]}"}; do
  host_path="${v%%:*}"
  container_path="${v#*:}"
  if [ "${host_path#/}" != "$host_path" ]; then
    # An absolute host path is a system path the container needs from the machine
    # itself, not data the overlay owns: /run/dbus so home-assistant can reach the
    # host's message bus for Bluetooth, /etc/localtime so its clock matches. Used
    # verbatim, and deliberately NOT created — a missing one means the host is not
    # what the manifest assumed, and mkdir would paper over that with an empty
    # directory the container then mounts and silently does without.
    [ -e "$host_path" ] || {
      echo "service-runner.sh: $CAP declares the system path $host_path, which does not exist on this host" >&2
      exit 1
    }
    CONTAINER_ARGS+=(-v "$host_path:$container_path")
  else
    resolved="$SJEL_PERSONAL_ROOT/$host_path"
    mkdir -p "$resolved"
    CONTAINER_ARGS+=(-v "$resolved:$container_path")
  fi
done
CONTAINER_ARGS+=(--env-file "$ENV_FILE")
}

# --- process kind ---------------------------------------------------------
# Everything a host process needs, read the same lazy way. The pid file, log and
# error paths follow the watchdog's /tmp convention for the same reason it does: the
# holder may be a login shell (per-user /var/folders tmpdir) while the supervisor runs
# under launchd, and the two have to agree on one path or a "stop" silently stops
# nothing.
PID_FILE="/tmp/axon-$CAP.pid"
PROC_LOG="/tmp/axon-$CAP.log"
PROC_ERR="/tmp/axon-$CAP.err"

COMMAND=(); BUILD_CMD=(); PANEL_BUILD_CMD=(); WORKDIR=""; PORT=""; HEALTH_PATH=""; BUILD_OUTPUT=""; SIGN_ID=""

process_init() {
  while IFS= read -r line; do [ -n "$line" ] && COMMAND+=("$line"); done < <(toml_array command "$MANIFEST")
  while IFS= read -r line; do [ -n "$line" ] && BUILD_CMD+=("$line"); done < <(toml_array build "$MANIFEST")
  while IFS= read -r line; do [ -n "$line" ] && PANEL_BUILD_CMD+=("$line"); done < <(toml_array panel_build "$MANIFEST")
  BUILD_OUTPUT="$(toml_get build_output "$MANIFEST")"
  SIGN_ID="$(toml_get sign "$MANIFEST")"
  WORKDIR="$(toml_get workdir "$MANIFEST")"
  PORT="$(toml_get port "$MANIFEST")"
  HEALTH_PATH="$(toml_get health_path "$MANIFEST")"

  [ ${#COMMAND[@]} -gt 0 ] || {
    echo "service-runner.sh: '$CAP' is kind=process but declares no command = [...]" >&2
    exit 1
  }

  # ${SJEL_ROOT} and ${SJEL_OVERLAY_ROOT} in any argument expand first. They exist so an
  # overlay capability can name a shared Axon tool, and hand that tool a path back into
  # the overlay, without either side hardcoding one machine's checkout location.
  # ${SJEL_PORT} is the third and last interpolation, and it deliberately expands later —
  # see below, after the machine.toml override has had its say.
  local _i
  for _i in "${!COMMAND[@]}"; do
    COMMAND[$_i]="${COMMAND[$_i]//\$\{SJEL_ROOT\}/$SJEL_ROOT}"
    COMMAND[$_i]="${COMMAND[$_i]//\$\{SJEL_OVERLAY_ROOT\}/$SJEL_OVERLAY_ROOT}"
  done

  # command[0] is resolved against the capability's own root, not against workdir: a
  # manifest naming target/release/... means that path in the checkout it came from,
  # wherever the process then runs. A bare name (bun, uv) is left alone -- PATH's job.
  case "${COMMAND[0]}" in
    /*) ;;
    */*) COMMAND[0]="$CAP_ROOT/${COMMAND[0]}" ;;
  esac

  # Same per-machine override seam the container `ports` field has: a port is a fact
  # about the host, and service.toml is tracked and shared.
  if [ -f "$SJEL_MACHINE_TOML" ]; then
    local override
    override="$(toml_get_in "capability.$CAP" port "$SJEL_MACHINE_TOML")"
    if [ -n "$override" ]; then PORT="$override"; fi
  fi

  # ${SJEL_PORT} expands HERE, after the override above, so a manifest that passes its
  # port as an argument gets the same value the registry and the dev-server proxy read
  # from `port`. An Axon-owned process reads the SJEL_PORT env var this exports later; an
  # adopted binary takes its port on argv (macmon serve --port N) and cannot. Without this
  # the number would be written twice, and a machine.toml override would move one of them.
  for _i in "${!COMMAND[@]}"; do
    COMMAND[$_i]="${COMMAND[$_i]//\$\{SJEL_PORT\}/$PORT}"
  done
}

health_url() {
  [ -n "$PORT" ] && [ -n "$HEALTH_PATH" ] || return 1
  echo "http://127.0.0.1:$PORT$HEALTH_PATH"
}

process_healthy() {
  local url
  url="$(health_url)" || return 1
  curl -sf -o /dev/null --max-time 2 "$url"
}

# port_answers <port> — true when something accepts a TCP connection on 127.0.0.1:<port>.
#
# bash's /dev/tcp rather than lsof or ss: neither is declared in toolchain.toml, and a stop path
# that silently skips its own safety check on a host without lsof is the failure this exists to
# prevent. It answers "is the port held", not "by whom" — which is all the stop path needs, and
# deliberately not enough to justify killing an unidentified process.
port_answers() {
  local p="${1:-}"
  [ -n "$p" ] || return 1
  (: < "/dev/tcp/127.0.0.1/$p") 2>/dev/null
}

running_pid() {  # echo the live pid, or nothing
  [ -f "$PID_FILE" ] || return 0
  local pid
  pid="$(cat "$PID_FILE" 2>/dev/null || true)"
  case "$pid" in ''|*[!0-9]*) return 0 ;; esac
  kill -0 "$pid" 2>/dev/null && echo "$pid"
  return 0
}

# Kill the tree, leaves first. `bun run dev` is a supervisor of its own -- vite is its
# child, and killing only the parent leaves the port held by an orphan. pgrep -P is on
# both macOS and Linux; recursion depth here is 2-3 in practice.
kill_tree() {
  local pid="$1" child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do kill_tree "$child"; done
  kill -TERM "$pid" 2>/dev/null || true
}

maybe_build() {  # [force] — build when the artifact is missing, or always on restart
  [ ${#BUILD_CMD[@]} -gt 0 ] || return 0
  if [ "${1:-}" != "force" ]; then
    # What "already built" means, in the order the manifest can say it:
    #
    #   build_output — an explicit path the build produces. Needed by any capability
    #     whose command[0] is a bare interpreter (bun, uv): the old rule below returned
    #     early on those, so a manifest could declare a build that silently never ran.
    #     A panel serving a `dist/` that does not exist yet is exactly that case.
    #   command[0] — the compiled binary, for every Rust capability. Unchanged.
    if [ -n "$BUILD_OUTPUT" ]; then
      case "$BUILD_OUTPUT" in
        /*) if [ -e "$BUILD_OUTPUT" ]; then return 0; fi ;;
        *)  if [ -e "$CAP_ROOT/$BUILD_OUTPUT" ]; then return 0; fi ;;
      esac
    else
      case "${COMMAND[0]}" in
        /*) if [ -e "${COMMAND[0]}" ]; then return 0; fi ;;
        *)  return 0 ;;   # a bare name on PATH, and nothing declared to look for
      esac
    fi
  fi
  # The panel first, because the server binary serves that bundle: a start that compiles
  # the binary and then fails on the UI leaves a capability answering /health with a 404
  # panel. Fixed directory, not a declared one — CONTRIBUTING.md#placement-guide puts a
  # capability's own UI at <capability>/ui/, so the manifest has no per-capability fact to
  # state. node_modules is assumed installed, the same assumption dashboard/service.toml's
  # `bun run dev` already makes.
  if [ ${#PANEL_BUILD_CMD[@]} -gt 0 ]; then
    echo "building $CAP panel: ${PANEL_BUILD_CMD[*]}"
    ( cd "${MANIFEST%/service.toml}/ui" && "${PANEL_BUILD_CMD[@]}" )
  fi
  echo "building $CAP: ${BUILD_CMD[*]}"
  # In the capability's own workdir, not the repo root: `bun run build` has to run where
  # the package.json is. A capability without a workdir (every Rust one) still builds at
  # the root, which is where cargo resolves the workspace and its shared target/.
  ( cd "$CAP_ROOT/${WORKDIR:-.}" && "${BUILD_CMD[@]}" ) || return 1
  # `sign = "<identifier>"` signs the built command binary with a stable identity, so a
  # firewall or privacy grant survives rebuilds (tools/codesign-binary.sh). Here, after the
  # build, and not inside the build command: the unit's PATH is derived from the build
  # command's first word, and wrapping cargo in `sh -c` left cargo off it (2026-09-27, the
  # shell stayed down until built by hand).
  if [ -n "$SIGN_ID" ]; then
    local bin="${COMMAND[0]}"
    case "$bin" in /*) ;; *) bin="$CAP_ROOT/$bin" ;; esac
    "$TOOLS_DIR/codesign-binary.sh" "$bin" "$SIGN_ID"
  fi
}

wait_healthy() {  # returns 0 as soon as the service answers; 1 after the deadline
  health_url >/dev/null || return 0   # nothing declared to wait on
  local i=0
  while [ "$i" -lt 60 ]; do           # 60 * 0.5s = 30s, enough for a cold Next/Vite boot
    if process_healthy; then return 0; fi
    if [ -z "$(running_pid)" ]; then
      echo "service-runner.sh: '$CAP' exited during startup — tail $PROC_ERR" >&2
      return 1
    fi
    sleep 0.5
    i=$((i + 1))
  done
  echo "service-runner.sh: '$CAP' started but never answered $(health_url) — tail $PROC_ERR" >&2
  return 1
}

start_process() {
  if maintenance_hold_active; then
    echo "service-runner.sh: '$CAP' is held for maintenance, not starting ($MAINT_LOCK)"
    return 0
  fi

  # A scheduled job is started, not supervised: it runs, does its work and exits, and the
  # question the caller has is "did that work", not "is it up". So it runs in the FOREGROUND and
  # this function's exit status is the job's.
  #
  # Backgrounding it, which is what every other process capability wants, broke both halves of
  # that. `nohup ... >>$PROC_LOG` sent its output to the service log while the launchd plist and
  # the systemd unit declare /tmp/axon-<cap>-schedule.{log,err} — so the files the schedule
  # promises stayed 0 bytes, and the run's actual output landed in a file named for a service
  # that is not running. And `start` returned 0 the moment the fork succeeded, so a job that
  # failed every six hours reported success to the supervisor every six hours. Found by adding
  # the first `schedule` consumer: the first real run 400'd, and every surface said fine.
  if [ -n "$SCHEDULE" ]; then
    maybe_build
    STARTED_DEPS=""
    # Bring up what this job talks to, first.
    #
    # `requires` was a build-and-enable-time relation only: capability.sh resolves it transitively
    # when enabling, the manifest gate checks it resolves, and nothing acted on it at RUN time. For
    # an autostart capability that was invisible, because its dependencies were autostart too.
    #
    # It stops being invisible the moment capabilities go on-demand. sparpreis-watch requires
    # transit and trips, both of which are started by a page asking for them -- and a 12-hourly job
    # is not a page. Measured 2026-08-29: the job failed with ConnectionRefused against transit,
    # and had been failing that way whenever it ran, because transit has never been autostart.
    #
    # Only for a scheduled job, deliberately. A long-running capability that needs another one is a
    # startup-ordering question the supervisor already owns; a job that runs and exits has no
    # supervisor watching it and one shot at finding its dependencies up.
    while IFS= read -r dep; do
      [ -n "$dep" ] || continue
      # Already up is the common case and costs one probe. `status` is the runner's own answer, so
      # this cannot drift from what every other verb means by "running" -- but it is a padded
      # table, not a bare word, so it is matched rather than compared. (An equality test against
      # "running" was written first and silently never matched, which would have restarted every
      # dependency on every tick.)
      _dep_status="$("$0" status "$dep" 2>/dev/null | head -1)"
      case "$_dep_status" in *running*) continue ;; esac
      # An operator held this capability. A background tick must not quietly undo that: `resume`
      # is what a PERSON asking through the shell gets, and a 12-hourly timer is not a person.
      # Said out loud instead, so the job's failure a second later reads as a consequence rather
      # than a mystery.
      case "$_dep_status" in
        *held*)
          echo "service-runner.sh: $CAP requires $dep, which is HELD — not overriding an operator hold." >&2
          echo "  Release it with: tools/service-runner.sh resume $dep" >&2
          continue
          ;;
      esac
      echo "service-runner.sh: $CAP requires $dep — starting it"
      if "$0" start "$dep" >&2; then
        STARTED_DEPS="$STARTED_DEPS $dep"
        # Wait for it to ANSWER, not merely to exist. `start` returns once the process is forked,
        # which is well before an HTTP server has bound its port — so without this the job races
        # the dependency it just asked for and loses. Measured: sparpreis-watch died with
        # ConnectionRefused against trips on :8086 while trips was starting perfectly.
        #
        # Bounded, and a timeout is not fatal here. The job is about to try the dependency itself
        # and will produce a better error than this loop can; refusing to run would replace one
        # capability's failure with the whole job's.
        # What "ready" means depends on what the dependency declared. With a health path, ready is
        # `healthy` — the port being bound is not the same as the server answering. Without one
        # there is nothing to poll, so `running` is the strongest available answer and waiting for
        # `healthy` would burn the whole timeout on every start.
        _dep_mf="$SJEL_ROOT/capabilities/$dep/service.toml"
        _dep_health="$(toml_get health_path "$_dep_mf" 2>/dev/null)"
        _want="running"
        [ -n "$_dep_health" ] && _want="healthy"
        _waited=0
        while [ "$_waited" -lt 30 ]; do
          # `case` over the captured line, not `grep -q`: under `set -o pipefail` -q exits the
          # moment it matches, and a producer still writing then fails the pipeline (tools/pipe.test.sh).
          case "$("$0" status "$dep" 2>/dev/null | head -1)" in *"$_want"*) break ;; esac
          sleep 1
          _waited=$((_waited + 1))
        done
        [ "$_waited" -lt 30 ] || echo "service-runner.sh: $dep did not report $_want in ${_waited}s; running $CAP anyway" >&2
      else
        echo "service-runner.sh: could not start $dep; $CAP may fail" >&2
      fi
    done < <(toml_array requires "$MANIFEST")

    # Started for this run, so stopped after it. Otherwise "on demand" quietly becomes "on demand
    # once, then forever": the first 12-hourly tick brings transit and trips up and nothing ever
    # takes them down, which is most of B20 undone by a job nobody was watching.
    #
    # Only what THIS run started. A dependency that was already up belongs to whoever started it —
    # a page someone has open, or another job — and stopping it would make a background tick reach
    # out and break something in front of a person.
    #
    # `idle-stop`, not `stop`: the verb sjel-status' reaper already uses, and for the same reason.
    # A finished job is not a maintenance window, so it must not leave a hold that turns the next
    # start into a silent no-op.
    #
    # The race is real and accepted: somebody can open the page for a dependency while the job
    # runs, and this will stop it underneath them. The window is one job, and the capability comes
    # back the moment the page asks again — which is the whole point of on-demand. The alternative
    # is a leak that never resolves, and a leak is not a race you win by waiting.
    stop_started_deps() {
      local d
      for d in $STARTED_DEPS; do
        echo "service-runner.sh: $CAP started $d for this run — stopping it"
        "$0" idle-stop "$d" >&2 || echo "service-runner.sh: could not stop $d" >&2
      done
    }

    (
      cd "$CAP_ROOT/${WORKDIR:-.}"
      SJEL_SHELL_PORT="$(toml_get port "$SJEL_ROOT/dashboard/service.toml")"
      export SJEL_SHELL_PORT
      # No redirect and no pid file on purpose: stdout and stderr are inherited so the
      # supervisor's own capture is the one that gets them, and there is no long-lived process
      # for a pid file to describe.
      exec "${COMMAND[@]}"
    )
    # Captured before the cleanup runs, and returned after it. A job's exit code is the whole
    # signal the supervisor gets; letting `idle-stop` overwrite it would report every failed run
    # as a success as long as the teardown worked.
    _job_status=$?
    stop_started_deps
    return $_job_status
  fi

  if [ -n "$(running_pid)" ]; then return 0; fi   # already ours, already up
  # Something else is on the port -- a hand-started dev server, or the survivor of a
  # lost pid file. Adopting it would be a lie (this script cannot stop what it did not
  # start) and starting a second one just fails to bind. Say which it is instead.
  if process_healthy; then
    echo "service-runner.sh: '$CAP' already answers on port $PORT but is not managed here — stop it yourself first if you want this to own it" >&2
    return 0
  fi
  maybe_build
  (
    cd "$CAP_ROOT/${WORKDIR:-.}"
    # One number, one declaration. The manifest's `port` (after any machine-local
    # override) is what the dashboard proxies to and what sjel-status polls, so it has
    # to be what the process binds -- otherwise the registry describes a service that
    # is listening somewhere else. A capability honours SJEL_PORT above its own config;
    # one that ignores it is free to, and simply has to keep its config in step.
    if [ -n "$PORT" ]; then export SJEL_PORT="$PORT"; fi
    # Where the shell lives, for a capability that serves its own page and needs a way
    # back to it. Read from dashboard/service.toml — the port keeps exactly one home,
    # and a panel never learns a number. Only the port: the HOST has to come from the
    # browser's own `location`, or the link breaks the moment the dashboard is opened
    # over Tailscale rather than as localhost (same reasoning as api.ts's panelUrl).
    SJEL_SHELL_PORT="$(toml_get port "$SJEL_ROOT/dashboard/service.toml")"
    export SJEL_SHELL_PORT
    nohup "${COMMAND[@]}" >>"$PROC_LOG" 2>>"$PROC_ERR" &
    echo $! > "$PID_FILE"
  )
  wait_healthy
}

stop_process() {  # [hold|nohold]
  if [ "${1:-hold}" = hold ]; then : > "$MAINT_LOCK"; fi
  local pid; pid="$(running_pid)"
  if [ -n "$pid" ]; then
    kill_tree "$pid"
    local i=0
    while [ "$i" -lt 20 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.25; i=$((i + 1)); done
    kill -KILL "$pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"

  # The recorded pid being gone does not mean the service is. `bun run dev` supervises vite; when
  # the supervisor exits first its child is reparented, keeps the port, and running_pid() returns
  # nothing -- so kill_tree above never ran and this function used to remove the pid file and
  # report success over a service that is demonstrably still up. The next `start` then reached the
  # "already answers on port N but is not managed here" branch, which is accurate but leaves the
  # operator to clean up something `stop` claimed to have done.
  #
  # Give the tree a moment to release the socket first: a listener in TIME_WAIT-adjacent teardown
  # would otherwise make a successful stop look like a failure.
  if [ -n "$PORT" ]; then
    local waited=0
    while [ "$waited" -lt 12 ] && port_answers "$PORT"; do sleep 0.25; waited=$((waited + 1)); done
    if port_answers "$PORT"; then
      echo "service-runner.sh: '$CAP' still answers on port $PORT after stop — refusing to report success." >&2
      echo "  Something outside this pid file holds it (a reparented child, or an unrelated process)." >&2
      echo "  Identify and stop it, then re-run: lsof -ti tcp:$PORT   (or: ss -ltnp 'sport = :$PORT')" >&2
      # Deliberately not killing it. This check knows the port is held, not by what; killing an
      # unidentified listener on the operator's machine is a worse failure than refusing.
      return 1
    fi
  fi
}

status_process() {
  local pid state health
  pid="$(running_pid)"
  if [ -n "$pid" ]; then state="running (pid $pid)"; else state="stopped"; fi
  if health_url >/dev/null; then
    if process_healthy; then health="healthy"; else health="no answer"; fi
  else
    health="no health_path"
  fi
  if maintenance_hold_active 2>/dev/null; then state="$state, held"; fi
  printf '  %-14s %-9s %-22s %s\n' "$CAP" "process" "$state" "$health"
}

# ensure_runtime() and qualified_image() stood here until 2026-09-02 (Q75). Both
# existed for apple-container alone. ensure_runtime revived its apiserver, a launchd XPC
# service that a reboot left neither running nor registered; docker and podman daemons own
# their own lifecycle (OrbStack, Docker Desktop, systemd) and are deliberately not managed
# from here. qualified_image prefixed a Docker Hub short name with `docker.io/` because
# apple-container demanded an explicit registry; docker and podman resolve short names
# themselves, so start_service below passes `$IMAGE:$TAG` — the reference the manifest
# declares — through as written.

# Three states, not two. The old existence-only check re-issued `start` against an
# already-running container on every 30s watchdog cycle, and the resulting error is why
# watchdog.sh discarded its own output. Branch on running/stopped/absent and the healthy
# path is genuinely silent, which is what makes honest logging affordable.
# Idempotent: resume_service reaches start_service, and container_init appends to
# arrays, so calling it twice would build a doubled argv.
#
# There was a third step here, ensure_runtime, ordered between these two because
# container_init issued the process's first runtime CLI call and a dead apple-container
# apiserver killed the run before the recovery could fire (#125). It retired with that
# runtime on 2026-09-02 (Q75); container_init now issues no runtime call at all.
CONTAINER_READY=0
container_prepare() {
  if [ "$CONTAINER_READY" -eq 1 ]; then return 0; fi
  resolve_runtime
  container_init
  CONTAINER_READY=1
}

start_service() {
  container_prepare
  if maintenance_hold_active; then
    echo "service-runner.sh: '$CAP' is held for maintenance, not starting ($MAINT_LOCK)"
    return 0
  fi
  if "$RUNTIME_BIN" ps --format '{{.Names}}' 2>/dev/null | stream_matches -x "$NAME"; then
    :  # already running
  elif "$RUNTIME_BIN" ps -a --format '{{.Names}}' 2>/dev/null | stream_matches -x "$NAME"; then
    report_arg_drift >&2 || true
    "$RUNTIME_BIN" start "$NAME"
  else
    "$RUNTIME_BIN" run -d --restart unless-stopped "${CONTAINER_ARGS[@]}" "$IMAGE:$TAG"
  fi
}

# recreate — apply the current declaration to an existing container.
#
# Removal is safe for declared state by construction: container_init mounts every state path as a
# named volume or a host path, both of which outlive the container. What does NOT survive is state
# written inside the container and never declared -- which CONTRIBUTING.md#state-mounts-record-reality
# says should not exist, and which this makes visible if it does.
#
# The bounded part of the rollback is the manifest: the new container is built from the same
# resolved declaration a fresh install would use, so a failed `run` leaves the capability down with
# its data intact and its next `start` builds it again. There is no old spec to restore, because
# the old spec is exactly what is being discarded -- so this is a deliberate verb, never something
# `start` or `restart` does on its own.
recreate_service() {
  container_prepare
  echo "service-runner.sh: recreating '$CAP' — declared state mounts survive, undeclared in-container state does not"
  "$RUNTIME_BIN" stop "$NAME" >/dev/null 2>&1 || true
  "$RUNTIME_BIN" rm "$NAME" >/dev/null 2>&1 || "$RUNTIME_BIN" delete "$NAME" >/dev/null 2>&1 || true
  rm -f "$MAINT_LOCK"
  start_service
}

# Take it down and KEEP it down. Idempotent: an already-stopped capability is fine,
# the hold is what the caller is really after.
stop_service() {  # [hold|nohold]
  container_prepare
  if [ "${1:-hold}" = hold ]; then : > "$MAINT_LOCK"; fi
  "$RUNTIME_BIN" stop "$NAME" >/dev/null 2>&1 || true
}

resume_service() {
  rm -f "$MAINT_LOCK"
  start_service
}

# The runtime's own description of this container, as one JSON document.
#
# Kept in a variable and never in a file, deliberately: it contains the container's resolved
# environment, so writing it to $TMPDIR to answer a drift question would put every credential the
# capability holds on disk. One call per drift report, reused across all five classes.
inspect_json() {
  container_prepare
  "$RUNTIME_BIN" inspect "$NAME" --format '{{json .}}' 2>/dev/null
}

# The classes report_arg_drift walks, in the order an operator would act on them. Each one is an
# argument that `run -d` takes at creation and `start` cannot change (CONTRIBUTING.md#state-mounts-record-reality).
RUNARG_CLASSES="port mount cap network"

# report_arg_drift — print every difference between the declaration and the container, exit 1 if
# any. Writes to stdout; callers redirect.
#
# start_service starts an existing container by NAME and only builds a new one when none exists,
# so every `run -d` argument is frozen at creation time. A changed value parses, resolves, and
# never reaches the container; `restart` does not help either, because stop_service stops without
# removing. #11 said so for ports. Everything else stayed silent, and `--env-file` is the one that
# matters most: a rotated credential in the overlay looked applied when the container was still
# serving the old one.
report_arg_drift() {
  local json d r class out rc=0 envfile
  json="$(inspect_json)" || {
    echo "  the container runtime could not be asked about '$NAME' — drift unverified, NOT verified equal"
    return 1
  }
  if [ -z "$json" ] || [ "$json" = "[]" ] || [ "$json" = "null" ]; then
    echo "  no container named '$NAME' exists — drift unverified, NOT verified equal"
    return 1
  fi

  d="$(mktemp)"; r="$(mktemp)"
  declared_runspec ${CONTAINER_ARGS[@]+"${CONTAINER_ARGS[@]}"} | sort > "$d"
  printf '%s\n' "$json" | runspec_from_docker > "$r"

  for class in $RUNARG_CLASSES; do
    if ! out="$(runspec_diff "$d" "$r" "$class")"; then
      printf '  %s:\n%s\n' "$class" "$out"
      rc=1
    fi
  done

  # The env file last, and through its own comparison: its values are secrets, so they never join
  # the canonical stream above and only their key names are ever printed.
  envfile="$(grep '^envfile ' "$d" | head -1 | sed 's/^envfile //')"
  rm -f "$d" "$r"
  if [ -n "$envfile" ]; then
    out="$(printf '%s\n' "$json" | env_from_docker | env_diff "$envfile")" || rc=1
    [ -n "$out" ] && printf '  env-file:\n%s\n' "$out"
  fi

  [ "$rc" -eq 0 ] || echo "  the running container was created from a different declaration — 'service-runner.sh recreate $CAP' applies the current one"
  return $rc
}

status_service() {
  container_prepare
  local state="stopped" report="" classes=""
  if "$RUNTIME_BIN" ps --format '{{.Names}}' 2>/dev/null | stream_matches -x "$NAME"; then
    state="running"
  fi
  if maintenance_hold_active 2>/dev/null; then state="$state, held"; fi
  # Drift is only meaningful for a container that exists; a stopped-and-absent one gets its
  # declaration applied by the next start anyway. Captured once rather than asked three times:
  # the summary and the detail below are the same report, so they cannot disagree, and the
  # runtime is queried once instead of per class.
  if [ "${state#running}" != "$state" ] && ! report="$(report_arg_drift 2>/dev/null)"; then
    classes="$(printf '%s\n' "$report" | sed -n 's/^  \([a-z-]*\):$/\1/p' | tr '\n' ',' | sed 's/,$//; s/,/, /g')"
    # No class headings means the report is the "could not be asked" line, not a set of
    # differences — which must not render as a clean container either.
    state="$state, drift: ${classes:-unverified}"
  fi
  printf '  %-14s %-9s %-22s %s\n' "$CAP" "container" "$state" "$IMAGE:$TAG"
  [ -z "$report" ] || printf '%s\n' "$report" >&2
}

# --- boot persistence -----------------------------------------------------
#
# Rendering is separated from installing (#9) for one reason: nothing could ask whether a
# capability's persistence was actually in place. `install-persistence` existed, install.sh never
# called it, and neither did `capability.sh enable` — so a capability could declare autostart, be
# enabled, run all day, and simply be gone after the next reboot with no warning at any point.
# Answering "is it installed, and does it still match the declaration" means being able to render
# the unit WITHOUT loading it, which is what these three functions separate out.

# persistence_mode — which of three declarations this manifest makes, on stdout:
#
#   watchdog   autostart = "true"   keep it up; restart it if it dies
#   scheduled  schedule = "6h"      on-demand AND periodic: run it, let it exit, run it again
#   (empty)    neither              purely on-demand; a surface or an operator starts it
#
# The middle case is the third thing `install_persistence` had to learn (#129), and it is not
# "autostart, loosened". A watchdog calls `start` every 30s forever; a scheduled job is expected
# to EXIT between runs. Declaring both is a contradiction rather than a preference — the watchdog
# would hold the process up continuously, leaving nothing for an interval to start — so it is
# refused here instead of silently resolved in whichever direction the code happens to check first.
persistence_mode() {
  if [ "$AUTOSTART" = "true" ] && [ -n "$SCHEDULE" ]; then
    echo "service-runner.sh: '$CAP' declares autostart AND schedule = \"$SCHEDULE\"." >&2
    echo "  A watchdog keeps the process up continuously, so an interval would never have anything to start." >&2
    echo "  autostart is for a service, schedule is for a periodic job — declare one ($MANIFEST)." >&2
    return 1
  fi
  if [ "$AUTOSTART" = "true" ]; then echo watchdog; return 0; fi
  if [ -n "$SCHEDULE" ]; then echo scheduled; return 0; fi
  echo ""
}

# persistence_applicable — 0 when this capability should have a supervisor unit, 1 when it should
# not. Prints the reason on stdout either way, because "not applicable" is an answer a caller
# reports, not an error it swallows.
#
# A watchdog and an on-demand capability are opposite claims about the same process. The watchdog
# calls `start` every 30s and knows nothing about `autostart`, so installing one on a capability
# the manifest declares on-demand keeps it up forever while the Projects page says the opposite.
# The manifest is the authority; persistence is only meaningful for what is supposed to run.
persistence_applicable() {
  local mode secs
  if ! mode="$(persistence_mode)"; then
    echo "contradictory manifest: autostart and schedule cannot both be declared"
    return 1
  fi
  case "$mode" in
    scheduled)
      # No container-runtime short-circuit in this branch, deliberately. `--restart
      # unless-stopped` answers "bring it back when it dies", which is not "run it again in six
      # hours" — a scheduled container job needs the timer exactly as much as a process one does.
      if ! secs="$(schedule_seconds "$SCHEDULE")"; then
        echo "$secs"
        return 1
      fi
      echo "schedule declared — every ${secs}s"
      return 0
      ;;
    watchdog)
      # A container capability never gets a supervisor unit: both remaining runtimes restart
      # it themselves (`--restart unless-stopped`, start_service above), and a unit calling
      # `start` every 30s would fight the runtime. This branched on the runtime until
      # 2026-09-02 (Q75), because apple-container had no restart policy and was the
      # one case where a container did need a watchdog.
      if [ "$KIND" = container ]; then
        echo "$SJEL_CONTAINER_RUNTIME restarts it natively (--restart unless-stopped) — no watchdog needed"
        return 1
      fi
      echo "autostart declared"
      return 0
      ;;
    *)
      echo "on-demand (neither autostart nor schedule in the manifest) — a watchdog would defeat that"
      return 1
      ;;
  esac
}

# persistence_unit_path — where this OS keeps the unit. Non-zero, with the reason on stdout, for
# an OS with no backend. One home for the path so status, install and remove cannot disagree
# about which file they are talking about.
# The launchd label prefix. It was com.axon until 2026-09-26, when the product became Sjel;
# install-persistence and remove-persistence also clear a unit left under the old label.
UNIT_LABEL_PREFIX="com.sjel"
LEGACY_UNIT_LABEL_PREFIX="com.axon"

persistence_unit_path() {
  local systemd_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  case "$SJEL_OS" in
    macos) printf '%s\n' "$HOME/Library/LaunchAgents/$UNIT_LABEL_PREFIX.$CAP.plist" ;;
    linux)
      # For a scheduled job the TIMER is the primary unit: it holds the interval, it is what gets
      # enabled, and it is therefore what `installed` has to mean. Its oneshot companion is
      # rendered and compared alongside — persistence_companion_path.
      if [ "$(persistence_mode 2>/dev/null)" = scheduled ]; then
        printf '%s\n' "$systemd_dir/axon-$CAP.timer"
      else
        printf '%s\n' "$systemd_dir/axon-$CAP.service"
      fi
      ;;
    windows)
      echo "no windows persistence backend yet"
      return 1
      ;;
    *)
      echo "unknown os '$SJEL_OS' (machine.toml)"
      return 1
      ;;
  esac
}

# persistence_companion_path — the second file a scheduled systemd job needs, or non-zero when
# this case does not have one (every other combination is a single file).
#
# systemd splits WHEN from WHAT: axon-<cap>.timer carries the interval, axon-<cap>.service carries
# the command. launchd puts both in one plist, so this is the one place the two backends differ in
# shape rather than in syntax. Named separately rather than folded into render_persistence_unit so
# that `stale` can mean "either file drifted" — a hand-edited companion is exactly as broken as a
# hand-edited timer, and a check that only looked at one would report green for it.
persistence_companion_path() {
  [ "$SJEL_OS" = linux ] || return 1
  [ "$(persistence_mode 2>/dev/null)" = scheduled ] || return 1
  printf '%s\n' "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/axon-$CAP.service"
}

# persistence_path_dirs — the PATH the supervisor must inject.
#
# Both launchd and systemd --user hand a supervised process a MINIMAL environment that does NOT
# inherit the login shell's PATH. Every container runtime installs outside the bare set
# (/usr/local/bin, /opt/homebrew/bin on macOS; the runtime can sit outside /usr/bin on Linux too),
# so without an injected PATH the watchdog loops on exit 127 forever — which is exactly what
# launchd did from 2026-07-09 until 2026-07-25. Resolved from the real binary rather than a
# hardcoded candidate list: this machine's real answer lands in this machine's own unit, and Axon
# stays free of per-machine paths. Moving the runtime binary means re-rendering, which is now
# something `persistence-status` reports as stale rather than something nobody notices.
#
# Same defect, same fix, for a process capability: what it needs on PATH is its own interpreter
# (bun, uv) and its builder (cargo, bun), resolved here rather than hardcoded.
persistence_path_dirs() {
  local runtime_dir cmd_bin build_bin dep_mf dep_build_bin
  if [ "$KIND" = container ]; then
    resolve_runtime
    runtime_dir="$(dirname "$RUNTIME_PATH")"
  else
    runtime_dir=""
    case "${COMMAND[0]}" in
      /*) cmd_bin="${COMMAND[0]}" ;;
      *)  cmd_bin="$(command -v "${COMMAND[0]}" 2>/dev/null || true)" ;;
    esac
    if [ -n "$cmd_bin" ]; then runtime_dir="$(dirname "$cmd_bin")"; fi
    if [ ${#BUILD_CMD[@]} -gt 0 ]; then
      build_bin="$(command -v "${BUILD_CMD[0]}" 2>/dev/null || true)"
      if [ -n "$build_bin" ]; then runtime_dir="$runtime_dir:$(dirname "$build_bin")"; fi
    fi
    # A scheduled job that requires on-demand dependencies calls `service-runner.sh start <dep>`
    # from within the launchd environment, which in turn runs each dependency's build command
    # (e.g. `cargo build …`). Without the dependency's build tool on PATH that build fails with
    # "command not found" even though the tool exists on the login shell's PATH.
    # Measured 2026-09-30: sparpreis-watch's plist PATH contained only /opt/homebrew/bin (bun),
    # so `service-runner.sh start trips` hit `cargo: command not found` and trips never started.
    # Fix: also add every required capability's build tool dir to the rendered PATH.
    while IFS= read -r dep; do
      [ -n "$dep" ] || continue
      dep_mf="$SJEL_ROOT/capabilities/$dep/service.toml"
      [ -f "$dep_mf" ] || continue
      local dep_build_word
      dep_build_word="$(toml_array build "$dep_mf" 2>/dev/null | head -1)"
      [ -n "$dep_build_word" ] || continue
      dep_build_bin="$(command -v "$dep_build_word" 2>/dev/null || true)"
      if [ -n "$dep_build_bin" ]; then
        local dep_build_dir
        dep_build_dir="$(dirname "$dep_build_bin")"
        case ":$runtime_dir:" in *":$dep_build_dir:"*) ;; *)
          runtime_dir="$runtime_dir:$dep_build_dir" ;;
        esac
      fi
    done < <(toml_array requires "$MANIFEST")
  fi
  printf '%s\n' "$runtime_dir"
}

# persistence_env_block — the supervisor environment this machine declares for this capability,
# rendered in the unit's own syntax. Empty when nothing is declared.
#
# Why this exists (#44): the templates could carry exactly one variable, PATH. A capability that
# needs another had nowhere to put it, so the only way to get one in was to hand-edit the
# GENERATED unit — which `persistence-status` then correctly reports as `stale` forever, and which
# `install-persistence` silently deletes on the next run. The real case was the dashboard needing
# __VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS so its dev server accepts the machine's tailnet name.
#
# Home: `[capability.<name>] env = ["KEY=VALUE", ...]` in the overlay's machine.toml — the same
# section and the same single-line array contract `ports` already uses, because it is the same
# shape of fact: true for one machine, and unable to live in a tracked, shared service.toml.
#
# NOT for secrets. A unit file sits unencrypted in the operator's home and is read by a supervisor
# that logs; a credential belongs in the capability's env_file, which comes from Vaultwarden
# (CONTRIBUTING.md#secrets). Nothing here enforces that — it is a contract, stated where it is violated.
persistence_env_block() {
  local line key val out=""
  [ -f "$SJEL_MACHINE_TOML" ] || return 0
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    if [ "${line#*=}" = "$line" ]; then
      echo "service-runner.sh: [capability.$CAP] env entry '$line' has no '=' — expected KEY=VALUE" >&2
      return 1
    fi
    key="${line%%=*}"; val="${line#*=}"
    case "$SJEL_OS" in
      macos)
        # A plist value is XML text: an unescaped & or < makes the whole file unparseable, and
        # launchd's failure for that is silent.
        val="$(printf '%s' "$val" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')"
        key="$(printf '%s' "$key" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')"
        out="$out    <key>$key</key>
    <string>$val</string>
"
        ;;
      linux)
        # Quoted form: systemd otherwise stops a value at the first space.
        out="$out"'Environment="'"$key=$val"'"'"
"
        ;;
    esac
  done < <(toml_array_in "capability.$CAP" env "$SJEL_MACHINE_TOML")
  # Trailing newline trimmed: the placeholder occupies its own line, and awk re-adds one.
  printf '%s' "${out%
}"
}

# render_persistence_unit <dst> — write the unit this machine's declaration currently implies.
# Pure: no launchctl, no systemctl, no daemon-reload. That is what lets persistence_state render
# to a temp file and diff, and what lets the tests cover both OS branches on one host.
#
# Swaps the __EXTRA_ENV__ placeholder line for the rendered block, or drops the line when nothing
# is declared — so a machine declaring no env renders byte-for-byte what it rendered before that
# feature existed. Reads a unit on stdin, writes it on stdout.
#
# Done in bash rather than sed or awk, and that is not a style choice: sed cannot substitute a
# multi-line replacement portably (GNU accepts \n there, BSD does not), and BSD awk rejects a
# -v value containing a newline outright — `awk: newline in string`, which is exactly how the
# first cut of this failed on macOS while it would have passed on the Linux runner.
expand_env_placeholder() {  # <env-block>
  local env_block="$1" tline
  while IFS= read -r tline || [ -n "$tline" ]; do
    case "$tline" in
      # `if`, not `[ -n … ] &&`: the && form makes an empty block the function's exit status
      # whenever the placeholder is the last line it sees, which is a `set -e` failure waiting
      # for a template whose layout changes.
      *__EXTRA_ENV__*) if [ -n "$env_block" ]; then printf '%s\n' "$env_block"; fi ;;
      *)               printf '%s\n' "$tline" ;;
    esac
  done
}

render_persistence_unit() {
  local dst="$1" runtime_dir watchdog runner log_out log_err env_block tmpl mode secs
  mode="$(persistence_mode)" || return 1
  runtime_dir="$(persistence_path_dirs)"
  watchdog="$TOOLS_DIR/watchdog.sh"
  runner="$TOOLS_DIR/service-runner.sh"
  env_block="$(persistence_env_block)" || return 1
  # Distinct log names per mode: a scheduled run's output is not a watchdog's output, and one file
  # holding both would make "what happened at the last tick" unanswerable.
  if [ "$mode" = scheduled ]; then
    secs="$(schedule_seconds "$SCHEDULE")" || return 1
    log_out="/tmp/axon-$CAP-schedule.log"; log_err="/tmp/axon-$CAP-schedule.err"
  else
    log_out="/tmp/axon-$CAP-watchdog.log"; log_err="/tmp/axon-$CAP-watchdog.err"
  fi
  case "$SJEL_OS:$mode" in
    macos:watchdog)
      tmpl="$TOOLS_DIR/templates/launchd-watchdog.plist.tmpl"
      sed -e "s|__LABEL__|$UNIT_LABEL_PREFIX.$CAP|" \
          -e "s|__WATCHDOG_PATH__|$watchdog|" \
          -e "s|__PATH__|$runtime_dir:/usr/bin:/bin:/usr/sbin:/sbin|" \
          -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__LOG_OUT__|$log_out|" \
          -e "s|__LOG_ERR__|$log_err|" \
          "$tmpl"
      ;;
    macos:scheduled)
      tmpl="$TOOLS_DIR/templates/launchd-schedule.plist.tmpl"
      local launcher_line=""
      if [ "$FULL_DISK_ACCESS" = "true" ]; then
        local launcher="$SJEL_PERSONAL_ROOT/bin/axon-fda-launcher"
        if [ ! -x "$launcher" ]; then
          echo "service-runner.sh: $CAP declares full_disk_access but $launcher is not installed." >&2
          echo "  Run tools/fda-launcher/install, grant it Full Disk Access, then install-persistence again." >&2
          return 1
        fi
        launcher_line="    <string>$launcher</string>"
      fi
      sed -e "s|__LAUNCHER__|$launcher_line|" \
          -e '/^$/d' \
          -e "s|__LABEL__|$UNIT_LABEL_PREFIX.$CAP|" \
          -e "s|__RUNNER_PATH__|$runner|" \
          -e "s|__INTERVAL_SECONDS__|$secs|" \
          -e "s|__PATH__|$runtime_dir:/usr/bin:/bin:/usr/sbin:/sbin|" \
          -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__LOG_OUT__|$log_out|" \
          -e "s|__LOG_ERR__|$log_err|" \
          "$tmpl"
      ;;
    linux:watchdog)
      tmpl="$TOOLS_DIR/templates/systemd-watchdog.service.tmpl"
      sed -e "s|__WATCHDOG_PATH__|$watchdog|" \
          -e "s|__PATH__|$runtime_dir:/usr/local/bin:/usr/bin:/bin|" \
          -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__LOG_OUT__|$log_out|" \
          -e "s|__LOG_ERR__|$log_err|" \
          "$tmpl"
      ;;
    linux:scheduled)
      # The timer carries only WHEN, so it needs neither PATH nor the declared env — those belong
      # to the oneshot the timer activates (render_persistence_companion).
      tmpl="$TOOLS_DIR/templates/systemd-schedule.timer.tmpl"
      sed -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__INTERVAL_SECONDS__|$secs|" \
          "$tmpl"
      ;;
    *) return 1 ;;
  esac | expand_env_placeholder "$env_block" > "$dst"
}

# render_persistence_companion <dst> — the oneshot systemd unit a timer activates. Same purity
# contract as render_persistence_unit: no systemctl, no daemon-reload.
render_persistence_companion() {
  local dst="$1" runtime_dir env_block
  runtime_dir="$(persistence_path_dirs)"
  env_block="$(persistence_env_block)" || return 1
  sed -e "s|__RUNNER_PATH__|$TOOLS_DIR/service-runner.sh|" \
      -e "s|__PATH__|$runtime_dir:/usr/local/bin:/usr/bin:/bin|" \
      -e "s|__CAPABILITY__|$CAP|" \
      -e "s|__LOG_OUT__|/tmp/axon-$CAP-schedule.log|" \
      -e "s|__LOG_ERR__|/tmp/axon-$CAP-schedule.err|" \
      "$TOOLS_DIR/templates/systemd-schedule.service.tmpl" \
    | expand_env_placeholder "$env_block" > "$dst"
}

# persistence_state — one line: `<state>\t<detail>`. States:
#
#   n/a          persistence does not apply here (on-demand, or a natively-restarting runtime)
#   misdeclared  the manifest claims both autostart and schedule — nothing can be installed for it
#   unsupported  this OS has no backend
#   missing      it applies, and the unit is not there — the capability is down after a reboot
#   stale        the unit is there but no longer matches what the declaration renders to
#   installed    the unit matches
#
# File-level on purpose, and named as such: this compares what is written against what the
# declaration implies. Whether the supervisor has actually LOADED it is a second question, asked
# separately below, because a check that cannot be run must not be reported as one that passed.
persistence_state() {
  local why unit tmp companion
  # A manifest declaring both autostart and schedule is not "not applicable" — it is wrong, and it
  # can never be installed. Its own state rather than n/a, because n/a is what doctor passes over
  # in silence: reporting a manifest error as a green is the exact failure mode this whole state
  # machine exists to prevent.
  if ! persistence_mode >/dev/null 2>&1; then
    printf 'misdeclared\tautostart and schedule cannot both be declared (%s)\n' "$MANIFEST"
    return 0
  fi
  if ! why="$(persistence_applicable)"; then
    printf 'n/a\t%s\n' "$why"
    return 0
  fi
  if ! unit="$(persistence_unit_path)"; then
    printf 'unsupported\t%s\n' "$unit"
    return 0
  fi
  if [ ! -f "$unit" ]; then
    printf 'missing\t%s\n' "$unit"
    return 0
  fi
  tmp="$(mktemp)"
  if ! render_persistence_unit "$tmp"; then
    rm -f "$tmp"
    printf 'unsupported\tcannot render a unit for os %s\n' "$SJEL_OS"
    return 0
  fi
  if ! cmp -s "$tmp" "$unit"; then
    rm -f "$tmp"
    printf 'stale\t%s no longer matches the declaration — re-run install-persistence\n' "$unit"
    return 0
  fi
  rm -f "$tmp"
  # A scheduled systemd job is two files, and only the timer has been checked so far. A timer that
  # matches while its oneshot companion has drifted is still a broken declaration — reporting that
  # as `installed` is the same false green the whole state machine exists to prevent.
  if companion="$(persistence_companion_path)"; then
    if [ ! -f "$companion" ]; then
      printf 'missing\t%s — the timer is installed, the unit it activates is not\n' "$companion"
      return 0
    fi
    tmp="$(mktemp)"
    if ! render_persistence_companion "$tmp"; then
      rm -f "$tmp"
      printf 'unsupported\tcannot render the oneshot companion for os %s\n' "$SJEL_OS"
      return 0
    fi
    if ! cmp -s "$tmp" "$companion"; then
      rm -f "$tmp"
      printf 'stale\t%s no longer matches the declaration — re-run install-persistence\n' "$companion"
      return 0
    fi
    rm -f "$tmp"
  fi
  printf 'installed\t%s\n' "$unit"
}

# persistence_loaded — is the supervisor actually running it? Best-effort and honest: prints
# `yes`, `no`, or `unknown` when the supervisor cannot be asked on this host. An installed unit
# file that was never loaded is exactly the false green this whole issue is about, so "unknown"
# is never rendered as "yes".
persistence_loaded() {
  local hits
  case "$SJEL_OS" in
    macos)
      command -v launchctl >/dev/null 2>&1 || { echo unknown; return 0; }
      # `grep -c`, not `grep -q`, and the reason is this script's `set -o pipefail`: -q exits at
      # the first match, `launchctl list` then dies of SIGPIPE, and pipefail turns a FOUND label
      # into a failed pipeline. It reported the loaded sjel-status agent as not loaded, and
      # whether it did so depended on where in the output the label happened to sit. -c consumes
      # the whole stream, so the producer always finishes.
      hits="$(launchctl list 2>/dev/null | grep -cE "${UNIT_LABEL_PREFIX//./\\.}\.${CAP}\$" || true)"
      if [ "${hits:-0}" -gt 0 ]; then echo yes; else echo no; fi
      ;;
    linux)
      command -v systemctl >/dev/null 2>&1 || { echo unknown; return 0; }
      [ -d /run/systemd/system ] || { echo unknown; return 0; }
      # For a scheduled job the TIMER is what has to be active. Its oneshot service sits inactive
      # between ticks by design, so asking after the service would report every healthy schedule
      # as not-loaded except during the seconds it happens to be running.
      local unit_name="axon-$CAP.service"
      if [ "$(persistence_mode 2>/dev/null)" = scheduled ]; then unit_name="axon-$CAP.timer"; fi
      if systemctl --user is-active --quiet "$unit_name" 2>/dev/null; then echo yes; else echo no; fi
      ;;
    *) echo unknown ;;
  esac
}

# persistence-status — the reporting verb. One line, machine-readable first field, so doctor and
# capability.sh read the same answer this prints.
status_persistence() {
  local line state detail loaded
  line="$(persistence_state)"
  state="${line%%	*}"; detail="${line#*	}"
  case "$state" in
    installed)
      loaded="$(persistence_loaded)"
      case "$loaded" in
        yes)     printf '%s\tinstalled\t%s\n' "$CAP" "$detail" ;;
        no)      printf '%s\tinstalled-not-loaded\tthe unit exists but the supervisor is not running it: %s\n' "$CAP" "$detail" ;;
        unknown) printf '%s\tinstalled\t%s (supervisor could not be asked — load state unverified)\n' "$CAP" "$detail" ;;
      esac
      ;;
    *) printf '%s\t%s\t%s\n' "$CAP" "$state" "$detail" ;;
  esac
}

# A unit under the pre-rename label keeps its watchdog running next to the new one, so it is
# unloaded and deleted before the new unit loads, and on removal.
remove_legacy_launchd_unit() {
  local legacy="$HOME/Library/LaunchAgents/$LEGACY_UNIT_LABEL_PREFIX.$CAP.plist"
  [ -f "$legacy" ] || return 0
  launchctl unload "$legacy" 2>/dev/null || true
  rm -f "$legacy"
  echo "removed legacy unit $legacy"
}

install_persistence() {
  local why unit companion
  if ! why="$(persistence_applicable)"; then
    echo "service-runner.sh: '$CAP' — $why. Not installing persistence." >&2
    # A contradictory manifest is a declaration error, never "nothing owed". Checked before the
    # case below, which would otherwise return 0 for it purely because it happens to say
    # autostart = "true" — the natively-restarting exit, reached by a manifest that is broken.
    persistence_mode >/dev/null 2>&1 || return 1
    case "$AUTOSTART" in
      true) return 0 ;;   # a natively-restarting runtime: nothing owed, so this is not a failure
      *)
        echo "  (in $MANIFEST: autostart = \"true\" if it is meant to always run," >&2
        echo "   schedule = \"6h\" if it is meant to run periodically and exit)" >&2
        return 1
        ;;
    esac
  fi
  if ! unit="$(persistence_unit_path)"; then
    echo "service-runner.sh: $unit" >&2
    echo "For now: run '$TOOLS_DIR/watchdog.sh $CAP' manually, or add a scheduler entry here." >&2
    exit 1
  fi
  # systemd --user is the per-user analogue of a LaunchAgent. It needs a running systemd (PID 1,
  # or WSL2 with `systemd=true` in /etc/wsl.conf) and the user bus; fail with the fix rather than
  # a cryptic systemctl error where it's absent.
  if [ "$SJEL_OS" = linux ] && { ! command -v systemctl >/dev/null 2>&1 || [ ! -d /run/systemd/system ]; }; then
    echo "service-runner.sh: systemd not available (no systemctl, or systemd isn't PID 1)." >&2
    echo "  On WSL, add 'systemd=true' under [boot] in /etc/wsl.conf, then 'wsl --shutdown' and reopen." >&2
    echo "  Until then run '$TOOLS_DIR/watchdog.sh $CAP' manually (e.g. inside tmux/screen)." >&2
    exit 1
  fi
  mkdir -p "$(dirname "$unit")"
  render_persistence_unit "$unit"
  # Written before the timer is enabled, not after: systemd resolves the timer's Unit= at
  # activation, and an enable that races a missing companion fails with a message about a unit
  # nobody declared.
  if companion="$(persistence_companion_path)"; then
    render_persistence_companion "$companion"
    echo "installed $companion"
  fi
  case "$SJEL_OS" in
    macos)
      # `enable` BEFORE load, and this is not belt-and-braces.
      #
      # launchd keeps a per-user disabled set that outlives the plist. A label that was ever
      # `launchctl disable`d -- which is what `remove-persistence` and a plain `bootout` leave
      # behind -- stays in it, and every later `load` of a freshly rendered plist fails with
      # "Bootstrap failed: 5: Input/output error". The plist is correct, the file is on disk,
      # `install-persistence` prints "installed", and the job never runs.
      #
      # This was recorded as a known silent failure and left unfixed: sparpreis-watch hit it again
      # on 2026-08-29, which is the second time the same five minutes were spent on it. A tool
      # that reports success while leaving a job disabled is the failure mode this file exists to
      # refuse everywhere else.
      launchctl unload "$unit" 2>/dev/null || true
      remove_legacy_launchd_unit
      launchctl enable "gui/$(id -u)/$UNIT_LABEL_PREFIX.$CAP" 2>/dev/null || true
      launchctl load "$unit"
      echo "installed $unit"
      ;;
    linux)
      systemctl --user daemon-reload
      # basename, not a hardcoded axon-$CAP.service: for a scheduled job the TIMER is what gets
      # enabled, and enabling the oneshot instead would run it once at boot and never again.
      systemctl --user enable --now "$(basename "$unit")"
      echo "installed $unit (systemctl --user)"
      # A --user unit only runs while the user has a session unless lingering is enabled.
      # For a capability meant to survive logout / start at boot, enable it once.
      if ! loginctl show-user "$(id -un)" 2>/dev/null | stream_matches '^Linger=yes'; then
        echo "  note: run 'loginctl enable-linger $(id -un)' so this survives logout / reboot." >&2
      fi
      ;;
  esac
}

# remove-persistence — the disposition verb `disable` needs.
#
# A leftover unit is not inert: the watchdog calls `service-runner.sh start <cap>` every 30s and
# consults nothing about the enabled set, so persistence left behind by a `capability.sh disable`
# brings the capability straight back up. Removal is therefore a real operation with a real verb —
# and it is deliberately NOT something disable performs on its own, because unloading a supervisor
# unit is a machine-level side effect and capability.sh starts nothing.
remove_persistence() {
  local unit companion
  if ! unit="$(persistence_unit_path)"; then
    echo "service-runner.sh: $unit — nothing to remove" >&2
    return 0
  fi
  # The companion is resolved before the timer is deleted: persistence_companion_path answers from
  # the manifest, not from what is on disk, but reading it first keeps the two halves of one
  # removal from depending on the order they happen in.
  companion="$(persistence_companion_path || true)"
  if [ ! -f "$unit" ] && [ ! -f "${companion:-/nonexistent}" ]; then
    echo "service-runner.sh: no persistence installed for '$CAP' ($unit)"
    return 0
  fi
  case "$SJEL_OS" in
    macos)
      launchctl unload "$unit" 2>/dev/null || true
      remove_legacy_launchd_unit
      ;;
    linux)
      systemctl --user disable --now "$(basename "$unit")" 2>/dev/null || true
      ;;
  esac
  rm -f "$unit"
  echo "removed $unit"
  # A timer without its oneshot is inert, but leaving the oneshot behind leaves an enabled-by-hand
  # foothold that starts the capability with no schedule attached and nothing reporting it.
  if [ -n "$companion" ] && [ -f "$companion" ]; then
    rm -f "$companion"
    echo "removed $companion"
  fi
  [ "$SJEL_OS" = linux ] && { systemctl --user daemon-reload 2>/dev/null || true; }
  return 0
}

# kind = "data": a file this machine owns, with no process behind it (capabilities/store —
# the shared SQLite database nine capabilities open directly). Refused by name here rather
# than left to fall through to the container path, which would look for an image the
# manifest deliberately does not declare and report its absence as a broken capability.
#
# `persistence-status` answers instead of refusing, because it is a REPORTING verb: doctor
# reads its first field for every row, and a data unit legitimately owes no supervisor unit.
if [ "$KIND" = data ]; then
  case "$CMD" in
    persistence-status)
      printf '%s\tn/a\tkind=data — a file, not a process: nothing to supervise\n' "$CAP"
      exit 0
      ;;
    *)
      echo "service-runner.sh: '$CAP' is kind=data — it declares a file and how it is backed up, not something to $CMD." >&2
      echo "  The capabilities that read it are the processes; this manifest exists for tools/backup.sh." >&2
      exit 1
      ;;
  esac
fi

if [ "$KIND" = process ]; then process_init; fi

case "$CMD" in
  start)
    if [ "$KIND" = process ]; then start_process; else start_service; fi
    ;;
  stop)
    # Default holds: `stop <cap>` means "down, and stay down", which is what a maintenance window
    # needs. --no-hold is the same stop without that promise.
    _mode=hold; [ "$FLAG" = --no-hold ] && _mode=nohold
    if [ "$KIND" = process ]; then stop_process "$_mode"; else stop_service "$_mode"; fi
    ;;
  restart)
    # nohold on the way down: a restart is not a maintenance window, and leaving the
    # hold set would make the start half a no-op.
    if [ "$KIND" = process ]; then
      stop_process nohold; maybe_build force; start_process
    else
      stop_service nohold; start_service
    fi
    ;;
  idle-stop)
    # sjel-status's idle reaper spells `stop --no-hold` this way, and keeps its own verb because
    # it is a distinct decision with a distinct owner: an unread page is not a maintenance window,
    # so holding it would mean the next thing to ask for the panel gets a silent no-op until the
    # lock aged out. A capability with no idle_timeout in its manifest is never a target.
    if [ "$KIND" = process ]; then stop_process nohold; else stop_service nohold; fi
    ;;
  resume)
    if [ "$KIND" = process ]; then rm -f "$MAINT_LOCK"; start_process; else resume_service; fi
    ;;
  status)
    if [ "$KIND" = process ]; then status_process; else status_service; fi
    ;;
  recreate)
    if [ "$KIND" = process ]; then
      echo "service-runner.sh: 'recreate' is for container capabilities; use restart for a process" >&2
      exit 1
    fi
    recreate_service
    ;;
  install-persistence) install_persistence ;;
  remove-persistence)  remove_persistence ;;
  persistence-status)  status_persistence ;;
  *) usage ;;
esac

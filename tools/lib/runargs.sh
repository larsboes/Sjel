# tools/lib/runargs.sh — one canonical form for a container's run-time arguments, so a
# declaration and a running container can actually be compared.
#
# Why this exists: tools/service-runner.sh builds CONTAINER_ARGS from the resolved declaration and
# passes it to `run -d` only. An existing container is started by name, so EVERY argument in that
# array is frozen at creation time — a changed value parses correctly, resolves correctly, and
# never reaches the thing it describes. #11 fixed the publish set; this is the same freeze on the
# other four classes (#33), of which `--env-file` is the one that bites quietly: rotate a
# credential in the overlay, restart, and the container keeps serving the old value with nothing
# reporting it.
#
# The declared side is read from CONTAINER_ARGS itself rather than re-derived from the manifest.
# That is the point: the compared thing IS the argv a fresh container would get, so the comparison
# cannot drift away from the construction the way a second derivation does. The ports-only version
# did re-derive, and had already drifted — it emitted the declared ports even when `network_mode`
# suppressed every `-p`, which would report permanent false drift on the first capability that
# declared both. None does today; the class is gone rather than the instance.
#
# Canonical forms, one `<class> <value>` line each, sorted:
#
#     port     HOSTADDR:HOSTPORT:CONTAINERPORT/PROTO
#     mount    SOURCE:DESTINATION          (SOURCE = volume name, or absolute host path)
#     cap      NAME                        (no CAP_ prefix, upper case)
#     network  NAME                        (absent/bridge/default all mean default)
#     envfile  PATH                        (the path only — see env_diff for the contents)
#
# The env file's CONTENTS are deliberately not in that stream: they are secrets, and they are
# compared by env_diff(), which reports key names only and never writes them anywhere.
#
# bash 3.2-safe (CONTRIBUTING.md#portable-shell). jq is a declared host requirement (toolchain.toml).

# --- canonical forms -------------------------------------------------------------------

# normalize_publish <-p spec> — canonical form of one declared publish entry.
#
# An unset host address normalises to 0.0.0.0 and an absent protocol to tcp, because that is what
# every runtime here means by leaving them out. A `-p` with no host port at all (`8080`) keeps `*`
# in that field: the runtime picks a port at random, so there is no value to compare against and
# pretending otherwise would report drift on every single start.
normalize_publish() {
  local spec="${1:-}" proto=tcp body rest hostaddr hostport cport
  [ -n "$spec" ] || return 0
  case "$spec" in
    */*) proto="${spec##*/}"; body="${spec%/*}" ;;
    *)   body="$spec" ;;
  esac
  case "$body" in
    *:*:*)
      hostaddr="${body%%:*}"; rest="${body#*:}"
      hostport="${rest%%:*}"; cport="${rest##*:}"
      ;;
    *:*)
      hostaddr=""; hostport="${body%%:*}"; cport="${body##*:}"
      ;;
    *)
      hostaddr=""; hostport=""; cport="$body"
      ;;
  esac
  printf '%s:%s:%s/%s\n' "${hostaddr:-0.0.0.0}" "${hostport:-*}" "$cport" "${proto:-tcp}"
}

# normalize_cap <name> — bare upper-case capability name.
# The manifest writes docker's canonical bare form (NET_ADMIN); a runtime may echo back the kernel
# spelling (CAP_NET_ADMIN). Both name the same bit, so neither may look like drift.
normalize_cap() {
  local c="${1:-}"
  [ -n "$c" ] || return 0
  c="$(printf '%s' "$c" | tr 'a-z' 'A-Z')"
  printf '%s\n' "${c#CAP_}"
}

# normalize_network <name> — the network a container is on.
# Absent, "default" and "bridge" are one state: the runtime's own default network. Axon manifests
# only ever declare `host` or nothing (schemas/service.toml.example), so collapsing them cannot
# hide a declared change — it only stops docker's two spellings of its own bridge from reading as
# a difference between two identically configured containers.
normalize_network() {
  case "${1:-}" in
    ""|default|bridge) printf 'default\n' ;;
    *)                 printf '%s\n' "$1" ;;
  esac
}

# --- the declared side -----------------------------------------------------------------

# declared_runspec <container args...> — the canonical stream for the argv a fresh container would
# be created with. Takes the array as parameters rather than reading the caller's global, so a
# test can hand it a synthetic argv.
declared_runspec() {
  local flag val a has_network=0
  # An argv with no --network is on the default network, and the running side always reports one.
  # Saying nothing here would make every default-network container look like it had gained a
  # network it never declared.
  for a in "$@"; do
    [ "$a" = "--network" ] && { has_network=1; break; }
  done
  [ "$has_network" -eq 1 ] || printf 'network default\n'

  while [ $# -gt 0 ]; do
    flag="$1"
    case "$flag" in
      -p|-v|--cap-add|--network|--env-file)
        shift
        [ $# -gt 0 ] || return 0
        val="$1"
        case "$flag" in
          -p)         normalize_publish "$val" | sed 's/^/port /' ;;
          -v)         printf 'mount %s\n' "$val" ;;
          --cap-add)  normalize_cap "$val" | sed 's/^/cap /' ;;
          --network)  normalize_network "$val" | sed 's/^/network /' ;;
          --env-file) printf 'envfile %s\n' "$val" ;;
        esac
        ;;
    esac
    shift
  done
}

# --- the running side ------------------------------------------------------------------

# runspec_from_docker <stdin: `docker inspect <name> --format '{{json .}}'`>
#
# Mount source is the volume NAME for a named volume and the host path for a bind, which is
# exactly what the declaration writes on the left of the colon. tmpfs mounts are dropped: no Axon
# manifest can declare one (schemas/service.toml.example has no syntax for it), so anything the
# runtime adds itself would otherwise be reported as an undeclared mount on every container.
runspec_from_docker() {
  jq -r '
    (.HostConfig.PortBindings // {} | to_entries[]
      | (.key | split("/")) as $cp
      | .value[]?
      | "port \(if (.HostIp // "") == "" then "0.0.0.0" else .HostIp end):\(.HostPort):\($cp[0])/\($cp[1] // "tcp")"),
    (.Mounts[]? | select(.Type != "tmpfs")
      | "mount \(if .Type == "volume" then .Name else .Source end):\(.Destination)"),
    (.HostConfig.CapAdd[]? | "cap \(ascii_upcase | sub("^CAP_"; ""))"),
    ("network \(.HostConfig.NetworkMode // "" | if . == "" or . == "bridge" or . == "default" then "default" else . end)")
  ' 2>/dev/null | sort
}

# runspec_from_apple() and env_from_apple() stood here until 2026-09-02 (Q75). They read
# apple-container's own inventory JSON, which reported every container at once and carried a
# mount's kind as the single key of a `type` object. No machine runs that runtime now, and a
# reader for a JSON shape nothing produces is a second answer waiting to diverge from the first.

# env_from_docker <stdin: the same JSON as above> — KEY=VALUE, one per line.
#
# These carry secret VALUES. Nothing may print, log or redirect their output to a file; it exists
# only to be piped into env_diff(), which compares in memory and reports key names.
env_from_docker() { jq -r '.Config.Env[]?' 2>/dev/null; }

# --- comparison ------------------------------------------------------------------------

# runspec_diff <declared-file> <running-file> <class> — one line per difference in that class,
# exit 1 when they differ at all. Both files hold canonical `<class> <value>` lines.
runspec_diff() {
  local declared="$1" running="$2" class="$3" d r only_declared only_running rc=0
  d="$(mktemp)"; r="$(mktemp)"
  grep "^$class " "$declared" 2>/dev/null | sed "s/^$class //" | sort > "$d"
  grep "^$class " "$running"  2>/dev/null | sed "s/^$class //" | sort > "$r"
  only_declared="$(comm -23 "$d" "$r")"
  only_running="$(comm -13 "$d" "$r")"
  rm -f "$d" "$r"
  [ -n "$only_declared$only_running" ] || return 0
  [ -n "$only_declared" ] && printf '%s\n' "$only_declared" | sed 's/^/    declared, not in container: /'
  [ -n "$only_running" ]  && printf '%s\n' "$only_running"  | sed 's/^/    in container, not declared: /'
  return 1
}

# env_diff <env-file> <stdin: the container's KEY=VALUE lines> — exit 1 when they differ.
#
# Key names only, never a value. A key name is already public — it is in the manifest schema and
# the overlay's example env files — while its value is the credential the rotation was about.
# Digesting the values instead would leave a comparable artifact for a low-entropy secret, so they
# are compared in memory and never leave this function.
#
# No here-string, no here-doc, no temp file for the running side: bash 3.2 backs both `<<<` and
# `<<EOF` with a real file, which would write the container's whole environment to disk to answer
# a question about whether it changed. The walk below is parameter expansion for that reason.
#
# Only keys the FILE declares are compared. A container's environment also carries whatever its
# image baked in (PATH, LANG, the image's own defaults), and none of that is drift.
env_diff() {
  local envfile="${1:?env_diff needs an env file}" running line key want rest r rc=0 found
  running="$(cat)"
  if [ ! -f "$envfile" ]; then
    printf '    env file is missing: %s\n' "$envfile"
    return 1
  fi
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ''|'#'*) continue ;;
    esac
    if [ "${line#*=}" = "$line" ]; then
      # `KEY` with no `=` means "take it from the host environment at creation time". There is no
      # declared value to compare, and the host's current one is not necessarily what the
      # container was given — so this is named as uncheckable rather than skipped.
      printf '    cannot check (value comes from the host environment): %s\n' "$line"
      rc=1
      continue
    fi
    key="${line%%=*}"; want="${line#*=}"
    found=0; rest="$running"
    while [ -n "$rest" ]; do
      r="${rest%%$'\n'*}"
      if [ "$r" = "$rest" ]; then rest=""; else rest="${rest#*$'\n'}"; fi
      case "$r" in
        "$key="*)
          found=1
          [ "${r#*=}" = "$want" ] || { printf '    value differs: %s\n' "$key"; rc=1; }
          break
          ;;
      esac
    done
    if [ "$found" -eq 0 ]; then
      printf '    declared, not in container: %s\n' "$key"
      rc=1
    fi
  done < "$envfile"
  return $rc
}

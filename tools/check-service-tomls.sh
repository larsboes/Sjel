#!/bin/bash
# check-service-tomls.sh — the service.toml schema gate (CI: repo gates).
# Asserts every service.toml declares the load-bearing fields its kind needs. A container's
# `tag` must be present and non-empty, and it may be a rolling channel: "latest" and "stable"
# were refused until 2026-09-02 and are now the preferred form (Q77,
# CONTRIBUTING.md#patch-first). The gate that refused them was defending reproducibility a tag never
# provided — publishers rebuild under the same literal — so what it actually bought was delay.
# The digest of the running container is the version fact (ISA.md C4). Pure file-based: it
# reads the tracked manifests and nothing else — no git, no network, no live service.
#
# Three kinds, three field sets (schemas/service.toml.example):
#   container (the default when `kind` is absent) -> name, image, tag
#   process                                       -> name, command, port; no image/tag
#   data                                          -> name + a backup contract; nothing runnable
# A manifest carrying fields from more than one kind is rejected rather than resolved by
# precedence: "which half is real" must never be a guess at start time. The same principle
# covers `autostart` + `schedule`, which is the kind-independent version of the same
# contradiction — a service and a periodic job are opposite claims about one process.
#
# Spine manifests (a service.toml at the repo root, today only dashboard/) are checked
# by the same rules — the runner reads them through the same interpreter.
set -e

# paths.sh sources toml.sh and exports SJEL_ROOT — both are all we need here.
_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
# The one `schedule` parser, shared with tools/service-runner.sh — a gate that accepted a spec the
# runner then refused would be worse than no gate at all. Sourced BEFORE paths.sh, which unsets
# `_lib` on its way out (tools/lib/paths.sh:166) and would leave this reaching for "/schedule.sh".
source "$_lib/schedule.sh"
source "$_lib/paths.sh"

fail=0
found=0
for svc in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue          # empty glob → literal path, skip it
  found=1
  cap="$(basename "$(dirname "$svc")")"

  kind="$(toml_get kind "$svc")"
  [ -n "$kind" ] || kind="container"

  case "$kind" in
    container)
      for key in name image tag; do
        if [ -z "$(toml_get "$key" "$svc")" ]; then
          echo "FAIL [$cap]: missing or empty required field '$key' — $svc" >&2
          fail=1
        fi
      done

      ;;
    process)
      if [ -z "$(toml_get name "$svc")" ]; then
        echo "FAIL [$cap]: missing or empty required field 'name' — $svc" >&2
        fail=1
      fi
      if [ -z "$(toml_array command "$svc")" ]; then
        echo "FAIL [$cap]: kind=process needs a non-empty command = [...] — $svc" >&2
        fail=1
      fi
      # A port is what the registry, the dev-server proxy and the health poll all read, so a
      # long-running process without one is invisible to every consumer it has. A SCHEDULED
      # process is the opposite case, and the exception: it runs, does its work and exits, so
      # there is nothing to reach, and a port would put an entry in the proxy table aimed at a
      # process that is not running. Stated as a pair — required without `schedule`, refused
      # with it — because making it merely optional leaves both mistakes expressible.
      #
      # Found by writing the first `schedule` consumer. The field landed in c3458ee with no
      # manifest declaring it, and this gate would have rejected the first one under a rule
      # written before jobs existed. A feature with no consumer has an untested edge by
      # construction; this was it.
      if [ -n "$(toml_get schedule "$svc")" ]; then
        if [ -n "$(toml_get port "$svc")" ]; then
          echo "FAIL [$cap]: declares schedule AND port — a job runs and exits, so nothing can reach that port — $svc" >&2
          fail=1
        fi
      elif [ -z "$(toml_get port "$svc")" ]; then
        echo "FAIL [$cap]: kind=process needs a port — the registry, the proxy and the health poll all read it — $svc" >&2
        fail=1
      fi
      for key in image tag volumes env_file; do
        if [ -n "$(toml_get "$key" "$svc")" ] || [ -n "$(toml_array "$key" "$svc")" ]; then
          echo "FAIL [$cap]: kind=process must not declare container field '$key' — $svc" >&2
          fail=1
        fi
      done
      ;;
    data)
      # State with no process: a file this machine owns and backs up, and nothing to run.
      # The one that exists is capabilities/store (the shared SQLite database, PRD Q45).
      #
      # A backup contract is REQUIRED, not optional. A container or a process is worth
      # declaring for its own sake; a manifest that neither runs anything nor says how its
      # data is kept declares nothing at all, and would sit in the registry as a name with
      # no consequence. Both fields, because either alone is inert: backup.sh refuses a run
      # with no `backup_target`, and a target with no source has nothing to ship.
      if [ -z "$(toml_get name "$svc")" ]; then
        echo "FAIL [$cap]: missing or empty required field 'name' — $svc" >&2
        fail=1
      fi
      if [ -z "$(toml_get backup_sqlite_online "$svc")" ] && [ -z "$(toml_get backup_sqlite "$svc")" ] \
         && [ -z "$(toml_array backup_paths "$svc")" ]; then
        echo "FAIL [$cap]: kind=data needs a backup source (backup_paths, backup_sqlite or backup_sqlite_online) — a manifest that runs nothing and keeps nothing declares nothing — $svc" >&2
        fail=1
      fi
      if [ -z "$(toml_get backup_target "$svc")" ]; then
        echo "FAIL [$cap]: kind=data needs a backup_target — $svc" >&2
        fail=1
      fi
      # Everything a lifecycle verb would read. The runner refuses kind=data by name, so a
      # manifest carrying these would describe a service nothing will ever start.
      for key in image tag volumes env_file command port autostart schedule; do
        if [ -n "$(toml_get "$key" "$svc")" ] || [ -n "$(toml_array "$key" "$svc")" ]; then
          echo "FAIL [$cap]: kind=data must not declare runnable field '$key' — nothing starts it — $svc" >&2
          fail=1
        fi
      done
      ;;
    *)
      echo "FAIL [$cap]: unknown kind '$kind' (expected container, process or data) — $svc" >&2
      fail=1
      ;;
  esac

  # Scheduling, which applies to either kind. Same rejection principle as the two field sets
  # above: a manifest claiming to be both a service and a periodic job is refused here rather
  # than resolved by whichever check happens to run first at install time.
  schedule="$(toml_get schedule "$svc")"
  if [ -n "$schedule" ]; then
    if [ "$(toml_get autostart "$svc")" = "true" ]; then
      echo "FAIL [$cap]: declares autostart AND schedule — a watchdog holds the process up continuously, leaving an interval nothing to start. Declare one — $svc" >&2
      fail=1
    fi
    # Caught here rather than at install time: an unparseable duration is a property of the file,
    # so it should fail in CI on the machine that wrote it, not on the host that deploys it.
    if ! why="$(schedule_seconds "$schedule")"; then
      echo "FAIL [$cap]: $why — $svc" >&2
      fail=1
    fi
  fi
done

if [ "$found" -eq 0 ]; then
  echo "FAIL: no capabilities/*/service.toml found — expected at least one" >&2
  exit 1
fi

# One host port, one owner. A scalar `port` (process kind) and the host side of every
# container `ports = ["[ip:]host:container"]` mapping share one namespace on the machine;
# two manifests shipping the same default is the scouting-vs-vaultwarden 8080 collision,
# which was found by starting the full enabled set instead of by a gate. Now it's a gate.
port_lines=""
for svc in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue
  cap="$(basename "$(dirname "$svc")")"
  hostports="$(toml_get port "$svc")"
  for m in $(toml_array ports "$svc"); do
    hp="${m%:*}"                      # drop the container side
    hp="${hp##*:}"                    # drop an optional bind address
    hostports="$hostports $hp"
  done
  for hp in $hostports; do
    [ -n "$hp" ] || continue
    port_lines="${port_lines}${hp} ${cap}
"
  done
done
dups="$(printf '%s' "$port_lines" | sort -n | awk '{ if ($1 == prev) print $1 " (" prevcap ", " $2 ")"; prev = $1; prevcap = $2 }')"
if [ -n "$dups" ]; then
  echo "FAIL: host port declared by more than one manifest: $dups" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "service.toml schema check FAILED." >&2
  exit 1
fi

echo "service.toml schema check passed ($(basename "$SJEL_ROOT"): containers declaring name/image/tag, services with name/command/port, data units with a backup contract and nothing runnable, scheduled jobs with no port, no container fields, host ports unique)."

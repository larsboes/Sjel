#!/bin/bash
# Move this machine's persisted state out of the pre-rename names, once.
#
# `axon` names four things on disk: the shared database, the admission locks beside it, the pack
# deployment ledger and the cached vault unlock. The code reads `sjel` names only, so this is the
# step that carries an existing deployment across. It is idempotent: a machine already moved gets
# a one-line answer and no changes.
#
# Not moved here: the keychain entry (`axon-bw-master`) and Google Calendar's exported
# `extendedProperties`, both of which `sjel-cli` and the calendar capability read under either
# name. A stored identifier that cannot be renamed in place is read, not migrated.
#
# Run it with every capability stopped (`tools/service-runner.sh down`). It refuses while a
# process holds the old database, because a rename under a live writer is how a database ends up
# half in each directory.
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=/dev/null
source "$_dir/lib/paths.sh"

DRY_RUN=0
FORCE=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    -h | --help)
      cat <<'USAGE'
Usage: tools/migrate-axon-names.sh [--dry-run] [--force]

Moves <overlay>/data/axon to <overlay>/data/sjel and renames the database inside it,
then the pack ledger under ~/.local/state and the cached vault unlock under ~/.cache.
Stop the capabilities first: tools/service-runner.sh down.

  --dry-run  print the moves and change nothing
  --force    move even while a process holds the old database
USAGE
      exit 0
      ;;
    *)
      echo "migrate-axon-names: unknown argument '$arg' (--help for usage)" >&2
      exit 2
      ;;
  esac
done

say() { printf '%s\n' "$*"; }
die() {
  echo "migrate-axon-names: $*" >&2
  exit 1
}

DATA_DIR="${SJEL_OVERLAY_ROOT:-}/data"
[ -n "${SJEL_OVERLAY_ROOT:-}" ] || die "no overlay resolved — run tools/install.sh first"

OLD_DATA="$DATA_DIR/axon"
NEW_DATA="$DATA_DIR/sjel"
STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"

lsof_capture() {  # lsof_capture <output variable> <lsof arguments...>
  local output_variable="$1" scratch status output diagnostic
  shift
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/sjel-migrate-lsof.XXXXXX")" || {
    echo "migrate-axon-names: cannot create a private directory for lsof output" >&2
    return 1
  }
  if lsof "$@" > "$scratch/out" 2> "$scratch/err"; then
    status=0
  else
    status=$?
  fi
  output="$(<"$scratch/out")"
  diagnostic="$(<"$scratch/err")"
  rm -rf "$scratch" || {
    echo "migrate-axon-names: cannot remove temporary lsof output $scratch" >&2
    return 1
  }
  if [ -n "$diagnostic" ] || [ "$status" -gt 1 ]; then
    echo "migrate-axon-names: lsof inspection failed (exit $status): ${diagnostic:-no diagnostic}" >&2
    return 1
  fi
  printf -v "$output_variable" '%s' "$output"
  LSOF_STATUS="$status"
}

verify_lsof() {
  local target="$1" probe
  command -v lsof >/dev/null 2>&1 || {
    echo "migrate-axon-names: cannot verify whether $target is open; install lsof or stop capabilities and retry" >&2
    return 1
  }
  lsof_capture probe -t -p "$$" || return 1
  [ "$LSOF_STATUS" -eq 0 ] && [ "$probe" = "$$" ] || {
    echo "migrate-axon-names: lsof did not confirm its own process while checking $target: ${probe:-no PID}" >&2
    return 1
  }
}

# The shared database and the locks beside it, as the pre-rename code named them.
clean_old_locks() {
  local data_dir="${1:-$NEW_DATA}" lock
  for lock in "$data_dir"/axon-local-*.lock; do
    [ -e "$lock" ] || continue
    verify_lsof "$lock" || exit 1
    local holders status parsed
    lsof_capture holders -t +w "$lock" || die "cannot verify whether legacy lock $lock is open"
    status="$LSOF_STATUS"
    if [ -n "$holders" ]; then
      if ! parsed="$(printf '%s\n' "$holders" | awk 'NF == 1 && $1 ~ /^[0-9]+$/ { print; next } { exit 1 }')"; then
        die "lsof returned malformed PID output for $lock"
      fi
      [ -n "$parsed" ] || die "lsof returned malformed PID output for $lock"
      die "legacy lock $lock is still open by PID(s) $parsed"
    fi
    [ "$status" -eq 1 ] || die "lsof returned no PIDs while reporting success for $lock"
    if [ "$DRY_RUN" -eq 1 ]; then
      say "would remove stale lock $lock"
    else
      rm -f "$lock" || die "cannot remove stale lock $lock"
      say "data: removed a stale legacy lock"
    fi
  done
}

rename_old_backup() {
  local old="$NEW_DATA/axon.db.pre-turso-backup"
  local new="$NEW_DATA/sjel.db.pre-turso-backup"
  if [ -e "$old" ]; then
    [ ! -e "$new" ] || die "both $old and $new exist — resolve the two backups by hand"
    if [ "$DRY_RUN" -eq 1 ]; then
      say "would rename $old → $new"
    else
      mv "$old" "$new" || die "cannot rename the pre-Turso database backup"
      say "data: renamed the pre-Turso backup"
    fi
  fi
}

move_data() {
  if [ ! -d "$OLD_DATA" ]; then
    say "data: already sjel ($DATA_DIR/sjel)"
    clean_old_locks "$NEW_DATA"
    rename_old_backup
    return 0
  fi
  if [ -e "$NEW_DATA/sjel.db" ]; then
    die "both $OLD_DATA and $NEW_DATA/sjel.db exist — two deployments, not one to merge. Resolve by hand."
  fi
  for suffix in "" "-wal" "-shm"; do
    [ ! -e "$OLD_DATA/sjel.db$suffix" ] \
      || die "both $OLD_DATA/axon.db and $OLD_DATA/sjel.db$suffix exist — resolve the collision by hand"
  done
  if [ -e "$OLD_DATA/axon.db.pre-turso-backup" ] && [ -e "$OLD_DATA/sjel.db.pre-turso-backup" ]; then
    die "both pre-Turso database backups exist in $OLD_DATA — resolve the collision by hand"
  fi

  local holders table count_before
  holders="$(database_holders "$OLD_DATA")" || exit 1
  if [ -n "$holders" ] && [ "$FORCE" -eq 0 ]; then
    die "still open by $holders — stop the capabilities first (tools/service-runner.sh down)"
  fi
  clean_old_locks "$OLD_DATA"

  count_before="$(table_count "$OLD_DATA/axon.db")"

  if [ -e "$NEW_DATA" ]; then
    die "$NEW_DATA already exists — something wrote there before the migration. Move it aside and re-run."
  fi
  if [ "$DRY_RUN" -eq 1 ]; then
    say "would move $OLD_DATA → $NEW_DATA and rename axon.db → sjel.db inside it"
    return 0
  fi

  # Same volume, so this is one atomic rename of the whole directory: the database, its WAL and
  # shared-memory sidecars, the admission locks, and anything else an operator left there.
  mv "$OLD_DATA" "$NEW_DATA" || die "cannot move $OLD_DATA"

  # SQLite derives the sidecar names from the database file name, so both move together.
  for suffix in "" "-wal" "-shm"; do
    [ -e "$NEW_DATA/axon.db$suffix" ] || continue
    mv "$NEW_DATA/axon.db$suffix" "$NEW_DATA/sjel.db$suffix" || die "cannot rename axon.db$suffix"
  done
  if [ ! -e "$NEW_DATA/sjel.db" ]; then
    rollback_data
    die "no sjel.db after the move — nothing was renamed"
  fi
  table="$(table_count "$NEW_DATA/sjel.db")"
  if [ "$count_before" != "$table" ] || [ "$table" = "0" ]; then
    rollback_data
    die "table count changed ($count_before → $table) — moved back"
  fi
  rename_old_backup
  say "data: moved $OLD_DATA → $NEW_DATA ($table tables)"
}

rollback_data() {
  for suffix in "" "-wal" "-shm"; do
    [ -e "$NEW_DATA/sjel.db$suffix" ] && mv "$NEW_DATA/sjel.db$suffix" "$NEW_DATA/axon.db$suffix"
  done
  [ -d "$NEW_DATA" ] && mv "$NEW_DATA" "$OLD_DATA"
}

# Every PID holding a file under the old directory, from the kernel rather than from a guess.
database_holders() {
  verify_lsof "files under $1" || return 1
  local listing status holders parsed
  lsof_capture listing -t +w +D "$1" || return 1
  status="$LSOF_STATUS"
  if [ -n "$listing" ]; then
    if ! parsed="$(printf '%s\n' "$listing" | awk 'NF == 1 && $1 ~ /^[0-9]+$/ { print; next } { exit 1 }' | sort -u)"; then
      echo "migrate-axon-names: lsof returned malformed PID output while scanning $1: $listing" >&2
      return 1
    fi
    [ -n "$parsed" ] || {
      echo "migrate-axon-names: lsof returned malformed PID output while scanning $1: $listing" >&2
      return 1
    }
    printf '%s' "$parsed" | tr '\n' ' '
    return 0
  fi
  if [ "$status" -eq 1 ]; then
    return 0
  fi
  echo "migrate-axon-names: lsof could not complete the scan under $1 (exit $status): ${listing:-no output}" >&2
  return 1
}

table_count() {
  command -v sqlite3 >/dev/null 2>&1 || {
    echo 0
    return 0
  }
  sqlite3 "$1" "select count(*) from sqlite_master where type='table';" 2>/dev/null || echo 0
}

# `state` and `cache` hold directories rather than one file, so each child moves on its own and
# an existing destination child wins: a machine that already wrote to the new path keeps what it
# wrote.
merge_dir() {  # merge_dir <from> <to> <label>
  local from="$1" to="$2" label="$3"
  if [ ! -d "$from" ]; then
    say "$label: already sjel"
    return 0
  fi
  if [ "$DRY_RUN" -eq 1 ]; then
    say "would merge $from into $to"
    return 0
  fi
  mkdir -p "$to" || die "cannot create $to"
  local child base
  for child in "$from"/*; do
    [ -e "$child" ] || continue
    base="$(basename "$child")"
    if [ -e "$to/$base" ]; then
      say "$label: kept $to/$base, left $child in place"
      continue
    fi
    mv "$child" "$to/$base" || die "cannot move $child"
  done
  rmdir "$from" 2>/dev/null && say "$label: moved $from → $to" || say "$label: moved what was missing into $to; $from still holds entries"
}

move_data
merge_dir "$STATE_HOME/axon" "$STATE_HOME/sjel" "state"
merge_dir "$CACHE_HOME/axon" "$CACHE_HOME/sjel" "cache"

say "done. Start the capabilities again: tools/service-runner.sh up"

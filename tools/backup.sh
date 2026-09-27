#!/bin/bash
# tools/backup.sh — capability data backup, driven by the capability's own
# service.toml [backup] declaration (same "commands as data" split as
# service-runner.sh). Takes a COLD SQLite copy with the capability held down, or a LIVE
# one through sqlite3's own backup API where every reader is a host process
# (backup_sqlite vs backup_sqlite_online — see those two blocks), tars the declared
# paths, and ships a timestamped tarball to a remote host with retention. The remote is
# a systems.toml id; its private coordinates (host/ssh_user/backup_root) live in the
# overlay's systems.local.toml.
#
#   tools/backup.sh <capability>              # take + push a backup now
#   tools/backup.sh --no-prune <capability>   # additive recovery-rehearsal backup
#   tools/backup.sh --stream <capability>     # archive bytes on stdout; diagnostics on stderr
#
# Reached over ssh. With vault-only SSH keys (Bitwarden agent) the vault must be
# unlocked — approve the agent prompt when it appears. bash 3.2-safe.
set -euo pipefail

# Stream mode is a machine-initiated pull contract: stdout must contain archive bytes
# and nothing else, including output from sourced helpers. Preserve the caller's stdout
# on fd 3 before loading any helper, and route every normal message to stderr.
case "$#:${1:-}" in
  2:--stream) exec 3>&1 1>&2 ;;
esac

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"                 # SJEL_ROOT, SJEL_PERSONAL_ROOT, toml_*
source "$TOOLS_DIR/lib/platform.sh"              # SJEL_CONTAINER_RUNTIME (container-path exec)
source "$TOOLS_DIR/lib/external-ref.sh"          # capability_provider — whose data is this?
# Best-effort: make the vault SSH agent available even if not launched from an
# interactive shell (shared with init.zsh). No-op if the app isn't running.
source "$TOOLS_DIR/lib/bw-agent.sh" 2>/dev/null || true

usage() { echo "usage: backup.sh [--no-prune|--stream] <capability>" >&2; exit 1; }
NO_PRUNE=0
STREAM=0
case "$#:${1:-}" in
  1:--no-prune|1:--stream) usage ;;
  1:*) CAP="$1" ;;
  2:--no-prune) NO_PRUNE=1; CAP="$2" ;;
  2:--stream) STREAM=1; CAP="$2" ;;
  *) usage ;;
esac

MANIFEST="$SJEL_ROOT/capabilities/$CAP/service.toml"
[ -f "$MANIFEST" ] || { echo "backup.sh: no $MANIFEST" >&2; exit 1; }

# Backup authority does not travel with a reference (retired-tracker#169). A capability this
# machine consumes has its manifest here, backup contract and all, and every path in that
# contract is overlay-relative — so a run would find an empty directory where another host's
# live data is and write a valid, empty archive. A backup that succeeds while backing up
# nothing is worse than one that fails: it resets "last run" and you find out at restore time.
if [ -n "$(capability_provider "$CAP")" ]; then
  echo "backup.sh: '$CAP' is provided by another deployment — [capability.$CAP] provided_by in $SJEL_MACHINE_TOML." >&2
  echo "  Its data lives on that host, and so does the authority to back it up. Run this there." >&2
  exit 1
fi

TARGET_ID="$(toml_get backup_target "$MANIFEST")"
SQLITE_REL="$(toml_get backup_sqlite "$MANIFEST")"
# The same database, copied while it is open. Declared instead of backup_sqlite by a
# capability whose readers are host processes rather than a container: then every
# connection IS on one host, which is the condition SQLite's WAL requires and the exact
# one a virtiofs bind mount fails (the cold-copy block below carries that story).
SQLITE_ONLINE_REL="$(toml_get backup_sqlite_online "$MANIFEST")"
# The container name, read with the other manifest fields rather than inside the branch
# that uses it: a variable defined in the branch that happens to run first is a trap for
# the second.
NAME="$(toml_get name "$MANIFEST")"; NAME="${NAME:-$CAP}"
RETAIN="$(toml_get backup_retain "$MANIFEST")"; RETAIN="${RETAIN:-14}"
PATHS=()
while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$MANIFEST")
CPATHS=()
while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$MANIFEST")
# Subtrees inside a declared path that are not this capability's data. rsync --exclude
# patterns, relative to each declared path root.
#
# Needed the day the vault got a contract: `.obsidian/plugin-backups/` held 19 symlinks pointing
# out of the vault at a since-retired plugin monorepo, so every one dangled, and 4 KB of dead
# links made a 704 MB archive unrestorable — verify_archive below refuses exactly that. That
# refusal was itself broken from 2026-08-29 to 2026-09-23: `grep -q` under `pipefail` inverted it,
# so it shipped what it should have refused. See verify_archive for the measurement and
# tools/backup-archive-guard.test.sh for the regression test. An exclusion is
# the right answer rather than a louder failure, because a plugin backup is not vault content:
# nothing is protected by carrying it and it cannot be restored if it is.
EXCLUDES=()
while IFS= read -r line; do [ -n "$line" ] && EXCLUDES+=("$line"); done < <(toml_array backup_exclude "$MANIFEST")
EXCLUDE_ARGS=()
for x in ${EXCLUDES[@]+"${EXCLUDES[@]}"}; do EXCLUDE_ARGS+=(--exclude "$x"); done

# Where the declared backup_paths hang from. The overlay for every capability whose data Axon
# itself writes; a machine-local root for one whose data lives where the operator put it. The
# vault is that case — an iCloud container path is a fact about this machine, so it cannot sit
# in a tracked manifest, exactly as [[state_mount]] cannot.
#
# Only the ROOT moves. The paths stay relative and still pass safe_relative() below, because
# they are tar member names as well as sources, and an absolute member name is how an archive
# gets to write outside the directory a restore chose. tools/restore.sh therefore needs to know
# nothing about this: it compares member names, which are unchanged.
#
# Same per-machine override seam as [capability.<name>] port in tools/service-runner.sh.
SRC_ROOT="$SJEL_PERSONAL_ROOT"
SRC_ROOT_DECL="$(toml_get_in "capability.$CAP" backup_source_root "$SJEL_MACHINE_TOML")"
if [ -n "$SRC_ROOT_DECL" ]; then
  case "$SRC_ROOT_DECL" in "~/"*) SRC_ROOT_DECL="$HOME/${SRC_ROOT_DECL#\~/}" ;; esac
  case "$SRC_ROOT_DECL" in
    /*) ;;
    *) echo "backup.sh: [capability.$CAP] backup_source_root must be absolute or ~-relative: $SRC_ROOT_DECL" >&2; exit 1 ;;
  esac
  # Checked here rather than at first use. A root that does not exist yields a staging tree of
  # empty directories and an archive that is valid and useless — the failure this whole file is
  # written against.
  [ -d "$SRC_ROOT_DECL" ] || {
    echo "backup.sh: [capability.$CAP] backup_source_root is not a directory: $SRC_ROOT_DECL" >&2; exit 1; }
  SRC_ROOT="$SRC_ROOT_DECL"
fi

# These strings become source paths and tar member names. The manifest gate catches
# bad tracked declarations; the runtime check is still required because backup must not
# depend on CI having run before it touches private state.
safe_relative() {
  case "$1" in ''|/*|..|../*|*/../*) return 1 ;; *) return 0 ;; esac
}
for p in ${PATHS[@]+"${PATHS[@]}"}; do
  safe_relative "$p" || { echo "backup.sh: unsafe backup path declaration: $p" >&2; exit 1; }
done
if [ -n "$SQLITE_REL" ]; then
  safe_relative "$SQLITE_REL" || { echo "backup.sh: unsafe SQLite path declaration: $SQLITE_REL" >&2; exit 1; }
  [ "${#CPATHS[@]}" -eq 0 ] || {
    echo "backup.sh: backup_sqlite cannot be combined with container paths in one coherent snapshot" >&2
    exit 1
  }
fi
if [ -n "$SQLITE_ONLINE_REL" ]; then
  safe_relative "$SQLITE_ONLINE_REL" || { echo "backup.sh: unsafe SQLite path declaration: $SQLITE_ONLINE_REL" >&2; exit 1; }
  # The two are opposite claims about the same file — one says a run must hold the
  # capability down, the other says it must not — so a manifest declaring both is refused
  # rather than resolved by whichever branch runs first.
  [ -z "$SQLITE_REL" ] || {
    echo "backup.sh: backup_sqlite and backup_sqlite_online are contradictory; declare one" >&2
    exit 1
  }
  # `.backup '<path>'` is a dot-command argument, quoted inside the SQL string, so a single
  # quote in the staging path would end that string early. Refused rather than escaped: the
  # path comes from a tracked manifest and has no business containing one.
  case "$SQLITE_ONLINE_REL" in
    *"'"*) echo "backup.sh: SQLite path may not contain a single quote: $SQLITE_ONLINE_REL" >&2; exit 1 ;;
  esac
fi
for cp in ${CPATHS[@]+"${CPATHS[@]}"}; do
  case "$cp" in /*) ;; *) echo "backup.sh: container backup path must be absolute: $cp" >&2; exit 1 ;; esac
  safe_relative "${cp#/}" || { echo "backup.sh: unsafe container backup path declaration: $cp" >&2; exit 1; }
done

# The container runtime, resolved once — backup_container_paths needs it. Both remaining
# runtimes name their own CLI, so the manifest value IS the command.
runtime_exec() {
  case "$SJEL_CONTAINER_RUNTIME" in
    docker|podman) echo "$SJEL_CONTAINER_RUNTIME exec $NAME" ;;
    *) echo "backup.sh: unsupported container_runtime '$SJEL_CONTAINER_RUNTIME'" >&2; exit 1 ;;
  esac
}

# Where a capability's backups land is a fact about the machine, not about the
# capability: the Mac ships vaultwarden's tarballs to the Pi, and the Pi cannot ship
# them to itself (ISA anti-claim A3). Same override home as service-runner.sh's `ports`.
if [ -f "$SJEL_MACHINE_TOML" ]; then
  TARGET_OVERRIDE="$(toml_get_in "capability.$CAP" backup_target "$SJEL_MACHINE_TOML")"
  if [ -n "$TARGET_OVERRIDE" ]; then TARGET_ID="$TARGET_OVERRIDE"; fi
fi

if [ "$STREAM" -eq 0 ]; then
  [ -n "$TARGET_ID" ] || { echo "backup.sh: $CAP has no backup_target in service.toml" >&2; exit 1; }
fi
# A capability declares its data as host paths, paths read from inside the container, a
# live database copy, or a mix. capabilities/store declares only the third: its file has no
# containing directory worth tarring and no container to read it from.
[ "${#PATHS[@]}" -gt 0 ] || [ "${#CPATHS[@]}" -gt 0 ] || [ -n "$SQLITE_ONLINE_REL" ] || {
  echo "backup.sh: $CAP declares no backup_paths, backup_container_paths or backup_sqlite_online in service.toml" >&2; exit 1; }

# Resolve every local precondition before a SQLite capability is held. A missing source
# or verifier discovered after stop would create avoidable downtime, and shipping an
# unverified credential database is not an acceptable degraded mode.
for p in ${PATHS[@]+"${PATHS[@]}"}; do
  [ -e "$SRC_ROOT/$p" ] || {
    echo "backup.sh: declared backup path is missing: $SRC_ROOT/$p" >&2; exit 1; }
done
if [ "${#PATHS[@]}" -gt 0 ]; then
  command -v rsync >/dev/null 2>&1 || {
    echo "backup.sh: rsync is required to stage declared backup paths" >&2; exit 1; }
fi
if [ -n "$SQLITE_REL" ]; then
  [ -f "$SJEL_PERSONAL_ROOT/$SQLITE_REL" ] || {
    echo "backup.sh: declared SQLite database is missing: $SQLITE_REL" >&2; exit 1; }
  command -v sqlite3 >/dev/null 2>&1 || {
    echo "backup.sh: sqlite3 is required to verify a cold SQLite backup" >&2; exit 1; }
fi
# The same preconditions, and one more: here sqlite3 TAKES the copy as well as verifying
# it. A deployment that moved its database with SJEL_DB_PATH and left the manifest behind
# fails on the missing source rather than shipping an archive of nothing.
if [ -n "$SQLITE_ONLINE_REL" ]; then
  [ -f "$SJEL_PERSONAL_ROOT/$SQLITE_ONLINE_REL" ] || {
    echo "backup.sh: declared SQLite database is missing: $SQLITE_ONLINE_REL" >&2; exit 1; }
  command -v sqlite3 >/dev/null 2>&1 || {
    echo "backup.sh: sqlite3 is required to take and verify a live SQLite backup" >&2; exit 1; }
fi

# Remote coordinates are required only by push mode. Stream mode deliberately has no
# destination knowledge: the authenticated caller owns transport and encrypted storage.
if [ "$STREAM" -eq 0 ]; then
  SYS_LOCAL="$SJEL_PERSONAL_ROOT/config/systems.local.toml"
  [ -f "$SYS_LOCAL" ] || { echo "backup.sh: no $SYS_LOCAL (target coordinates)" >&2; exit 1; }
  # A destination is a KIND plus its coordinates. `ssh` is the original and stays the default,
  # so every existing target entry keeps working without being edited.
  #
  # `local` exists because a destination does not have to be a host. A directory that some other
  # process replicates -- an iCloud/Dropbox folder, a mounted external volume, a NAS share -- is
  # a destination this tool can write to and verify, and the replication is somebody else's job.
  # It is deliberately NOT called "icloud": nothing here knows or cares which service watches the
  # directory, and a name that claimed otherwise would be the first thing to go stale.
  TARGET_KIND="$(toml_get_in "$TARGET_ID" kind "$SYS_LOCAL")"; TARGET_KIND="${TARGET_KIND:-ssh}"
  case "$TARGET_KIND" in
    ssh)
      HOST="$(toml_get_in "$TARGET_ID" host "$SYS_LOCAL")"
      SSH_USER="$(toml_get_in "$TARGET_ID" ssh_user "$SYS_LOCAL")"
      REMOTE_ROOT="$(toml_get_in "$TARGET_ID" backup_root "$SYS_LOCAL")"
      { [ -n "$HOST" ] && [ -n "$SSH_USER" ] && [ -n "$REMOTE_ROOT" ]; } || {
        echo "backup.sh: target '$TARGET_ID' needs host + ssh_user + backup_root in $SYS_LOCAL" >&2; exit 1; }
      ;;
    local)
      HOST=""; SSH_USER=""
      REMOTE_ROOT="$(toml_get_in "$TARGET_ID" path "$SYS_LOCAL")"
      [ -n "$REMOTE_ROOT" ] || {
        echo "backup.sh: target '$TARGET_ID' is kind=local and needs a path in $SYS_LOCAL" >&2; exit 1; }
      case "$REMOTE_ROOT" in "~/"*) REMOTE_ROOT="$HOME/${REMOTE_ROOT#\~/}" ;; esac
      # The directory must already exist. Creating it would turn a mistyped path, or a volume that
      # failed to mount, into a new empty folder that accepts backups nobody will find again --
      # the same "succeeds while protecting nothing" failure the rest of this file is written
      # against. An external volume that is not plugged in must fail, not be reinvented.
      [ -d "$REMOTE_ROOT" ] || {
        echo "backup.sh: target '$TARGET_ID' path does not exist: $REMOTE_ROOT" >&2
        echo "  Create it, or attach the volume it lives on. backup.sh will not create a destination." >&2
        exit 1; }
      ;;
    *)
      echo "backup.sh: target '$TARGET_ID' has unknown kind '$TARGET_KIND' (expected ssh or local)" >&2; exit 1 ;;
  esac
fi

# Finish the toolchain preflight before creating staging state or acquiring a service
# hold. Conditional commands are checked only for contracts that use them.
require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "backup.sh: $1 is required for this backup contract" >&2
    exit 1
  }
}
for command_name in awk basename cat date dirname mkdir rm tar tr wc; do
  require_command "$command_name"
done
if [ "$STREAM" -eq 0 ]; then require_command ssh; fi
if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
  echo "backup.sh: shasum or sha256sum is required to record archive identity" >&2
  exit 1
fi
if [ -n "$SQLITE_REL" ]; then
  require_command cp
  require_command head
  [ -x "$TOOLS_DIR/service-runner.sh" ] || {
    echo "backup.sh: executable service-runner.sh is required for a coherent SQLite snapshot" >&2
    exit 1
  }
fi
if [ "${#CPATHS[@]}" -gt 0 ]; then
  read -r -a PREFLIGHT_EXEC <<< "$(runtime_exec)"
  require_command "${PREFLIGHT_EXEC[0]}"
fi
if [ -n "$SQLITE_ONLINE_REL" ]; then
  require_command head
fi

TS="$(date -u +%Y%m%dT%H%M%SZ)"
STAGE="$SJEL_PERSONAL_ROOT/backup/staging/$CAP"
TARBALL="$SJEL_PERSONAL_ROOT/backup/staging/$CAP-$TS.tar.gz"
REMOTE_DIR=""
if [ "$STREAM" -eq 0 ]; then REMOTE_DIR="$REMOTE_ROOT/$CAP"; fi
CAPABILITY_HELD=0

resume_capability() {
  [ "$CAPABILITY_HELD" -eq 1 ] || return 0
  if "$TOOLS_DIR/service-runner.sh" resume "$CAP" >/dev/null; then
    CAPABILITY_HELD=0
    return 0
  fi
  echo "backup.sh: CRITICAL: failed to resume '$CAP'; the maintenance hold remains active" >&2
  return 1
}

cleanup() { rm -rf "$STAGE" "$TARBALL"; }
on_exit() {
  status=$?
  trap - EXIT HUP INT TERM
  resume_failed=0
  resume_capability || resume_failed=1
  cleanup
  if declare -F close_master >/dev/null 2>&1; then close_master; fi
  if [ "$status" -eq 0 ] && [ "$resume_failed" -ne 0 ]; then status=1; fi
  exit "$status"
}
trap on_exit EXIT
interrupted() {
  signal="$1"
  trap - "$signal"
  case "$signal" in
    HUP)  exit 129 ;;
    INT)  exit 130 ;;
    TERM) exit 143 ;;
  esac
}
trap 'interrupted HUP' HUP
trap 'interrupted INT' INT
trap 'interrupted TERM' TERM

rm -rf "$STAGE"; mkdir -p "$STAGE"

stage_host_paths() {
  # Guard the expansion: under `set -u`, bash 3.2 treats "${EMPTY[@]}" as unbound.
  # A database-only capability legitimately declares zero paths.
  for p in "${PATHS[@]}"; do
    src="$SRC_ROOT/$p"
    dest="$STAGE/$p"; mkdir -p "$(dirname "$dest")"
    # A declared path may be a single FILE, not only a directory. `capabilities/finance`
    # is the case that needed it: its canonical truth is one journal directory plus two
    # individual files (config/finance.json and the reviewed-holdings snapshot), and
    # neither has a directory of its own to declare — config/ holds every other
    # capability's configuration and vaultwarden's credentials, so declaring it would
    # put all of that inside finance's contract.
    #
    # The trailing slashes below are what makes the directory form a contents copy; on
    # a file they make rsync fail with "Not a directory" (exit 23) AFTER mkdir -p has
    # already created a directory with the file's name in the staging tree. So the two
    # cases are told apart here rather than discovered there.
    if [ -f "$src" ] && [ ! -d "$src" ]; then
      rsync -a "$src" "$dest"
      continue
    fi
    rsync -a ${EXCLUDE_ARGS[@]+"${EXCLUDE_ARGS[@]}"} "$src/" "$dest/"
  done
}

echo "→ staging $CAP data"
# A SQLite capability is one stopped-state snapshot: acquire the hold BEFORE the first
# path copy, keep it through cold-copy integrity checking, then resume before compression
# or network I/O. This keeps attachments/config and their database from describing
# different moments.
if [ -n "$SQLITE_REL" ]; then
  echo "→ coherent cold snapshot: $CAP offline while declared paths and SQLite are staged"
  # Set before stop: resume is idempotent, so even a stop command that creates the hold
  # and then exits non-zero cannot strand the capability silently.
  CAPABILITY_HELD=1
  "$TOOLS_DIR/service-runner.sh" stop "$CAP"
fi
if [ "${#PATHS[@]}" -gt 0 ]; then
  stage_host_paths
fi

# Paths read from INSIDE the container, for data the host user cannot read.
#
# home-assistant runs as root and owns .storage that way — the auth store, the user
# database, core.config_entries. A host-side rsync as the invoking user hits Permission
# denied on exactly those files: rsync exits 23 and the run aborts, which is the correct
# outcome and also means the capability simply cannot be backed up from the host at all.
#
# Same reasoning the retired backup_pg_dumpall contract had: when the host cannot read the
# data correctly, read it where it lives instead. Staged as a tar rather than extracted, so
# ownership and modes survive the round trip — restoring a root-owned auth store as the
# invoking user is the kind of quiet degradation that only surfaces during a restore.
if [ "${#CPATHS[@]}" -gt 0 ]; then
  read -r -a EXEC <<< "$(runtime_exec)"
  for cp in "${CPATHS[@]}"; do
    safe="$(printf '%s' "${cp#/}" | tr '/' '_')"
    out="$STAGE/container-$safe.tar.gz"
    echo "  from container '$NAME': $cp"
    "${EXEC[@]}" tar czf - -C "$cp" . > "$out" || {
      echo "backup.sh: reading $cp from container '$NAME' failed — is it running?" >&2; exit 1; }
    [ -s "$out" ] || {
      echo "backup.sh: $cp produced an empty archive — refusing to ship it as a backup" >&2; exit 1; }
    echo "    $(wc -c < "$out" | tr -d ' ') bytes"
  done
fi

# Overwrite the raw-copied DB with a COLD copy taken while nothing has it open.
#
# The obvious version of this -- `sqlite3 <live-db> ".backup"` from the host -- is a
# trap, and it cost a live vaultwarden outage on 2026-07-25. SQLite's WAL mode
# coordinates readers and writers through a shared-memory index (the -shm file) and
# requires every connection to sit on the same host; a virtiofs bind mount between host
# and container is exactly the case SQLite excludes. The host-side open invalidated the
# container's -shm and every subsequent write came back as `disk I/O error` until the
# capability was stopped and the stale -shm/-wal cleared.
#
# So: hold the capability down, copy the file cold, resume. A clean shutdown
# checkpoints the WAL into the main DB, which is what makes a plain `cp` correct here --
# the -wal/-shm are copied too only if the runtime left them behind. The alternatives
# were weighed and rejected: no sqlite3 in the vaultwarden image (rules out an
# in-container dump), and vaultwarden's admin backup endpoint would mean provisioning an
# ADMIN_TOKEN and exposing the admin panel just to take a copy. A LIVE `.backup` from the
# host is the fourth, and it is the one that caused the outage — see backup_sqlite_online,
# which is safe for the one reason this case is not: no container in between.
# Cost of this approach: seconds of downtime per backup. Correctness beats uptime for
# the thing that holds every other credential.
if [ -n "$SQLITE_REL" ]; then
  echo "→ cold sqlite copy: $SQLITE_REL"
  db_src="$SJEL_PERSONAL_ROOT/$SQLITE_REL"
  db_dst="$STAGE/$SQLITE_REL"; mkdir -p "$(dirname "$db_dst")"
  rm -f "$db_dst" "$db_dst-wal" "$db_dst-shm"

  cp "$db_src" "$db_dst"
  # `|| true`: a missing sidecar is the NORMAL case after a clean stop (the WAL is
  # checkpointed away), and under `set -e` a false test as the loop's last command
  # would abort the whole backup.
  for side in wal shm; do
    [ -f "$db_src-$side" ] && cp "$db_src-$side" "$db_dst-$side" || true
  done

  # A cold copy is only worth shipping if it opens. Verified on the COPY, never the
  # live file -- reading the original from here is the whole bug this replaced.
  #
  # `|| true`: sqlite3 exits non-zero when the file is not a database at all, and under
  # `set -e` a failing command substitution ended the run right here, with the reason
  # captured in this variable and never printed. The run still fails; it says why now.
  ic="$(sqlite3 "$db_dst" 'pragma integrity_check;' 2>&1 | head -1 || true)"
  [ "$ic" = "ok" ] || { echo "backup.sh: staged SQLite copy failed integrity_check: ${ic:-unreadable}" >&2; exit 1; }
  echo "  integrity_check: ok"

  # Explicit resume before compression/shipment minimizes downtime. The EXIT trap keeps
  # a second, loud attempt armed until this succeeds and clears CAPABILITY_HELD.
  resume_capability || exit 1
fi

# The shared store: one live copy through sqlite3's own backup API, with every capability
# still reading the file. Not a raw `cp` (an open WAL makes that a torn snapshot) and not
# the cold copy above (holding this file down means stopping nine capabilities to read one
# file, which is an outage rather than a backup).
#
# The 2026-07-25 vaultwarden outage is the reason this is a separate field and not the
# default: opening a database from the host while a CONTAINER has it open invalidates the
# shared-memory index SQLite's WAL coordinates through, and every subsequent write comes
# back as `disk I/O error`. The condition SQLite states is that every connection sits on
# one host. A container behind virtiofs is not that; the capabilities reading this file
# are, which is what makes the same command safe here and dangerous there.
if [ -n "$SQLITE_ONLINE_REL" ]; then
  echo "→ live sqlite copy: $SQLITE_ONLINE_REL"
  db_src="$SJEL_PERSONAL_ROOT/$SQLITE_ONLINE_REL"
  db_dst="$STAGE/$SQLITE_ONLINE_REL"; mkdir -p "$(dirname "$db_dst")"
  rm -f "$db_dst" "$db_dst-wal" "$db_dst-shm"

  # `.backup` checkpoints the WAL into the copy, so no sidecar is staged and the archive
  # holds one self-contained file. It retries internally while a writer holds the lock.
  sqlite3 "$db_src" ".backup '$db_dst'" || {
    echo "backup.sh: sqlite3 .backup failed on $SQLITE_ONLINE_REL — refusing to ship an incomplete copy" >&2; exit 1; }
  [ -s "$db_dst" ] || {
    echo "backup.sh: the live copy is empty — refusing to ship it as a backup" >&2; exit 1; }

  # Verified on the COPY, never on the live file. Reading the original from here is the
  # whole bug the block above describes.
  # `|| true`: sqlite3 exits non-zero when the file is not a database at all, and under
  # `set -e` a failing command substitution ends the run with the reason captured here and
  # never printed.
  ic="$(sqlite3 "$db_dst" 'pragma integrity_check;' 2>&1 | head -1 || true)"
  [ "$ic" = "ok" ] || { echo "backup.sh: staged SQLite copy failed integrity_check: ${ic:-unreadable}" >&2; exit 1; }
  echo "  integrity_check: ok"

  # What the copy is supposed to contain, recorded now so restore can check that it came
  # back. A file that opens and passes integrity_check proves the pages are consistent; it
  # says nothing about whether the rows are there. Same claim the retired pg_dumpall
  # contract recorded, and the same reason: an empty database is internally perfect.
  SQLITE_TABLES="$(sqlite3 "$db_dst" "select count(*) from sqlite_master where type = 'table' and name not like 'sqlite_%';")"
  # One generated UNION ALL rather than a loop: bash 3.2 has no arithmetic over a stream,
  # and a per-table sqlite3 invocation would open the file once per table.
  count_sql="$(sqlite3 "$db_dst" "select coalesce(group_concat('select count(*) as c from \"' || name || '\"', ' union all '), 'select 0 as c') from sqlite_master where type = 'table' and name not like 'sqlite_%';")"
  SQLITE_ROWS="$(sqlite3 "$db_dst" "select coalesce(sum(c), 0) from ($count_sql);")"
  echo "  contents: $SQLITE_TABLES table(s), $SQLITE_ROWS row(s)"
fi

# The archive describes the historical backup contract it was made from. Restore checks
# it against the tracked service.toml before using it, so a changed contract requires
# checking out the matching Axon revision instead of guessing. All values originate in
# tracked manifests and contain no host, user, target, receipt, or private path.
write_toml_array() { # <key> [values...]
  key="$1"; shift
  printf '%s = [' "$key"
  sep=""
  for value in "$@"; do
    case "$value" in
      *'"'*|*$'\n'*)
        echo "backup.sh: backup manifest value cannot contain quotes or newlines" >&2
        exit 1
        ;;
    esac
    printf '%s"%s"' "$sep" "$value"
    sep=", "
  done
  printf ']\n'
}

IMAGE="$(toml_get image "$MANIFEST")"
TAG="$(toml_get tag "$MANIFEST")"

# What the container was ACTUALLY running when this archive was taken.
#
# `tag` above stopped identifying a build on 2026-09-02 (Q77): every declared tag is now a
# rolling channel ("stable", "latest", "alpine"), so `image:tag` names a stream that moves under
# the same string. The digest is the only version fact left (ISA.md C4), and an archive that
# does not carry it can never say which build wrote the bytes inside it.
#
# Recorded, never enforced — tools/restore.sh prints it and compares nothing. Images roll daily
# by design, so a restore refused because the running digest moved would refuse exactly the
# restore an operator reaches for.
#
# RepoDigests[0] first: that is the registry-side identity and means the same thing on another
# machine. The local image id is the fallback for an image the daemon holds no registry digest
# for. Empty stays empty and the field is then omitted — no container, no runtime CLI on PATH,
# or an inspect that failed are all "unknown", and writing an empty string would make the
# archive claim the answer is known to be nothing.
IMAGE_DIGEST=""
if [ -n "$IMAGE" ] && command -v "$SJEL_CONTAINER_RUNTIME" >/dev/null 2>&1; then
  _image_id="$("$SJEL_CONTAINER_RUNTIME" inspect "$NAME" --format '{{.Image}}' 2>/dev/null || true)"
  if [ -n "$_image_id" ]; then
    IMAGE_DIGEST="$("$SJEL_CONTAINER_RUNTIME" image inspect "$_image_id" \
      --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' 2>/dev/null || true)"
    [ -n "$IMAGE_DIGEST" ] || IMAGE_DIGEST="$_image_id"
  fi
  unset _image_id
fi

{
  printf 'format = "1"\n'
  printf 'capability = "%s"\n' "$CAP"
  printf 'created_at = "%s"\n' "$TS"
  printf 'image = "%s"\n' "$IMAGE"
  printf 'tag = "%s"\n' "$TAG"
  # Optional and additive, like sqlite_tables below: the format stays "1" and an archive taken
  # before the field existed simply does not carry it.
  if [ -n "$IMAGE_DIGEST" ]; then
    printf 'image_digest = "%s"\n' "$IMAGE_DIGEST"
  fi
  printf 'sqlite = "%s"\n' "$SQLITE_REL"
  printf 'sqlite_online = "%s"\n' "$SQLITE_ONLINE_REL"
  # Optional and additive, which is why the format stays "1": an archive taken before the
  # field existed simply does not carry it, and restore.sh checks non-emptiness instead of
  # an exact match for those. The retired pg_dumpall contract recorded the same two numbers
  # under pg_user_tables/pg_total_rows.
  if [ -n "$SQLITE_ONLINE_REL" ]; then
    printf 'sqlite_tables = "%s"\n' "$SQLITE_TABLES"
    printf 'sqlite_rows = "%s"\n' "$SQLITE_ROWS"
  fi
  write_toml_array backup_paths ${PATHS[@]+"${PATHS[@]}"}
  write_toml_array backup_container_paths ${CPATHS[@]+"${CPATHS[@]}"}
  # Recorded, not enforced. tools/restore.sh compares the path contract and deliberately not this
  # one: an exclusion narrows what an archive holds, so a restore reading it can only be told
  # something true about what is absent. Without it, "the archive has no .obsidian/plugin-backups"
  # and "the source had none" are indistinguishable at restore time, and only one is a problem.
  write_toml_array backup_exclude ${EXCLUDES[@]+"${EXCLUDES[@]}"}
} > "$STAGE/axon-backup.toml"

echo "→ tarball $CAP-$TS.tar.gz"
# COPYFILE_DISABLE stops macOS tar emitting AppleDouble ._ files / xattrs; a
# no-op on Linux tar, so the tool stays portable.
COPYFILE_DISABLE=1 tar czf "$TARBALL" -C "$STAGE" .

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "backup.sh: need shasum or sha256sum to record archive identity" >&2
    exit 1
  fi
}
LOCAL_SHA256="$(sha256_file "$TARBALL")"

# `tar czf` exiting 0 is not the same as a complete archive. //:backup_test once observed a
# streamed archive arriving without its manifest member on a Linux runner -- same workflow, same
# runner image, same tree, green on a re-run -- and the producer exited 0 both times. A backup
# that is silently short is the one outcome a backup tool must never have, so completeness is the
# producer's own claim from here rather than something a consumer discovers later.
verify_archive() {  # <path>
  local members
  if ! members="$(tar -tzf "$1" 2>/dev/null)"; then
    echo "backup.sh: $1 is not a readable gzip archive — refusing to ship it" >&2
    return 1
  fi
  case "$members" in
    *./axon-backup.toml*) ;;
    *)
      echo "backup.sh: $1 is missing ./axon-backup.toml — incomplete, refusing to ship it" >&2
      return 1
      ;;
  esac
  # The same rule tools/restore.sh applies, applied here instead of only there.
  #
  # restore.sh refuses any archive carrying a link or special file, and it is right to: a symlink
  # inside an archive is how extraction writes outside the directory the operator chose. But
  # backup.sh happily PRODUCED such archives, so the two disagreed about what a valid archive is
  # and the disagreement could only surface at restore time — which is the worst possible moment
  # and the only one where the answer matters.
  #
  # Found 2026-08-29 by rehearsing the vault's first restore: it shipped 704 MB, verified the size
  # on the target, wrote a receipt, reported success, and could never have been restored. Nothing
  # was wrong with the copy. The producer and the consumer simply did not hold the same contract.
  #
  # Found again 2026-09-23, and the first fix never worked. This guard was written as
  # `if printf ... | grep -q '^[lhbcps]'`. `-q` exits at the first match, which closes the pipe
  # while printf is still writing 13,000 lines; the writer dies on SIGPIPE, and `set -o pipefail`
  # then makes the pipeline report 141. **A match therefore evaluated as false**, so the guard
  # shipped the very archive it exists to refuse — and its only trace was
  # `printf: write error: Broken pipe` on stderr, which reads like noise. Measured on the vault
  # archive: pipeline exit 141 with pipefail, 0 without. Every archive produced since 2026-08-29
  # was unchecked.
  #
  # So the offending lines are captured first, with no `-q` and no `head` (both of which close the
  # pipe early). Nothing below branches on a pipeline's status, which is what makes it correct
  # under pipefail rather than accidentally correct without it.
  local verbose
  verbose="$(tar -tvzf "$1" 2>/dev/null || true)"
  local offending
  offending="$(printf '%s\n' "$verbose" | LC_ALL=C grep -E '^[lhbcps]' | sed -n '1,5p' || true)"
  if [ -n "$offending" ]; then
    echo "backup.sh: $1 contains a link or special file, which tools/restore.sh refuses to extract." >&2
    echo "  Declare the offending subtree in backup_exclude, or remove it from the source:" >&2
    printf '%s\n' "$offending" | sed 's/^/    /' >&2
    return 1
  fi
}
verify_archive "$TARBALL"

# Pull mode stops here. The source capability has already resumed, the archive is
# complete, and the EXIT trap removes all staging state after the caller consumes it.
# The caller must pipe stdout directly into an encrypted repository; redirecting it to
# a file would create the plaintext intermediate this mode exists to avoid.
if [ "$STREAM" -eq 1 ]; then
  echo "→ stream $CAP archive ($LOCAL_SHA256)"
  # cat's status is checked explicitly rather than left to `set -e`, so the failure carries a
  # sentence. A consumer that goes away mid-stream, or a far end that runs out of room, leaves
  # the caller holding a truncated archive; exiting 0 there is how a short backup becomes one
  # nobody notices until a restore.
  if ! cat "$TARBALL" >&3; then
    echo "backup.sh: streaming the $CAP archive failed part-way — the consumer's copy is incomplete" >&2
    exit 1
  fi
  exit 0
fi

# A local destination: same contract as the remote one, minus the network. Written to a .part
# name, byte-count verified, renamed only then, pruned to the same retention. The verification is
# not ceremony here either -- a full disk, a volume that unmounted mid-write and a directory that
# is really a sync placeholder all produce a short file and no error.
if [ "$TARGET_KIND" = "local" ]; then
  LOCAL_DIR="$REMOTE_ROOT/$CAP"
  echo "→ ship → $LOCAL_DIR/"
  mkdir -p "$LOCAL_DIR"
  DEST_NAME="$(basename "$TARBALL")"
  DEST_PART="$LOCAL_DIR/.$DEST_NAME.part"
  cp "$TARBALL" "$DEST_PART"
  LOCAL_BYTES="$(wc -c < "$TARBALL" | tr -d ' ')"
  DEST_BYTES="$(wc -c < "$DEST_PART" | tr -d ' ')"
  if [ "$LOCAL_BYTES" != "$DEST_BYTES" ]; then
    rm -f "$DEST_PART"
    echo "backup.sh: short write — sent $LOCAL_BYTES bytes, destination holds $DEST_BYTES. Nothing was kept." >&2
    exit 1
  fi
  mv "$DEST_PART" "$LOCAL_DIR/$DEST_NAME"
  echo "  wrote $LOCAL_BYTES bytes, size verified at the destination"

  RETENTION_APPLIED=true
  if [ "$NO_PRUNE" -eq 1 ]; then
    RETENTION_APPLIED=false
    echo "→ retain every prior archive (--no-prune)"
  else
    echo "→ prune destination to last $RETAIN"
    # A listing that fails is said, not swallowed. Under launchd without Full Disk Access the
    # glob matched nothing, `ls` failed, and pipefail ended the run here with no message, after
    # the archive had been written (2026-09-25).
    if ! listing="$(ls -1t "$LOCAL_DIR/$CAP-"*.tar.gz 2>&1)"; then
      echo "backup.sh: cannot list $LOCAL_DIR to prune it: $listing" >&2
      echo "  On macOS a scheduled run needs full_disk_access = true in the manifest." >&2
      exit 1
    fi
    printf '%s\n' "$listing" | tail -n +$((RETAIN + 1)) | while IFS= read -r old; do rm -f "$old"; done
  fi
  # A local destination can be a directory something else replicates, and replication can also
  # take files AWAY. macOS with `com.apple.bird optimize-storage = 1` evicts the contents of an
  # iCloud folder under disk pressure and leaves a `.<name>.icloud` placeholder: the archive is
  # still listed, still named, and no longer here. Restoring it would need the network and a full
  # download, at exactly the moment that is least affordable.
  #
  # macOS offers no CLI to pin a folder — `brctl` on macOS 26 exposes log/status/quota/monitor and
  # nothing that downloads or holds. "Keep Downloaded" is a Finder action. So the pin cannot be
  # asserted here; only its absence can be detected, which is the more useful half anyway. This is
  # the same lesson the vault taught: a declaration nobody verifies is how three separate backups
  # were dead for weeks without anyone noticing.
  # Prevention first, where it can be checked. macOS records Finder's "Keep Downloaded" as an
  # extended attribute on the directory — `com.apple.fileprovider.pinned#PX`, where the `#PX` is
  # part of the name and not a display artifact. There is no CLI to SET it (brctl on macOS 26
  # exposes log, status, quota and monitor and nothing that downloads or holds), but reading it
  # turns "please remember to pin this" from an instruction nobody verifies into a fact this tool
  # checks on every run. That distinction is the whole of D10.
  #
  # Only meaningful for a directory a file provider actually manages, so its absence is a warning
  # rather than a failure: a plain external volume has no such attribute and needs none.
  case "$REMOTE_ROOT" in
    "$HOME/Library/Mobile Documents/"*)
      if ! xattr "$REMOTE_ROOT" 2>/dev/null | grep -q "^com\.apple\.fileprovider\.pinned"; then
        echo "  WARNING: $REMOTE_ROOT is not pinned — macOS may evict these archives." >&2
        echo "  In Finder, right-click it and choose \"Keep Downloaded\"." >&2
      fi
      ;;
  esac

  # An evicted archive is one that a listing still shows and the disk no longer holds.
  #
  # This looked only for `.<name>.icloud` placeholder files, and on this machine it had never
  # found one and never could: CloudDocs stopped writing that form when it moved onto the File
  # Provider. What it writes now is the SAME name with the kernel's SF_DATALESS flag set and
  # zero blocks allocated — `ls -lO` prints `dataless`, `find -flags +dataless` lists it, and a
  # search for `.*.icloud` cannot see it at all. Measured against the live target 2026-09-08:
  # three of four archives (finance, store, knowledge-base) were dataless and this check
  # answered 0, which is the same silent-green shape D10 is about.
  #
  # Both forms are counted. The `.icloud` one is still what a legacy provider and a
  # non-CloudDocs one write, so removing it would trade one blind spot for another.
  #
  # `-flags` is BSD find only, and eviction is a macOS question, so the dataless half runs on
  # Darwin and its ABSENCE there is reported rather than counted as zero. A detector that
  # cannot see the thing it detects must not answer "none".
  EVICTED="$(find "$LOCAL_DIR" -name ".*.icloud" 2>/dev/null | wc -l | tr -d " ")"
  if [ "$(uname -s)" = "Darwin" ]; then
    if find "$LOCAL_DIR" -maxdepth 0 -flags +dataless >/dev/null 2>&1; then
      DATALESS="$(find "$LOCAL_DIR" -type f -flags +dataless 2>/dev/null | wc -l | tr -d " ")"
      EVICTED=$((EVICTED + DATALESS))
    else
      echo "  WARNING: this find does not understand -flags, so an evicted archive cannot be seen." >&2
      echo "  Check the destination by hand: ls -lO $LOCAL_DIR" >&2
    fi
  fi
  if [ "$EVICTED" != "0" ]; then
    echo "  WARNING: $EVICTED archive(s) at this destination are evicted placeholders, not files." >&2
    echo "  A restore from them needs network and a full download. In Finder, right-click" >&2
    echo "  $REMOTE_ROOT and choose \"Keep Downloaded\"." >&2
  fi

  REMOTE_DIR="$LOCAL_DIR"
  SHIP_DESCRIPTION="$LOCAL_DIR"
else

# One shared ssh connection for every remote op (mkdir + ship + verify + prune) via
# ControlMaster — so the Bitwarden agent asks for approval ONCE, not per-command.
CTRL="/tmp/axon-backup-$CAP-$$.sock"
SSHM=(-o ControlMaster=auto -o "ControlPath=$CTRL" -o ControlPersist=30)
close_master() { ssh -O exit -o "ControlPath=$CTRL" "$SSH_USER@$HOST" 2>/dev/null || true; }

echo "→ ship → $SSH_USER@$HOST:$REMOTE_DIR/  (approve the Bitwarden agent prompt — ONCE)"
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "mkdir -p '$REMOTE_DIR'"

# Shipped by streaming into `cat`, not with rsync or scp, and that is deliberate.
# A backup target is the one machine you want to grant as little as possible, and a
# hardened one grants exactly a shell. Synology DSM refuses `rsync --server` and the
# sftp subsystem to any account outside the administrators group -- key auth succeeds,
# then the exec request is denied (measured on a real backup target). The choices were
# to make the backup account an administrator or to stop needing a remote binary; the
# second one is not a workaround, because rsync buys nothing here anyway. Every run ships
# exactly one new tarball, so there is no delta to compute and nothing to resume.
#
# Written to a .part name and renamed only after the byte count matches. A truncated
# transfer that lands under the final name is worse than a failed one: it looks like a
# backup, and you find out otherwise on the day you need it.
REMOTE_NAME="$(basename "$TARBALL")"
REMOTE_PART="$REMOTE_DIR/.$REMOTE_NAME.part"
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "cat > '$REMOTE_PART'" < "$TARBALL"

LOCAL_BYTES="$(wc -c < "$TARBALL" | tr -d ' ')"
REMOTE_BYTES="$(ssh "${SSHM[@]}" "$SSH_USER@$HOST" "wc -c < '$REMOTE_PART'" | tr -d ' ')"
if [ "$LOCAL_BYTES" != "$REMOTE_BYTES" ]; then
  ssh "${SSHM[@]}" "$SSH_USER@$HOST" "rm -f '$REMOTE_PART'"
  echo "backup.sh: short write — sent $LOCAL_BYTES bytes, target holds $REMOTE_BYTES. Nothing was kept." >&2
  exit 1
fi
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "mv '$REMOTE_PART' '$REMOTE_DIR/$REMOTE_NAME'"
echo "  shipped $LOCAL_BYTES bytes, size verified on the target"

RETENTION_APPLIED=true
if [ "$NO_PRUNE" -eq 1 ]; then
  RETENTION_APPLIED=false
  echo "→ retain every prior archive (--no-prune)"
else
  echo "→ prune remote to last $RETAIN"
  ssh "${SSHM[@]}" "$SSH_USER@$HOST" "ls -1t '$REMOTE_DIR'/$CAP-*.tar.gz 2>/dev/null | tail -n +$((RETAIN + 1)) | xargs -r rm -f"
fi
SHIP_DESCRIPTION="$SSH_USER@$HOST:$REMOTE_DIR"
fi  # end destination-kind branch

# Written only here, after the remote byte count matched and the .part was moved into
# place. A receipt that exists therefore means a backup landed, not that a run started
# -- which is the whole point of having one: cleanup() deletes the staging dir and the
# tarball, so without this nothing local can answer "when was the last successful
# backup" and reading the answer off the target costs an ssh round trip plus an
# unlocked vault agent. Instance state, so it lives in the overlay, not in Axon.
# Deliberately NOT backup/local/.last-backup-timestamp: that path belongs to a retired
# ad-hoc script and reusing it would inherit its meaning.
# Hand-built JSON rather than jq: bash 3.2 everywhere, no new dependency for eight fields
# whose values are all either numeric or shell-controlled.
RECEIPT_DIR="$SJEL_PERSONAL_ROOT/backup/receipts"
mkdir -p "$RECEIPT_DIR"
contents=""
[ "${#PATHS[@]}" -gt 0 ] && contents="paths"
[ "${#CPATHS[@]}" -gt 0 ] && contents="${contents:+$contents+}container_paths"
[ -n "$SQLITE_ONLINE_REL" ] && contents="${contents:+$contents+}sqlite_online"
cat > "$RECEIPT_DIR/$CAP.json" <<RECEIPT
{
  "capability": "$CAP",
  "completed_at": "$TS",
  "target": "$TARGET_ID",
  "tarball": "$CAP-$TS.tar.gz",
  "bytes": $LOCAL_BYTES,
  "sha256": "$LOCAL_SHA256",
  "contents": "$contents",
  "retention_applied": $RETENTION_APPLIED
}
RECEIPT
echo "  receipt → backup/receipts/$CAP.json"

echo "✓ backup complete: $CAP-$TS.tar.gz → $SHIP_DESCRIPTION/"

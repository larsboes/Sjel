#!/bin/bash
# Tests for tools/migrate-axon-names.sh.
#
# The migration runs once, on a machine holding the operator's real database, so the cases that
# matter are the refusals: a live writer, two directories that would be merged, and a database
# whose table count changes across the move. Each runs here against a scratch overlay and HOME.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
pass() { printf '  ✓ %s\n' "$*"; }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/migrate-axon-names.sh" ]; then TOOLS="$_c"; break; fi
done
[ -n "$TOOLS" ] || { echo "migrate-axon-names: cannot find the script next to $_dir" >&2; exit 1; }

command -v sqlite3 >/dev/null 2>&1 || { echo "migrate-axon-names: sqlite3 not installed"; exit 1; }

SCRATCH="$(mktemp -d "/tmp/sjel-migrate-axon.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

setup() {  # setup <name> -> prints the home directory
  local home="$SCRATCH/$1"
  mkdir -p "$home/overlay/data/axon" "$home/.local/state/axon/pack-deployments" \
    "$home/.cache/axon"
  sqlite3 "$home/overlay/data/axon/axon.db" \
    "create table a (id integer primary key, body text); create table b (id integer); insert into a (body) values ('kept');"
  printf 'write-ahead log\n' > "$home/overlay/data/axon/axon.db-wal"
  printf 'lock\n' > "$home/overlay/data/axon/axon-local-abc.lock"
  printf 'old database backup\n' > "$home/overlay/data/axon/axon.db.pre-turso-backup"
  printf '{"harness":"claude"}\n' > "$home/.local/state/axon/pack-deployments/claude.json"
  printf 'session\n' > "$home/.cache/axon/bw-session"
  printf '%s\n' "$home"
}

run() {  # run <home> [args...]
  local home="$1"
  shift
  HOME="$home" SJEL_OVERLAY_ROOT="$home/overlay" XDG_STATE_HOME="$home/.local/state" \
    XDG_CACHE_HOME="$home/.cache" bash "$TOOLS/migrate-axon-names.sh" "$@"
}

run_with_path() {  # run_with_path <home> <PATH> [args...]
  local home="$1" path="$2"
  shift 2
  PATH="$path" HOME="$home" SJEL_OVERLAY_ROOT="$home/overlay" XDG_STATE_HOME="$home/.local/state" \
    XDG_CACHE_HOME="$home/.cache" bash "$TOOLS/migrate-axon-names.sh" "$@"
}

# --- the move --------------------------------------------------------------

home="$(setup move)"
out="$(run "$home" 2>&1)" || fail "a clean migration exited non-zero: $out"
[ -e "$home/overlay/data/sjel/sjel.db" ] || fail "the database was not renamed to sjel.db"
[ -e "$home/overlay/data/axon" ] && fail "the old data directory is still there"
[ "$(sqlite3 "$home/overlay/data/sjel/sjel.db" "select body from a;")" = "kept" ] \
  || fail "the moved database lost its rows"
[ -e "$home/overlay/data/sjel/sjel.db-wal" ] || fail "the WAL sidecar did not follow the database"
[ -e "$home/overlay/data/sjel/sjel.db.pre-turso-backup" ] || fail "the historical backup was not renamed"
[ -e "$home/overlay/data/sjel/axon.db.pre-turso-backup" ] && fail "the historical backup kept the old name"
[ -e "$home/overlay/data/sjel/axon-local-abc.lock" ] && fail "a stale admission lock was carried across"
[ -e "$home/.local/state/sjel/pack-deployments/claude.json" ] || fail "the pack ledger did not move"
[ -e "$home/.cache/sjel/bw-session" ] || fail "the cached unlock did not move"
pass "a clean migration moves the database, its sidecars, the ledger and the cache"

# --- idempotence -----------------------------------------------------------

out="$(run "$home" 2>&1)" || fail "a second run exited non-zero: $out"
case "$out" in
  *"already sjel"*) ;;
  *) fail "a second run did not report the machine as already moved: $out" ;;
esac
[ -e "$home/overlay/data/sjel/sjel.db" ] || fail "a second run removed the database"
pass "a machine already moved is reported and left alone"

# --- a live writer ---------------------------------------------------------

home="$(setup busy)"
# A real holder: this shell's own sleep holds the file open through `lsof -D`.
sqlite3 "$home/overlay/data/axon/axon.db" "select 1" >/dev/null
(
  exec 9>"$home/overlay/data/axon/axon.db"
  sleep 5 9>&-
) &
holder_pid=$!
sleep 0.2
out="$(run "$home" 2>&1)" && fail "a migration with the database open exited zero"
case "$out" in
  *"still open by"* | *"lsof could not complete the scan"*) ;;
  *) fail "a live writer was not named or conservatively refused: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] || fail "a refused migration moved the database anyway"
wait "$holder_pid" 2>/dev/null
pass "a process holding the database is named and the migration refuses"

home="$(setup force-live-lock)"
(
  exec 9>"$home/overlay/data/axon/axon-local-abc.lock"
  sleep 5 9>&-
) &
holder_pid=$!
sleep 0.2
out="$(run "$home" --force 2>&1)" && fail "--force moved data after it bypassed the open-database check"
case "$out" in
  *"legacy lock"*"still open by"*) ;;
  *) fail "an open lock was not named before migration: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] || fail "the database moved while its lock was open"
[ -e "$home/overlay/data/axon/axon-local-abc.lock" ] || fail "the open lock was deleted"
wait "$holder_pid" 2>/dev/null
pass "--force cannot move or unlink an open admission lock"

home="$(setup uninspectable)"
mkdir -p "$home/lsof-bin"
printf '#!/bin/sh\nexit 2\n' > "$home/lsof-bin/lsof"
chmod +x "$home/lsof-bin/lsof"
out="$(run_with_path "$home" "$home/lsof-bin:$PATH" 2>&1)" \
  && fail "a migration proceeded when lsof failed"
case "$out" in
  *"lsof inspection failed"*) ;;
  *) fail "an lsof error did not stop the database move: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] \
  || fail "the database moved after lsof failed"
pass "the database move stops when holder inspection fails"

home="$(setup lsof-query-error)"
mkdir -p "$home/lsof-bin"
cat > "$home/lsof-bin/lsof" <<'STUB'
#!/bin/sh
if [ "$1" = "-t" ] && [ "$2" = "-p" ]; then
  printf '%s\n' "$3"
  exit 0
fi
printf 'lsof: WARNING: injected inspection failure\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n'
exit 2
STUB
chmod +x "$home/lsof-bin/lsof"
out="$(run_with_path "$home" "$home/lsof-bin:$PATH" --dry-run --force 2>&1)" \
  && fail "--force bypassed an lsof directory-query error"
case "$out" in
  *"lsof inspection failed"*) ;;
  *) fail "a directory-query error was not reported: $out" ;;
esac
case "$out" in
  *"would move"*) fail "a directory-query error reached the dry-run move" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] \
  || fail "a directory-query error moved the database"
pass "--force cannot bypass a directory-query inspection failure"

home="$(setup lsof-partial)"
mkdir -p "$home/lsof-bin"
cat > "$home/lsof-bin/lsof" <<'STUB'
#!/bin/sh
if [ "$1" = "-t" ] && [ "$2" = "-p" ]; then
  printf '%s\n' "$3"
  exit 0
fi
printf 'COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nbash 123 user 9w REG 1,13 0 123 %s/axon.db\n' "$3"
printf 'lsof: WARNING: injected partial inspection failure\n' >&2
exit 1
STUB
chmod +x "$home/lsof-bin/lsof"
out="$(run_with_path "$home" "$home/lsof-bin:$PATH" --dry-run --force 2>&1)" \
  && fail "--force bypassed an lsof partial-result diagnostic"
case "$out" in
  *"lsof inspection failed"*) ;;
  *) fail "a partial-result diagnostic was not rejected: $out" ;;
esac
case "$out" in
  *"would move"*) fail "a partial-result diagnostic reached the dry-run move" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] \
  || fail "a partial-result diagnostic moved the database"
pass "--force cannot bypass an lsof partial-result diagnostic"

for output_case in empty mixed; do
  home="$(setup lsof-success-$output_case)"
  mkdir -p "$home/lsof-bin"
  cat > "$home/lsof-bin/lsof" <<STUB
#!/bin/sh
if [ "\$1" = "-t" ] && [ "\$2" = "-p" ]; then
  printf '%s\\n' "\$3"
  exit 0
fi
STUB
  if [ "$output_case" = mixed ]; then
    printf "printf '123\\\\nnot-a-pid\\\\n'\\n" >> "$home/lsof-bin/lsof"
  fi
  printf 'exit 0\n' >> "$home/lsof-bin/lsof"
  chmod +x "$home/lsof-bin/lsof"
  out="$(run_with_path "$home" "$home/lsof-bin:$PATH" --dry-run --force 2>&1)" \
    && fail "--force accepted lsof success with $output_case output"
  case "$out" in
    *"lsof reported success without PIDs"* | *"malformed PID output"* | *"lsof could not complete the scan"*) ;;
    *) fail "invalid successful lsof output was not rejected: $out" ;;
  esac
  case "$out" in
    *"would move"*) fail "invalid successful lsof output reached the dry-run move" ;;
  esac
  [ -e "$home/overlay/data/axon/axon.db" ] \
    || fail "invalid successful lsof output moved the database"
done
pass "--force cannot bypass empty or mixed successful lsof output"

# --- a stale lock written under the old name after the data move -------------

home="$(setup stale-lock)"
mv "$home/overlay/data/axon" "$home/overlay/data/sjel" || fail "could not stage the already-moved data directory"
mv "$home/overlay/data/sjel/axon.db" "$home/overlay/data/sjel/sjel.db" || fail "could not stage the renamed database"
printf 'stale lock\n' > "$home/overlay/data/sjel/axon-local-stale.lock"
out="$(run "$home" 2>&1)" || fail "a stale lock cleanup exited non-zero: $out"
[ -e "$home/overlay/data/sjel/axon-local-stale.lock" ] && fail "a stale legacy lock survived cleanup"
pass "a stale lock created after the main move is removed on a safe rerun"

mkdir -p "$home/lsof-bin"
printf '#!/bin/sh\nexit 2\n' > "$home/lsof-bin/lsof"
chmod +x "$home/lsof-bin/lsof"
printf 'stale lock\n' > "$home/overlay/data/sjel/axon-local-unverified.lock"
out="$(run_with_path "$home" "$home/lsof-bin:$PATH" 2>&1)" \
  && fail "a lock was removed when lsof failed"
case "$out" in
  *"lsof inspection failed"*) ;;
  *) fail "an lsof error did not stop cleanup: $out" ;;
esac
[ -e "$home/overlay/data/sjel/axon-local-unverified.lock" ] \
  || fail "a legacy lock was removed after lsof failed"
pass "a lock stays in place when holder inspection fails"

home="$(setup stale-lock-query-error)"
mv "$home/overlay/data/axon" "$home/overlay/data/sjel" || fail "could not stage already-moved data"
mv "$home/overlay/data/sjel/axon.db" "$home/overlay/data/sjel/sjel.db" || fail "could not stage renamed database"
rm -f "$home/overlay/data/sjel/axon-local-abc.lock"
printf 'stale lock\n' > "$home/overlay/data/sjel/axon-local-query.lock"
mkdir -p "$home/lsof-bin"
cat > "$home/lsof-bin/lsof" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" >> "$HOME/lsof-args"
if [ "$1" = "-t" ] && [ "$2" = "-p" ]; then
  printf '%s\n' "$3"
  exit 0
fi
printf 'lsof: WARNING: injected lock-query failure\n' >&2
exit 2
STUB
chmod +x "$home/lsof-bin/lsof"
out="$(run_with_path "$home" "$home/lsof-bin:$PATH" 2>&1)" \
  && fail "lock cleanup proceeded after lsof failed on the lock itself"
grep -F -- "-t +w $home/overlay/data/sjel/axon-local-query.lock" "$home/lsof-args" >/dev/null \
  || fail "lsof never queried the lock whose preservation was asserted"
case "$out" in
  *"lsof inspection failed"*) ;;
  *) fail "lock-query failure was not reported: $out" ;;
esac
[ -e "$home/overlay/data/sjel/axon-local-query.lock" ] \
  || fail "the lock was removed after its query failed"
pass "a lock stays in place when its own lsof query fails"

# --- two deployments -------------------------------------------------------

home="$(setup two)"
mkdir -p "$home/overlay/data/sjel"
sqlite3 "$home/overlay/data/sjel/sjel.db" "create table a (id integer);"
out="$(run "$home" 2>&1)" && fail "a migration over an existing sjel.db exited zero"
case "$out" in
  *"two deployments"*) ;;
  *) fail "an existing sjel.db was not refused by name: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] || fail "the old database was moved over a newer one"
pass "two databases are refused rather than merged"

# --- same-directory database and sidecar collisions -------------------------

for suffix in "" "-wal" "-shm"; do
  home="$(setup source-collision-$suffix)"
  if [ -z "$suffix" ]; then
    sqlite3 "$home/overlay/data/axon/sjel.db" \
      "create table a (id integer primary key, body text); create table b (id integer); insert into a (body) values ('newer');"
  else
    printf 'newer sidecar\n' > "$home/overlay/data/axon/sjel.db$suffix"
  fi
  out="$(run "$home" 2>&1)" && fail "a source-directory $suffix collision exited zero"
  case "$out" in
    *"resolve the collision by hand"*) ;;
    *) fail "a source-directory collision was not refused: $out" ;;
  esac
  [ -e "$home/overlay/data/axon/axon.db" ] || fail "a source collision lost axon.db"
  [ -e "$home/overlay/data/axon/sjel.db$suffix" ] || fail "a source collision lost sjel.db$suffix"
  if [ -z "$suffix" ]; then
    [ "$(sqlite3 "$home/overlay/data/axon/sjel.db" 'select body from a;')" = "newer" ] \
      || fail "the newer source database was overwritten"
  fi
done
pass "same-directory database and sidecar collisions are preserved"

home="$(setup backup-collision)"
printf 'newer database backup\n' > "$home/overlay/data/axon/sjel.db.pre-turso-backup"
out="$(run "$home" 2>&1)" && fail "a same-directory backup collision exited zero"
case "$out" in
  *"both pre-Turso database backups exist"*) ;;
  *) fail "a same-directory backup collision was not refused: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db.pre-turso-backup" ] \
  || fail "the old backup was lost after a collision"
[ "$(<"$home/overlay/data/axon/sjel.db.pre-turso-backup")" = "newer database backup" ] \
  || fail "the newer backup was overwritten"
pass "same-directory backup collisions are preserved"

# --- dry run ---------------------------------------------------------------

home="$(setup dry)"
out="$(run "$home" --dry-run 2>&1)" || fail "a dry run exited non-zero: $out"
[ -e "$home/overlay/data/axon/axon.db" ] || fail "a dry run moved something"
case "$out" in
  *"would move"*) ;;
  *) fail "a dry run did not say what it would move: $out" ;;
esac
pass "a dry run reports the moves and changes nothing"

# --- a partial destination -------------------------------------------------

home="$(setup partial)"
mkdir -p "$home/overlay/data/sjel"
out="$(run "$home" 2>&1)" && fail "a migration into an existing data/sjel exited zero"
case "$out" in
  *"already exists"*) ;;
  *) fail "an existing data/sjel was not refused by name: $out" ;;
esac
pass "a directory already at the destination is refused"

if [ "$fails" -eq 0 ]; then
  echo "migrate-axon-names tests passed"
else
  echo "migrate-axon-names: $fails case(s) FAILED"
  exit 1
fi

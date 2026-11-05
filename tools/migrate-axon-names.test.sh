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

# --- the move --------------------------------------------------------------

home="$(setup move)"
out="$(run "$home" 2>&1)" || fail "a clean migration exited non-zero: $out"
[ -e "$home/overlay/data/sjel/sjel.db" ] || fail "the database was not renamed to sjel.db"
[ -e "$home/overlay/data/axon" ] && fail "the old data directory is still there"
[ "$(sqlite3 "$home/overlay/data/sjel/sjel.db" "select body from a;")" = "kept" ] \
  || fail "the moved database lost its rows"
[ -e "$home/overlay/data/sjel/sjel.db-wal" ] || fail "the WAL sidecar did not follow the database"
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
  *"still open by"*) ;;
  *) fail "a live writer was not named: $out" ;;
esac
[ -e "$home/overlay/data/axon/axon.db" ] || fail "a refused migration moved the database anyway"
wait "$holder_pid" 2>/dev/null
pass "a process holding the database is named and the migration refuses"

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

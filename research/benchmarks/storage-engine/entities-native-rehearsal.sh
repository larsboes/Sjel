#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo 'usage: sh entities-native-rehearsal.sh <Entities WAL file> <native entities-server> <new scratch directory>' >&2
    exit 2
fi
source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
binary=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
tools=$(cd "$(dirname "$0")/../../../tools" && pwd -P)
test -f "$source_db"
test ! -s "$source_db-wal"
test -x "$binary"
mkdir -m 700 "$3"
cd "$3"
work=$(pwd -P)
overlay="$work/overlay"
mkdir -p "$overlay/config" "$work/target"
cp "$source_db" "$work/entities.db"
fixture=entities-native-rehearsal
printf '%s\n' "$fixture" > "$work/fixture-bearer"
printf 'SJEL_INBOUND_TOKEN_FILE=%s/fixture-bearer\n' "$work" > "$overlay/config/deployment.env"
case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo 'unsupported rehearsal host' >&2; exit 1 ;;
esac
printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["entities-wal-backup"]\n' "$os" > "$overlay/config/machine.toml"
printf '[rehearsal-target]\nkind = "local"\npath = "%s/target"\n' "$work" > "$overlay/config/systems.local.toml"
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
base="http://127.0.0.1:$port"
export SJEL_OVERLAY_ROOT="$overlay" SJEL_PERSONAL_ROOT="$overlay"
export SJEL_DB_PATH="$work/unused-shared.db"
SJEL_PORT="$port" SJEL_ENTITIES_DB_PATH="$work/entities.db" "$binary" > service.log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :' EXIT
trap 'exit 1' HUP INT TERM
tries=0
until curl -qfsS --noproxy '*' --max-time 2 "$base/ready" >/dev/null 2>&1; do
    kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 50 ] || { echo "See $work/service.log" >&2; exit 1; }
    tries=$((tries + 1))
    sleep 0.1
done

response=$(curl -qfsS --noproxy '*' --max-time 5 -H "Authorization: Bearer $fixture" \
    -H 'Content-Type: application/json' --data '{"kind":"person","name":"Sjel Native Rehearsal"}' \
    "$base/api/entities")
entity=$(printf '%s' "$response" | jq -er '.id')
printf '%s' "$response" | jq -e '.revision == 1' >/dev/null
export SJEL_ENTITIES_DB_PATH="$work/unused-job.db"
"$binary" names | jq -e '.tokens | length > 0' >/dev/null
test ! -e "$SJEL_ENTITIES_DB_PATH"
test ! -e "$SJEL_DB_PATH"

cap=entities-wal-backup
SJEL_ENTITIES_BIN="$binary" "$tools/backup.sh" --no-prune --target rehearsal-target "$cap"
receipt="$overlay/backup/receipts/$cap.json"
archive="$work/target/$cap/$(jq -er '.tarball' "$receipt")"
"$tools/restore.sh" "$cap" "$archive" --receipt "$receipt" --destination "$work/restored"
restored="file:$work/restored/data/entities/snapshots/entities.db?immutable=1"
test "$(sqlite3 -readonly "$restored" "SELECT revision FROM entities_entities WHERE id='$entity'")" = 1
test -z "$(sqlite3 -readonly "$restored" 'PRAGMA foreign_key_check')"
test "$(sqlite3 -readonly "$restored" "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name GLOB 'entities_*'")" = 9
printf '%s\n' 'Native owner, private job socket, scheduled backup command, and receipt-backed restore passed.'

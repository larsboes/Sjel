#!/bin/sh
set -eu
umask 077

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo 'usage: sh inventory-backup-rehearsal.sh <closed native Inventory snapshot> <new private scratch directory> [native-enabled inventory binary]' >&2
    exit 2
fi

tools=$(cd "$(dirname "$0")/../../../tools" && pwd -P)
source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
inventory=
if [ "$#" -eq 3 ]; then
    inventory=$(cd "$(dirname "$3")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$3")")
    test -x "$inventory"
fi
test -f "$source_db"
test ! -s "$source_db-wal"
source_sha=$(shasum -a 256 "$source_db")
mkdir -m 700 "$2"
cd "$2"
work=$(pwd -P)
overlay="$work/overlay"
cap=inventory-wal-rehearsal
mkdir -p "$overlay/capabilities/$cap" "$overlay/config" "$overlay/data/inventory" "$work/target"
cp "$source_db" "$overlay/data/inventory/snapshot.db"
cat > "$overlay/capabilities/$cap/service.toml" <<'MANIFEST'
kind = "data"
name = "inventory-wal-rehearsal"
backup_sqlite_online = "data/inventory/snapshot.db"
backup_target = "rehearsal-target"
MANIFEST
cat > "$overlay/config/systems.local.toml" <<SYSTEMS
[rehearsal-target]
kind = "local"
path = "$work/target"
SYSTEMS

case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo 'unsupported backup rehearsal host' >&2; exit 1 ;;
esac
printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["%s"]\n' "$os" "$cap" > "$overlay/config/machine.toml"

SJEL_OVERLAY_ROOT="$overlay" "$tools/backup.sh" --no-prune "$cap"
receipt="$overlay/backup/receipts/$cap.json"
archive="$work/target/$cap/$(jq -er '.tarball' "$receipt")"
SJEL_OVERLAY_ROOT="$overlay" "$tools/restore.sh" "$cap" "$archive" \
    --receipt "$receipt" --destination "$work/restored"

restored_db="$work/restored/data/inventory/snapshot.db"
test ! -s "$restored_db-wal"
sqlite3 -readonly "file:$source_db?immutable=1" '.dump inventory_%' > source.sql
sqlite3 -readonly "file:$restored_db?immutable=1" '.dump inventory_%' > restored.sql
cmp -s source.sql restored.sql
test "$(grep -c '^CREATE TABLE' restored.sql)" = 10
sequences='SELECT name,seq FROM sqlite_sequence ORDER BY name;'
test "$(sqlite3 -readonly "file:$source_db?immutable=1" "$sequences")" = \
    "$(sqlite3 -readonly "file:$restored_db?immutable=1" "$sequences")"
test -z "$(sqlite3 -readonly "file:$restored_db?immutable=1" 'PRAGMA foreign_key_check')"
test "$(shasum -a 256 "$source_db")" = "$source_sha"
printf '%s\n' 'Inventory snapshot archive and receipt restored all ten tables and sequence counters.'

if [ -n "$inventory" ]; then
    # Use the public fixture only on the isolated preview listener.
    fixture=inventory-restore-fixture
    printf '%s\n' "$fixture" > "$overlay/fixture-bearer"
    printf 'SJEL_INBOUND_TOKEN_FILE=%s/fixture-bearer\n' "$overlay" > "$overlay/config/deployment.env"
    port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    base="http://127.0.0.1:$port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$overlay" "$inventory" \
        serve-wal-preview "$restored_db" "$port" > native-restore.log 2>&1 &
    pid=$!
    trap 'kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :' EXIT
    trap 'exit 1' HUP INT TERM
    tries=0
    until curl -qfsS --noproxy '*' --max-time 2 "$base/health" >/dev/null 2>&1; do
        kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 50 ] || {
            echo "The native preview did not open the restore. See $work/native-restore.log." >&2
            exit 1
        }
        tries=$((tries + 1))
        sleep 0.1
    done
    expected=$(sqlite3 -readonly "file:$restored_db?immutable=1" 'SELECT count(*) FROM inventory_item')
    curl -qfsS --noproxy '*' --max-time 3 -H "Authorization: Bearer $fixture" \
        "$base/api/inventory" > native-restored.json
    jq -e --argjson count "$expected" 'length == $count' native-restored.json >/dev/null
    echo 'The native Inventory API served the restored catalogue.'
fi

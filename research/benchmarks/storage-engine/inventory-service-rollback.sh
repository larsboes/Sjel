#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo 'usage: sh inventory-service-rollback.sh <wal-restored.db> <inventory binary> <new private scratch directory>' >&2
    exit 2
fi

source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
inventory=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
test -x "$inventory"
test ! -s "$source_db-wal"
source_sha=$(shasum -a 256 "$source_db")
mkdir -m 700 "$3"
cd "$3"
work=$(pwd -P)
cp "$source_db" owner.db
chmod 600 owner.db
mkdir config

# This public fixture is only for the isolated loopback server.
fixture=inventory-rehearsal-fixture
printf '%s\n' "$fixture" > fixture-bearer
printf 'SJEL_INBOUND_TOKEN_FILE=%s/fixture-bearer\n' "$work" > config/deployment.env

pid=
stop_service() {
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || :
        wait "$pid" 2>/dev/null || :
        pid=
    fi
}
trap stop_service EXIT
trap 'exit 1' HUP INT TERM

request() {
    curl -q --silent --show-error --noproxy '*' --max-time 3 \
        -H "Authorization: Bearer $fixture" "$@"
}

start_service() {
    port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    base="http://127.0.0.1:$port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/devices.db" \
        SJEL_INVENTORY_DB_PATH="$work/owner.db" SJEL_PORT="$port" \
        "$inventory" serve > "$1.log" 2>&1 &
    pid=$!
    tries=0
    until request --fail "$base/health" >/dev/null 2>&1; do
        if ! kill -0 "$pid" 2>/dev/null || [ "$tries" -eq 50 ]; then
            echo "Inventory did not start. See $work/$1.log." >&2
            return 1
        fi
        tries=$((tries + 1))
        sleep 0.1
    done
    kill -0 "$pid"
}

item=sjel-turso-rehearsal-item
reference="inv:$(printf '%s' "$item" | od -An -v -tx1 | tr -d ' \n')"

check_item() {
    request --fail "$base/api/item-refs/$reference" |
        jq -e --arg item "$item" --argjson revision "$1" \
            '.raw_id == $item and .revision == $revision and .state == "owned"' >/dev/null
}

start_service before-restart
check_item 3
test "$(request --output edit.json --dump-header edit.headers --write-out '%{http_code}' \
    -X PATCH -H 'Content-Type: application/json' -H 'If-Match: "3"' \
    --data '{"label":"Restored service edit"}' "$base/api/items/$item")" = 200
jq -e --arg item "$item" '.id == $item and .ok and .revision == 4' edit.json >/dev/null
tr -d '\r' < edit.headers | grep -iqx 'etag: "4"'
test "$(request --output stale.json --write-out '%{http_code}' -X PATCH \
    -H 'Content-Type: application/json' -H 'If-Match: "3"' \
    --data '{"label":"Refused stale edit"}' "$base/api/items/$item")" = 409
jq -e '.current.item.revision == 4 and .current.item.label == "Restored service edit"' stale.json >/dev/null
stop_service

start_service after-restart
check_item 4
stop_service
test "$(sqlite3 -readonly owner.db 'PRAGMA integrity_check')" = ok
test -z "$(sqlite3 -readonly owner.db 'PRAGMA foreign_key_check')"
test "$(sqlite3 -readonly owner.db "SELECT label FROM inventory_item WHERE id='sjel-turso-rehearsal-item' AND revision=4")" = 'Restored service edit'
test "$(sqlite3 -readonly owner.db "SELECT revision FROM inventory_sync_operation WHERE operation_id='sjel-turso-rehearsal-op'")" = 3
test "$(sqlite3 -readonly devices.db "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'inventory_*'")" = 0
test "$(shasum -a 256 "$source_db")" = "$source_sha"
echo 'Inventory served the restored revision, kept the Turso receipt, rejected a stale edit, and retained the new revision after restart.'

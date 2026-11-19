#!/bin/sh
set -eu
umask 077

if [ "$#" -lt 3 ] || [ "$#" -gt 5 ]; then
    echo 'usage: sh inventory-service-rollback.sh <wal-restored.db> <inventory binary> <new private scratch directory> [finance binary [interior binary]]' >&2
    exit 2
fi

source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
inventory=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
test -x "$inventory"
finance=
if [ "$#" -ge 4 ]; then
    finance=$(cd "$(dirname "$4")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$4")")
    test -x "$finance"
fi
interior=
if [ "$#" -eq 5 ]; then
    interior=$(cd "$(dirname "$5")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$5")")
    test -x "$interior"
    fixture_data=$(cd "$(dirname "$0")/../../../crates/interior/tests/fixtures/overlay/data" && pwd -P)
fi
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
aux_pid=
finance_url=
stop_service() {
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || :
        wait "$pid" 2>/dev/null || :
        pid=
    fi
}
stop_auxiliary() {
    if [ -n "$aux_pid" ]; then
        kill "$aux_pid" 2>/dev/null || :
        wait "$aux_pid" 2>/dev/null || :
        aux_pid=
    fi
}
trap 'stop_service; stop_auxiliary' EXIT
trap 'exit 1' HUP INT TERM

request() {
    curl -q --silent --show-error --noproxy '*' --max-time 3 \
        -H "Authorization: Bearer $fixture" "$@"
}

available_port() {
    python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

wait_for_service() {
    tries=0
    until request --fail "$2/health" >/dev/null 2>&1; do
        if ! kill -0 "$1" 2>/dev/null || [ "$tries" -eq 50 ]; then
            echo "Service did not start. See $work/$3.log." >&2
            return 1
        fi
        tries=$((tries + 1))
        sleep 0.1
    done
    kill -0 "$1"
}

start_service() {
    port=$(available_port)
    base="http://127.0.0.1:$port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/devices.db" \
        SJEL_INVENTORY_DB_PATH="$work/owner.db" SJEL_PORT="$port" SJEL_FINANCE_URL="$finance_url" \
        "$inventory" serve > "$1.log" 2>&1 &
    pid=$!
    wait_for_service "$pid" "$base" "$1"
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

if [ -n "$finance" ]; then
    finance_port=$(available_port)
    finance_url="http://127.0.0.1:$finance_port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/finance.db" \
        SJEL_PORT="$finance_port" "$finance" > finance.log 2>&1 &
    aux_pid=$!
    wait_for_service "$aux_pid" "$finance_url" finance
    request --fail "$finance_url/api/monthly-balance" | jq -e '. == null' >/dev/null
    sqlite3 -bail finance.db <<'SQL'
INSERT INTO finance_transaction_projection
    (id, booked_at, description, kind, account, category, amount_cents, currency)
VALUES ('income', '2000-01-01', '', 'income', '', '', 20000, 'EUR'),
       ('expense', '2000-01-02', '', 'expense', '', '', 5000, 'EUR');
SQL
    sqlite3 -bail owner.db <<'SQL'
INSERT INTO inventory_item(id, kind, label, price_cents, created_at, updated_at)
VALUES ('rehearsal-wish', 'gear', 'Rehearsal wish', 30000, '2000-01-01', '2000-01-01');
INSERT INTO inventory_item_state(item_id, state, since)
VALUES ('rehearsal-wish', 'wanted', '2000-01-01');
SQL
    start_service wishlist
    request --fail "$base/api/wishlist" | jq -e '
        .monthly_balance == {median_cents: 15000, months: 1, from: "2000-01", to: "2000-01"}
        and .months_to_pay == (.total_low_cents / 15000)
        and any(.items[]; .id == "rehearsal-wish" and .price_cents == 30000)
    ' >/dev/null
    stop_auxiliary
    request --fail "$base/api/wishlist" | jq -e '
        .monthly_balance == null and .months_to_pay == null and any(.items[]; .id == "rehearsal-wish")
    ' >/dev/null
    stop_service
    test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'finance_*'")" = 0
    echo 'Wishlist used the Finance balance across separate files and stayed available after Finance stopped.'
fi

if [ -n "$interior" ]; then
    start_service interior-owner
    inventory_url=$base
    mkdir -p data
    cp -R "$fixture_data/interior" data/
    capture=data/interior/captures/muster/revision-fixture
    mkdir -p "$capture"
    printf '%s\n' '{"draft_id":"revision-fixture","room":{"objects":[{"id":"object-fixture"}]}}' > "$capture/draft.json"
    touch "$capture/captured-room.usdz"
    interior_port=$(available_port)
    interior_url="http://127.0.0.1:$interior_port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/interior.db" \
        SJEL_INVENTORY_URL="$inventory_url" SJEL_INTERIOR_FLAT=muster SJEL_PORT="$interior_port" \
        "$interior" serve > interior.log 2>&1 &
    aux_pid=$!
    wait_for_service "$aux_pid" "$interior_url" interior
    catalogue_size=$(sqlite3 -readonly owner.db 'SELECT count(*) FROM inventory_item')
    request --fail "$interior_url/api/model" |
        jq -e --argjson size "$catalogue_size" '.catalogue_size == $size' >/dev/null
    request --fail -H 'Content-Type: application/json' --data '{"revision_id":"revision-fixture"}' \
        "$interior_url/api/roomplan/spaces" | jq -e '.space_id == "space-revision-fixture"' >/dev/null
    link="$interior_url/api/roomplan/spaces/space-revision-fixture/links"
    test "$(request --output /dev/null --write-out '%{http_code}' -X PUT \
        -H 'Content-Type: application/json' \
        --data '{"revision_id":"revision-fixture","observation_id":"object-fixture","item_id":"rehearsal-wish"}' "$link")" = 409
    request --fail -X PUT -H 'Content-Type: application/json' \
        --data '{"revision_id":"revision-fixture","observation_id":"object-fixture","item_id":"sjel-turso-rehearsal-item"}' "$link" |
        jq -e '.item_id == "sjel-turso-rehearsal-item"' >/dev/null
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/interior.db" \
        SJEL_INVENTORY_URL="$inventory_url" SJEL_INTERIOR_FLAT=muster "$interior" model >/dev/null
    stop_auxiliary
    stop_service
    test "$(sqlite3 -readonly interior.db "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'inventory_*'")" = 0
    test "$(sqlite3 -readonly interior.db "SELECT item_id FROM interior_roomplan_object_link")" = "$item"
    echo 'Interior served the owner catalogue and accepted only the owned RoomPlan link across separate files.'
fi

sqlite3 -bail owner.db <<'SQL'
INSERT INTO inventory_item(id, kind, label, created_at, updated_at)
VALUES ('cli/probe +?', 'gear', 'Price CLI fixture', '2000-01-01', '2000-01-01');
-- Keep the scheduled runner on local fixtures during the rehearsal.
UPDATE inventory_item SET link = NULL WHERE id IN (SELECT item_id FROM inventory_wish_group_item);
SQL
start_service price-cli
inventory_cli() {
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/cli-must-not-open.db" \
        SJEL_INVENTORY_DB_PATH="$work/cli-must-not-open.db" SJEL_INVENTORY_URL="$base" \
        "$inventory" "$@"
}
inventory_cli compare sjel-turso-rehearsal-group > compare.log
grep -q 'sjel-turso-rehearsal-group' compare.log
test "$(inventory_cli prices refresh)" = '0 read, 0 failed'
inventory_cli specs 'cli/probe +?' memory_gb=32 >/dev/null
inventory_cli needs sjel-turso-rehearsal-group memory_gb=24 >/dev/null
inventory_cli alternatives sjel-turso-rehearsal-group yes >/dev/null
inventory_cli price 'cli/probe +?' 79.90 --source rehearsal --on 2000-01-03 > price.log
inventory_cli price 'cli/probe +?' 81.00 --source rehearsal --on 2000-01-03 > price-retry.log
grep -q 'nothing written' price-retry.log
stop_service
test ! -e cli-must-not-open.db
test "$(sqlite3 -readonly owner.db "SELECT price_cents FROM inventory_price_observation WHERE item_id='cli/probe +?' AND source='rehearsal'")" = 7990
test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM inventory_wish_spec WHERE item_id='cli/probe +?' AND key='memory_gb' AND value=32;")" = 1
test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM inventory_wish_group_need WHERE group_id='sjel-turso-rehearsal-group' AND key='memory_gb' AND value=24;")" = 1
test "$(sqlite3 -readonly owner.db "SELECT alternatives FROM inventory_wish_group WHERE id='sjel-turso-rehearsal-group';")" = 1
echo 'The comparison CLI used the owner, preserved the first price on retry, and opened no local database.'

test "$(shasum -a 256 "$source_db")" = "$source_sha"
echo 'Inventory served the restored revision, kept the Turso receipt, rejected a stale edit, and retained the new revision after restart.'

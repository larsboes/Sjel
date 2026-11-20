#!/bin/sh
set -eu
umask 077

if [ "$#" -lt 3 ] || [ "$#" -gt 5 ]; then
    echo 'usage: sh inventory-service-rollback.sh <wal-restored.db> <inventory binary> <new private scratch directory> [finance binary [interior binary]]' >&2
    exit 2
fi

script_dir=$(cd "$(dirname "$0")" && pwd -P)
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
        SJEL_INVENTORY_DB_PATH="$work/${2:-owner.db}" SJEL_PORT="$port" SJEL_FINANCE_URL="$finance_url" \
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
CREATE TRIGGER refuse_fixture_initial_state BEFORE INSERT ON inventory_item_state
WHEN NEW.item_id = 'refused-create'
BEGIN SELECT RAISE(ABORT, 'fixture state refused'); END;
SQL
start_service price-cli
if env -i PATH="$PATH" SJEL_INVENTORY_DB_PATH="$work/cli-must-not-open.db" \
    "$inventory" inventory > missing-owner.log 2>&1; then exit 1; fi
grep -q 'Set SJEL_INVENTORY_URL' missing-owner.log
inventory_cli() {
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_DB_PATH="$work/cli-must-not-open.db" \
        SJEL_INVENTORY_DB_PATH="$work/cli-must-not-open.db" SJEL_INVENTORY_URL="$base" \
        "$inventory" "$@"
}
inventory_cli compare sjel-turso-rehearsal-group > compare.log
grep -q 'sjel-turso-rehearsal-group' compare.log
inventory_cli inventory > inventory.log
inventory_cli categories > categories.log
test "$(inventory_cli prices refresh)" = '0 read, 0 failed'
inventory_cli specs 'cli/probe +?' memory_gb=32 >/dev/null
inventory_cli needs sjel-turso-rehearsal-group memory_gb=24 >/dev/null
inventory_cli alternatives sjel-turso-rehearsal-group yes >/dev/null
inventory_cli price 'cli/probe +?' 79.90 --source rehearsal --on 2000-01-03 > price.log
inventory_cli price 'cli/probe +?' 81.00 --source rehearsal --on 2000-01-03 > price-retry.log
grep -q 'nothing written' price-retry.log
test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
    --data '{"id":"refused-create","kind":"gear","label":"Must roll back","state":"wanted"}' "$base/api/items")" = 500
request --fail -H 'Content-Type: application/json' \
    --data '{"id":"owner-create","kind":"gear","label":"First label","state":"wanted"}' "$base/api/items" |
    jq -e '.revision == 1 and .state == "wanted"' >/dev/null
test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
    --data '{"id":"owner-create","kind":"gear","label":"Must not overwrite","state":"owned"}' "$base/api/items")" = 409
mkdir -p data/inventory/inventory
cat > data/inventory/inventory/owned.toml <<'TOML'
[[item]]
id = "import-control"
label = "Import fixture"

[[item]]
id = "refused-create"
label = "Must roll back the entire import"
TOML
touch data/inventory/inventory/wishlist.toml
if inventory_cli import > import-refused.log 2>&1; then exit 1; fi
grep -q '409 Conflict' import-refused.log
if inventory_cli import --force > import-rolled-back.log 2>&1; then exit 1; fi
grep -q '500 Internal Server Error' import-rolled-back.log
reference="inv:$(printf '%s' 'import-control' | od -An -v -tx1 | tr -d ' \n')"
test "$(request --output /dev/null --write-out '%{http_code}' "$base/api/item-refs/$reference")" = 404
sed 's/refused-create/import-second/' data/inventory/inventory/owned.toml > owned-retry.toml
mv owned-retry.toml data/inventory/inventory/owned.toml
inventory_cli import --force > import.log 2>&1
grep -q '2 pieces, 0 needs, 2 state changes' import.log
stop_service
test ! -e cli-must-not-open.db
test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM inventory_item WHERE id='refused-create';")" = 0
test "$(sqlite3 -readonly owner.db "SELECT label || '|' || revision FROM inventory_item WHERE id='owner-create';")" = 'First label|1'
test "$(sqlite3 -readonly owner.db "SELECT state FROM inventory_item_state WHERE item_id='owner-create';")" = wanted
test "$(sqlite3 -readonly owner.db "SELECT price_cents FROM inventory_price_observation WHERE item_id='cli/probe +?' AND source='rehearsal'")" = 7990
test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM inventory_wish_spec WHERE item_id='cli/probe +?' AND key='memory_gb' AND value=32;")" = 1
test "$(sqlite3 -readonly owner.db "SELECT count(*) FROM inventory_wish_group_need WHERE group_id='sjel-turso-rehearsal-group' AND key='memory_gb' AND value=24;")" = 1
test "$(sqlite3 -readonly owner.db "SELECT alternatives FROM inventory_wish_group WHERE id='sjel-turso-rehearsal-group';")" = 1
echo 'The comparison CLI used the owner, preserved the first price on retry, and opened no local database.'
echo 'Item creation rolled back a refused state and left an existing item unchanged on a duplicate request.'
echo 'The owner rejected an unforced import, rolled back a failed forced import, and committed both rows on retry.'

if [ "${SJEL_TEST_NATIVE_PREVIEW:-0}" = 1 ]; then
    sqlite3 -bail owner.db '.backup native-owner.db'
    native_sha=$(shasum -a 256 native-owner.db)
    start_service sqlite-control
    sqlite_url=$base
    native_port=$(available_port)
    native_url="http://127.0.0.1:$native_port"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_FINANCE_URL="$finance_url" \
        "$inventory" serve-wal-preview "$work/native-owner.db" "$native_port" > native.log 2>&1 &
    aux_pid=$!
    wait_for_service "$aux_pid" "$native_url" native
    reference="inv:$(printf '%s' "$item" | od -An -v -tx1 | tr -d '[:space:]')"
    for path in /api/inventory '/api/search?q=Restored+service' "/api/item-refs/$reference" \
        /api/wishlist /api/wish-groups /api/wish-comparison /api/mail-proposals /routes; do
        request --fail "$sqlite_url$path" | jq -S . > sqlite-response.json
        request --fail "$native_url$path" | jq -S . > native-response.json
        cmp -s sqlite-response.json native-response.json
    done
    test "$(request --output /dev/null --write-out '%{http_code}' "$native_url/api/item-refs/inv:6d697373696e67")" = 404
    test "$(request --output /dev/null --write-out '%{http_code}' -X POST "$native_url/api/inventory")" = 405
    test "$(request --output /dev/null --write-out '%{http_code}' -X PATCH "$native_url/api/items/$item")" = 404
    stop_auxiliary
    stop_service
    test "$(shasum -a 256 native-owner.db)" = "$native_sha"
    echo 'The native WAL preview matched catalogue, search and reference responses and refused mutations.'

    native_port=$(available_port)
    native_url="http://127.0.0.1:$native_port"
    snapshot_dir="$work/data/inventory/snapshots"
    mkdir -p "$snapshot_dir"
    env -i PATH="$PATH" SJEL_PERSONAL_ROOT="$work" SJEL_INVENTORY_ENGINE=turso-wal \
        SJEL_INVENTORY_SNAPSHOT_DIR="$snapshot_dir" SJEL_DB_PATH="$work/devices.db" \
        SJEL_INVENTORY_DB_PATH="$work/native-owner.db" SJEL_PORT="$native_port" \
        SJEL_FINANCE_URL="$finance_url" "$inventory" serve > native-write.log 2>&1 &
    aux_pid=$!
    wait_for_service "$aux_pid" "$native_url" native-write
    request --fail --dump-header native-edit.headers -X PATCH -H 'Content-Type: application/json' \
        -H 'If-Match: "4"' --data '{"label":"Acknowledged native API edit"}' "$native_url/api/items/$item" |
        jq -e '.revision == 5' >/dev/null
    tr -d '\r' < native-edit.headers | grep -iqx 'etag: "5"'
    test "$(request --output native-stale.json --write-out '%{http_code}' -X PATCH \
        -H 'Content-Type: application/json' -H 'If-Match: "4"' \
        --data '{"label":"Must not overwrite"}' "$native_url/api/items/$item")" = 409
    jq -e '.current.item.revision == 5 and .current.item.label == "Acknowledged native API edit"' native-stale.json >/dev/null
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"id":"refused-create","kind":"gear","label":"Refused native create","state":"owned"}' "$native_url/api/items")" = 500
    test "$(request --output /dev/null --write-out '%{http_code}' "$native_url/api/item-refs/inv:726566757365642d637265617465")" = 404
    request --fail -H 'Content-Type: application/json' \
        --data '{"id":"native-item","kind":"gear","label":"Native item","state":"wanted"}' "$native_url/api/items" |
        jq -e '.revision == 1 and .state == "wanted"' >/dev/null
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"id":"native-item","kind":"gear","label":"Duplicate","state":"gone"}' "$native_url/api/items")" = 409
    request --fail -X PUT -H 'Content-Type: application/json' -H 'If-Match: "1"' \
        --data '{"id":"native-item","kind":"gear","label":"Native replacement","cost_max_cents":10000}' "$native_url/api/items/native-item" |
        jq -e '.revision == 2' >/dev/null
    request --fail -H 'Content-Type: application/json' \
        --data '{"state":"owned","note":"Native state transition"}' "$native_url/api/items/native-item/state" |
        jq -e '.state == "owned" and .changed' >/dev/null
    request --fail -H 'Content-Type: application/json' \
        --data '{"state":"owned"}' "$native_url/api/items/native-item/state" |
        jq -e '.state == "owned" and .changed == false' >/dev/null
    request --fail "$native_url/api/items/native-item/state" |
        jq -e 'map(.state) == ["wanted", "owned"] and .[1].note == "Native state transition"' >/dev/null
    sed -e 's/import-control/native-import-control/' -e 's/import-second/refused-create/' \
        data/inventory/inventory/owned.toml > native-import.toml
    mv native-import.toml data/inventory/inventory/owned.toml
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"force":false}' "$native_url/api/import/overlay")" = 409
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"force":true}' "$native_url/api/import/overlay")" = 500
    native_import_ref="inv:$(printf '%s' native-import-control | od -An -v -tx1 | tr -d '[:space:]')"
    test "$(request --output /dev/null --write-out '%{http_code}' "$native_url/api/item-refs/$native_import_ref")" = 404
    sed 's/refused-create/native-import-second/' data/inventory/inventory/owned.toml > native-import-retry.toml
    mv native-import-retry.toml data/inventory/inventory/owned.toml
    request --fail -H 'Content-Type: application/json' --data '{"force":true}' "$native_url/api/import/overlay" |
        jq -e '.pieces == 2 and .state_changes == 2' >/dev/null
    request --fail -H 'Content-Type: application/json' \
        --data '{"id":"native-group","title":"Native group","constraint_text":"Fixture","item_ids":["native-item"]}' \
        "$native_url/api/wish-groups" | jq -e '.id == "native-group"' >/dev/null
    test "$(request --output /dev/null --write-out '%{http_code}' -X PUT -H 'Content-Type: application/json' \
        --data '{"item_ids":["missing-native-item"]}' "$native_url/api/wish-groups/native-group/items")" = 500
    request --fail -X PUT -H 'Content-Type: application/json' --data '{"alternatives":true}' \
        "$native_url/api/wish-groups/native-group/alternatives" >/dev/null
    request --fail -X PUT -H 'Content-Type: application/json' --data '{"specs":[{"key":"weight","value":10,"unit":"g"}]}' \
        "$native_url/api/items/native-item/specs" >/dev/null
    request --fail -X PUT -H 'Content-Type: application/json' --data '{"needs":[{"key":"weight","value":15,"unit":"g"}]}' \
        "$native_url/api/wish-groups/native-group/needs" >/dev/null
    request --fail -H 'Content-Type: application/json' --data '{"price_cents":9000,"source":"fixture","observed_at":"2000-01-01"}' \
        "$native_url/api/items/native-item/prices" | jq -e '.added and .alert' >/dev/null
    request --fail -H 'Content-Type: application/json' --data '{"price_cents":8000,"source":"fixture","observed_at":"2000-01-01"}' \
        "$native_url/api/items/native-item/prices" | jq -e '.added == false and .alert == false' >/dev/null
    request --fail "$native_url/api/wish-comparison" |
        jq -e 'any(.groups[]; .id == "native-group" and .needs[0].value == 15
            and .candidates[0].id == "native-item" and .candidates[0].specs[0].value == 10
            and .candidates[0].observations[0].price_cents == 9000)' >/dev/null
    request --fail "$native_url/api/wish-comparison" | jq -S . > native-wish-comparison.json
    request --fail "$native_url/api/wish-groups" | jq -S . > native-wish-groups.json
    native_ref="inv:$(printf '%s' native-item | od -An -v -tx1 | tr -d '[:space:]')"
    request --fail "$native_url/api/items/native-item/mail-match-snapshot" > native-match.json
    alias_key='{"namespace":"fixture:serial","value":"observed-123","kind":"physical_item"}'
    jq --arg reference "$native_ref" --argjson key "$alias_key" \
        '{key:$key,reference:$reference,expected_item_revision:.item_revision,expected_state_id:.state_id,
          source_ref:"fixture:review",data_class:"c1"}' native-match.json > native-review.json
    alias_id=$(request --fail -H 'Content-Type: application/json' --data-binary @native-review.json \
        "$native_url/api/item-aliases" | jq -er '.id')
    test "$(request --fail -H 'Content-Type: application/json' --data-binary @native-review.json \
        "$native_url/api/item-aliases" | jq -er '.id')" = "$alias_id"
    request --fail -H 'Content-Type: application/json' --data "{\"alias\":$alias_key}" "$native_url/api/items/resolve" |
        jq -e --arg id "$alias_id" '.status == "exact" and (.candidates[0].alias_id|tostring) == $id' >/dev/null
    request --fail -H 'Content-Type: application/json' --data '{"title":"Native replacement"}' "$native_url/api/items/resolve" |
        jq -e '.status == "ambiguous" and .candidates[0].item_id == "native-item"' >/dev/null
    request --fail -H 'Content-Type: application/json' --data '{"version":1}' \
        "$native_url/api/item-aliases/$alias_id/revoke" | jq -e '.version == 2 and .revoked_at != null' >/dev/null
    request --fail -H 'Content-Type: application/json' --data "{\"alias\":$alias_key}" "$native_url/api/items/resolve" |
        jq -e '.status == "unknown" and .total_candidates == 0' >/dev/null
    private_proposal=$(request --fail -H 'Content-Type: application/json' \
        --data '{"source_id":"mail:v1:fixture-private:one","line_id":"1","event":"delivered","title":"Fixture private parcel","evidence":"Fixture evidence","data_class":"c2"}' \
        "$native_url/api/mail-proposals" | jq -er '.id')
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"action":"approve","version":1,"title":"Refuse private create","item_id":"native-private-item"}' \
        "$native_url/api/mail-proposals/$private_proposal/decision")" = 409
    jq '{action:"match",version:1,item_id:.item_id,expected_item_revision:.item_revision,expected_state_id:.state_id}' \
        native-match.json > native-mail-match.json
    request --fail -H 'Content-Type: application/json' --data-binary @native-mail-match.json \
        "$native_url/api/mail-proposals/$private_proposal/decision" |
        jq -e '.decision == "match" and .item_id == "native-item"' >/dev/null
    proposal=$(request --fail -H 'Content-Type: application/json' \
        --data '{"source_id":"mail:v1:fixture-public:one","line_id":"1","event":"delivered","title":"Fixture public parcel","evidence":"Fixture evidence","data_class":"c1"}' \
        "$native_url/api/mail-proposals" | jq -er '.id')
    test "$(request --output /dev/null --write-out '%{http_code}' -H 'Content-Type: application/json' \
        --data '{"action":"approve","version":1,"title":"Refuse atomic mail create","item_id":"refused-create"}' \
        "$native_url/api/mail-proposals/$proposal/decision")" = 500
    request --fail -H 'Content-Type: application/json' \
        --data '{"action":"approve","version":1,"title":"Native mail item","item_id":"native-mail-item"}' \
        "$native_url/api/mail-proposals/$proposal/decision" | jq -e '.decision == "approve" and .version == 2' >/dev/null
    request --fail "$native_url/api/mail-proposals" | jq -S . > native-mail.json
    request --fail "$native_url/api/item-refs/$native_ref/aliases" | jq -S . > native-aliases.json
    tools="$script_dir/../../../tools"
    case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) exit 1 ;; esac
    backup_cap=inventory-wal-backup
    printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["inventory", "%s"]\n' \
        "$os" "$backup_cap" > config/machine.toml
    mkdir backup-target
    cat > config/systems.local.toml <<TARGETS
[rehearsal-target]
kind = "local"
path = "$work/backup-target"
TARGETS
    run_backup() {
        env -i PATH="$PATH" HOME="$HOME" SJEL_PERSONAL_ROOT="$work" SJEL_OVERLAY_ROOT="$work" \
            SJEL_INVENTORY_URL="$1" SJEL_INVENTORY_BIN="$inventory" \
            "$tools/backup.sh" --target rehearsal-target "$backup_cap"
    }
    cp "$source_db" "$snapshot_dir/inventory.db"
    start_service sqlite-during-backup
    receipt="$work/backup/receipts/$backup_cap.json"
    if run_backup "$base" > refused-native-backup.log 2>&1; then
        echo 'Inventory backup accepted a stale snapshot after the owner refused to export.' >&2
        exit 1
    fi
    grep -q 'Inventory refused the snapshot' refused-native-backup.log
    test ! -e "$receipt"
    stop_service
    run_backup "$native_url" > native-backup.log
    archive="$work/backup-target/$backup_cap/$(jq -er '.tarball' "$receipt")"
    restored="$(dirname "$work")/native-restored"
    env -i PATH="$PATH" HOME="$HOME" SJEL_PERSONAL_ROOT="$work" SJEL_OVERLAY_ROOT="$work" \
        "$tools/restore.sh" "$backup_cap" "$archive" --receipt "$receipt" \
        --destination "$restored" > native-restore.log
    sh "$script_dir/inventory-backup-rehearsal.sh" "$snapshot_dir/inventory.db" \
        "$work/native-backup" "$inventory" > native-backup-checks.log
    stop_auxiliary
    cp "$restored/data/inventory/snapshots/inventory.db" rollback-to-sqlite.db
    start_service sqlite-after-native rollback-to-sqlite.db
    check_item 5
    request --fail "$base/api/inventory" |
        jq -e --arg item "$item" 'any(.[]; .item.id == $item and .item.label == "Acknowledged native API edit")' >/dev/null
    request --fail "$base/api/items/native-item/state" |
        jq -e 'map(.state) == ["wanted", "owned"] and .[1].note == "Native state transition"' >/dev/null
    request --fail "$base/api/item-refs/$native_import_ref" |
        jq -e '.revision == 1 and .state == "owned"' >/dev/null
    request --fail "$base/api/wish-comparison" | jq -S . > restored-wish-comparison.json
    request --fail "$base/api/wish-groups" | jq -S . > restored-wish-groups.json
    cmp -s native-wish-comparison.json restored-wish-comparison.json
    cmp -s native-wish-groups.json restored-wish-groups.json
    request --fail "$base/api/mail-proposals" | jq -S . > restored-mail.json
    request --fail "$base/api/item-refs/$native_ref/aliases" | jq -S . > restored-aliases.json
    cmp -s native-mail.json restored-mail.json
    cmp -s native-aliases.json restored-aliases.json
    stop_service
    test "$(sqlite3 -readonly rollback-to-sqlite.db 'PRAGMA integrity_check')" = ok
    test -z "$(sqlite3 -readonly rollback-to-sqlite.db 'PRAGMA foreign_key_check')"
    echo 'The backup producer rejected a stale snapshot and restored a fresh native snapshot through its receipt.'
    echo 'Native item, wishlist, alias and mail changes survived SQLite rollback.'
fi

test "$(shasum -a 256 "$source_db")" = "$source_sha"
echo 'Inventory served the restored revision, kept the Turso receipt, rejected a stale edit, and retained the new revision after restart.'

#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo 'usage: sh inventory-rehearsal.sh <restored database> <reviewed CLI> <new private scratch directory>' >&2
    exit 2
fi
source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
turso_cli=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
script_dir=$(cd "$(dirname "$0")" && pwd -P)
test "$(shasum -a 256 "$turso_cli" | awk '{print $1}')" = 764ddf8b552b7fa14c08abbc4d9e6d61934336f325e6b2210c81b24060217b09
test ! -s "$source_db-wal"
source_sha=$(shasum -a 256 "$source_db")
source_uri="file:$source_db?immutable=1"
mkdir -m 700 "$3"
cd "$3"
work=$(pwd -P)
mkdir db

test "$(sqlite3 -readonly "$source_uri" "SELECT count(*) FROM sqlite_schema s JOIN pragma_foreign_key_list(s.name) f WHERE s.type='table' AND (s.name GLOB 'inventory_*') <> (f.'table' GLOB 'inventory_*');")" = 0
sqlite3 -readonly "$source_uri" '.dump inventory_%' > inventory.sql
sqlite3 -bail db/before.db < inventory.sql
sqlite3 -readonly "$source_uri" "SELECT 'INSERT INTO sqlite_sequence VALUES(' || quote(name) || ',' || seq || ');' FROM sqlite_sequence WHERE name GLOB 'inventory_*' ORDER BY name;" > sequence.sql
{ echo 'DELETE FROM sqlite_sequence;'; cat sequence.sql; } | sqlite3 -bail db/before.db
sqlite3 -readonly db/before.db '.dump inventory_%' > extracted.sql
cmp inventory.sql extracted.sql
test "$(sqlite3 db/before.db 'PRAGMA integrity_check')" = ok
test -z "$(sqlite3 db/before.db 'PRAGMA foreign_key_check')"
printf 'Owner-only extraction matched for %s tables.\n' "$(grep -c '^CREATE TABLE' extracted.sql)"

run_turso() {
    docker run --rm -i --pull never --network none --read-only --cap-drop ALL \
        --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
        --env TMPDIR=/work --tmpfs /work:rw,noexec,nosuid,nodev,size=32m,mode=1777 \
        --mount "type=bind,src=$work/db,dst=/db" \
        --mount "type=bind,src=$turso_cli,dst=/tursodb,readonly" \
        debian:trixie-slim /tursodb --quiet --vfs syscall --output-mode list "/db/$1"
}

for engine in sqlite wal mvcc; do cp db/before.db "db/$engine.db"; done
sqlite3 -bail db/sqlite.db < "$script_dir/inventory-replay.sql" > sqlite.ack
if [ "$(cat sqlite.ack)" != '2
3
3|Acknowledged fixture|owned|2|3|sjel-turso-rehearsal-group|12999' ]; then
    echo 'SQLite control acknowledgements differ from the expected replay' >&2
    exit 1
fi
sqlite3 db/sqlite.db .dump > sqlite.after.sql

for engine in wal mvcc; do
    printf 'PRAGMA journal_mode=%s;\n' "$engine" | run_turso "$engine.db" > "$engine.mode" 2> "$engine.log"
    test "$(cat "$engine.mode")" = "$engine"
    if [ "$engine" = mvcc ]; then
        sed 's/^BEGIN IMMEDIATE;$/BEGIN CONCURRENT;/' "$script_dir/inventory-replay.sql" > mvcc-replay.sql
        mvcc_status=0
        run_turso mvcc.db < mvcc-replay.sql > mvcc.ack 2>> mvcc.log || mvcc_status=$?
        if [ "$mvcc_status" -ne 0 ]; then
            grep -q 'missing backing table for sequence' mvcc.ack mvcc.log
            echo 'MVCC: converted Inventory AUTOINCREMENT sequences failed; rollback parity is unproved.'
            continue
        fi
    else
        fk_status=0
        run_turso wal.db > wal-fk.out 2> wal-fk.log <<'SQL' || fk_status=$?
PRAGMA foreign_keys=ON;
INSERT INTO inventory_wish_group_item VALUES ('sjel-turso-missing-group', 'sjel-turso-missing-item');
SQL
        test "$fk_status" -ne 0
        grep -qi 'foreign key constraint failed' wal-fk.out wal-fk.log
        echo 'WAL rejected an orphan wish-group membership.'
        run_turso wal.db < "$script_dir/inventory-replay.sql" > wal.ack 2>> wal.log
    fi
    cmp sqlite.ack "$engine.ack"

    # Stop all engine processes before copying the latest acknowledged state.
    for suffix in '' -log -wal; do
        if [ -e "db/$engine.db$suffix" ]; then
            cp "db/$engine.db$suffix" "db/$engine-rollback.db$suffix"
        fi
    done
    if [ "$engine" = mvcc ]; then
        printf '%s\n' 'PRAGMA journal_mode=wal;' | run_turso mvcc-rollback.db > mvcc-rollback.mode 2>> mvcc.log
        test "$(cat mvcc-rollback.mode)" = wal
    fi
    sqlite3 -bail "db/$engine-rollback.db" ".backup db/$engine-restored.db"
    test "$(sqlite3 "db/$engine-restored.db" 'PRAGMA integrity_check')" = ok
    test -z "$(sqlite3 "db/$engine-restored.db" 'PRAGMA foreign_key_check')"
    sqlite3 "db/$engine-restored.db" .dump > "$engine.after.sql"
    cmp sqlite.after.sql "$engine.after.sql"
    printf '%s: acknowledgements, SQLite backup restore and complete logical contents matched.\n' "$engine"
done

run_turso fresh-mvcc.db > fresh-mvcc.ack 2> fresh-mvcc.log <<'SQL'
PRAGMA journal_mode=mvcc;
CREATE TABLE entries (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT);
INSERT INTO entries(label) VALUES ('first') RETURNING id;
DELETE FROM entries;
INSERT INTO entries(label) VALUES ('second') RETURNING id;
SQL
test "$(cat fresh-mvcc.ack)" = 'mvcc
1
2'
echo 'Fresh MVCC AUTOINCREMENT allocated increasing IDs after deletion.'
test "$(shasum -a 256 "$source_db")" = "$source_sha"
echo 'Restored source checksum is unchanged.'

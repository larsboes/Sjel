#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo 'usage: sh entities-rehearsal.sh <restored shared database> <reviewed tursodb binary> <new scratch directory>' >&2
    exit 2
fi
source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
turso_bin=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
test -f "$source_db"
test ! -s "$source_db-wal"
test -x "$turso_bin"
source_uri="file:$source_db?immutable=1"
source_sha=$(shasum -a 256 "$source_db")
mkdir -m 700 "$3"
cd "$3"
work=$(pwd -P)
mkdir db

cross_owner="SELECT count(*) FROM sqlite_schema s JOIN pragma_foreign_key_list(s.name) f
WHERE s.type='table' AND (s.name GLOB 'entities_*') <> (f.\"table\" GLOB 'entities_*');"
test "$(sqlite3 -readonly "$source_uri" "$cross_owner")" = 0
sqlite3 -readonly "$source_uri" '.dump entities_%' > owner.sql
sqlite3 -bail db/entities.db < owner.sql
indexes="SELECT sql || ';' FROM sqlite_schema WHERE type='index'
AND tbl_name GLOB 'entities_*' AND sql IS NOT NULL ORDER BY name;"
sqlite3 -readonly "$source_uri" "$indexes" > indexes.sql
sqlite3 -bail db/entities.db < indexes.sql
sqlite3 -readonly db/entities.db '.dump entities_%' > extracted.sql
cmp -s owner.sql extracted.sql
test "$(grep -c '^CREATE TABLE' extracted.sql)" = 9
test "$(sqlite3 -readonly db/entities.db "$indexes")" = "$(cat indexes.sql)"
test "$(sqlite3 -bail db/entities.db 'PRAGMA journal_mode=wal')" = wal

cp db/entities.db db/control.db
cp db/entities.db db/turso.db
cat > replay.sql <<'SQL'
PRAGMA foreign_keys=ON;
BEGIN IMMEDIATE;
INSERT INTO entities_entities(id,kind,name,revision,created_at,updated_at)
VALUES('sjel-entities-wal-rehearsal-only','person','Entities WAL rehearsal',1,'2026-01-01','2026-01-01');
INSERT INTO entities_values(entity_id,key,value,source,updated_at)
VALUES('sjel-entities-wal-rehearsal-only','company','"Fixture"','operator','2026-01-01');
UPDATE entities_entities SET revision=revision+1 WHERE id='sjel-entities-wal-rehearsal-only' AND revision=1;
COMMIT;
BEGIN IMMEDIATE;
UPDATE entities_entities SET revision=3 WHERE id='sjel-entities-wal-rehearsal-only';
ROLLBACK;
SELECT revision FROM entities_entities WHERE id='sjel-entities-wal-rehearsal-only';
SQL
sqlite3 -bail db/control.db < replay.sql > control.ack
docker run --rm -i --pull never --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
    --env TMPDIR=/work --tmpfs /work:rw,noexec,nosuid,nodev,size=32m,mode=1777 \
    --mount "type=bind,src=$work/db,dst=/db" \
    --mount "type=bind,src=$turso_bin,dst=/tursodb,readonly" \
    debian:trixie-slim /tursodb --quiet --vfs syscall --output-mode list /db/turso.db \
    < replay.sql > turso.ack
cmp -s control.ack turso.ack
test "$(cat turso.ack)" = 2
sqlite3 -bail db/turso.db '.backup db/restored.db'
restored_uri="file:$work/db/restored.db?immutable=1"
test "$(sqlite3 -readonly "$restored_uri" 'PRAGMA integrity_check')" = ok
test -z "$(sqlite3 -readonly "$restored_uri" 'PRAGMA foreign_key_check')"
sqlite3 -readonly "file:$work/db/control.db?immutable=1" .dump > control.sql
sqlite3 -readonly "$restored_uri" .dump > restored.sql
cmp -s control.sql restored.sql
test "$(shasum -a 256 "$source_db")" = "$source_sha"
rm owner.sql extracted.sql control.sql restored.sql
printf '%s\n' 'Nine Entities tables extracted. Turso WAL commit, rollback, and SQLite restore matched the control.'

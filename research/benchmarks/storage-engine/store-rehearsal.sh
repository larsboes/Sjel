#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo "usage: sh store-rehearsal.sh <restored database> <reviewed CLI> <new private scratch directory>" >&2
    exit 2
fi

source_db=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
turso_cli=$(cd "$(dirname "$2")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$2")")
test "$(shasum -a 256 "$turso_cli" | awk '{print $1}')" = 764ddf8b552b7fa14c08abbc4d9e6d61934336f325e6b2210c81b24060217b09
test ! -s "$source_db-wal"
source_sha=$(shasum -a 256 "$source_db")
mkdir -m 700 "$3"
cd "$3"
mkdir db
work=$(pwd -P)
source_uri="file:$source_db?immutable=1"

sqlite3 -readonly "$source_uri" "SELECT sql || ';' FROM sqlite_schema
    WHERE (type='table' AND name='documents_search')
       OR (type='trigger' AND tbl_name='documents_items' AND name LIKE 'documents_search_after_%')
    ORDER BY type, name;" > documents-search.sql
test "$(grep -ic 'USING fts5' documents-search.sql)" = 1
test "$(grep -ic 'CREATE TRIGGER' documents-search.sql)" = 3
sqlite3 -readonly "$source_uri" ".backup db/rehearsal.db"

sqlite3 -readonly "$source_uri" "SELECT 'SELECT ' || row_number() OVER (ORDER BY name) ||
        ', count(*) FROM ' || char(34) || replace(name,char(34),char(34)||char(34)) || char(34) || ';'
    FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%'
    ORDER BY name;" > counts.sql
sqlite3 -readonly "$source_uri" < counts.sql > expected.counts

sqlite3 db/rehearsal.db <<'SQL'
BEGIN;
DROP TRIGGER documents_search_after_insert;
DROP TRIGGER documents_search_after_update;
DROP TRIGGER documents_search_after_delete;
DROP TABLE documents_search;
COMMIT;
PRAGMA wal_checkpoint(TRUNCATE);
SQL

run_turso() {
    docker run --rm -i --pull never --network none --read-only --cap-drop ALL \
        --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
        --env TMPDIR=/work --tmpfs /work:rw,noexec,nosuid,nodev,size=128m,mode=1777 \
        --mount "type=bind,src=$work/db,dst=/db" \
        --mount "type=bind,src=$turso_cli,dst=/tursodb,readonly" \
        debian:trixie-slim /tursodb --quiet --vfs syscall --output-mode list "$@" /db/rehearsal.db
}

run_turso --readonly < counts.sql > turso.counts 2> turso.log
cmp expected.counts turso.counts
printf 'Turso table counts match SQLite for %s ordinary tables.\n' "$(wc -l < expected.counts | tr -d ' ')"

run_turso > write.out 2> write.log <<'SQL'
BEGIN IMMEDIATE;
CREATE TABLE sjel_turso_rehearsal_witness (value INTEGER NOT NULL);
INSERT INTO sjel_turso_rehearsal_witness VALUES (1);
COMMIT;
SQL
test "$(sqlite3 db/rehearsal.db 'SELECT value FROM sjel_turso_rehearsal_witness')" = 1
sqlite3 db/rehearsal.db <<'SQL'
BEGIN;
DROP TABLE sjel_turso_rehearsal_witness;
.read documents-search.sql
INSERT INTO documents_search(documents_search) VALUES ('rebuild');
INSERT INTO documents_search(documents_search,rank) VALUES ('integrity-check',1);
COMMIT;
SQL
sqlite3 db/rehearsal.db < counts.sql > rolled-back.counts
cmp expected.counts rolled-back.counts
test "$(sqlite3 db/rehearsal.db 'PRAGMA integrity_check')" = ok
test "$(shasum -a 256 "$source_db")" = "$source_sha"
echo 'SQLite reopen, FTS5 rebuild and integrity check passed. Source checksum is unchanged.'

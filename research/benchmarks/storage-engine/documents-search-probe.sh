#!/bin/sh
set -eu
umask 077

if [ "$#" -ne 2 ]; then
    echo 'usage: sh documents-search-probe.sh <absolute path to reviewed CLI> <new scratch directory>' >&2
    exit 2
fi
turso_cli=$1
test "$(shasum -a 256 "$turso_cli" | awk '{print $1}')" = 764ddf8b552b7fa14c08abbc4d9e6d61934336f325e6b2210c81b24060217b09
mkdir -m 700 "$2"
cd "$2"
work=$(pwd -P)

sqlite3 fixture.db <<'SQL'
CREATE TABLE documents_items (
    filename TEXT, extracted_title TEXT, extracted_text TEXT,
    category TEXT, project TEXT, status TEXT
);
INSERT INTO documents_items VALUES
    ('one.pdf', 'Rechnung März', NULL, 'Kauf', NULL, 'inbox'),
    ('two.pdf', 'Überweisung', NULL, 'Bank', NULL, 'filed'),
    ('three.pdf', NULL, 'Rechnungsprüfung', NULL, 'Steuer', 'inbox'),
    ('four.pdf', 'Garten', NULL, NULL, NULL, 'inbox'),
    ('five.pdf', 'Uberblick', NULL, NULL, NULL, 'inbox');
SQL
cp fixture.db turso.db
sqlite3 fixture.db <<'SQL'
CREATE VIRTUAL TABLE documents_search USING fts5(
    filename, extracted_title, extracted_text, category, project,
    content='documents_items', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO documents_search(documents_search) VALUES ('rebuild');
CREATE TABLE after_search_index (id INTEGER PRIMARY KEY);
INSERT INTO after_search_index VALUES (1);
SQL

run_turso() {
    docker run --rm -i --pull never --network none --read-only --cap-drop ALL \
        --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
        --env TMPDIR=/work --tmpfs /work:rw,noexec,nosuid,nodev,size=32m,mode=1777 \
        --mount "type=bind,src=$work,dst=/probe" \
        --mount "type=bind,src=$turso_cli,dst=/tursodb,readonly" \
        debian:trixie-slim /tursodb --quiet --vfs syscall --experimental-index-method \
        --output-mode list "$@"
}

catalog_status=0
printf '%s\n' 'SELECT count(*) FROM documents_items;' 'SELECT count(*) FROM after_search_index;' |
    run_turso --readonly /probe/fixture.db > catalog.out 2> catalog.log || catalog_status=$?
test "$catalog_status" -ne 0
test "$(awk '/^[0-9]+$/ {print}' catalog.out)" = 5
grep -q 'no such table: after_search_index' catalog.out catalog.log
echo 'An ordinary table after FTS5 is unavailable when Turso opens the legacy schema.'

printf '%s\n' 'CREATE INDEX documents_search ON documents_items USING fts (filename, extracted_title, extracted_text, category, project);' | run_turso /probe/turso.db
for query in 'rechnung' 'rechn*' 'uber*' 'über*' 'rechn* OR gart*'; do
    printf "SELECT d.rowid FROM documents_search JOIN documents_items d ON d.rowid = documents_search.rowid WHERE documents_search MATCH '%s' AND d.status = 'inbox' ORDER BY d.rowid;\n" "$query" | sqlite3 fixture.db > expected
    printf "SELECT rowid FROM documents_items WHERE fts_match(filename, extracted_title, extracted_text, category, project, '%s') AND status = 'inbox' ORDER BY rowid;\n" "$query" | run_turso /probe/turso.db > actual
    printf "WITH hits AS MATERIALIZED (SELECT rowid, status FROM documents_items WHERE fts_match(filename, extracted_title, extracted_text, category, project, '%s')) SELECT rowid FROM hits WHERE status = 'inbox' ORDER BY rowid;\n" "$query" | run_turso /probe/turso.db > staged
    printf '%s: SQLite=[%s] Turso=[%s] Turso-filter-after-search=[%s]\n' \
        "$query" "$(paste -sd , expected)" "$(paste -sd , actual)" "$(paste -sd , staged)"
done

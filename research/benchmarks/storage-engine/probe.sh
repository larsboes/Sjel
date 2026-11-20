#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: sh probe.sh <absolute fixture directory> <absolute path to reviewed Turso CLI>" >&2
    exit 2
fi

docker run --rm --pull never --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
    --env TMPDIR=/work --tmpfs /work:rw,noexec,nosuid,nodev,size=16m,mode=1777 \
    --mount "type=bind,src=$1,dst=/fixtures,readonly" \
    --mount "type=bind,src=$2,dst=/tursodb,readonly" \
    debian:trixie-slim sh -c '
query=$(cat /fixtures/query.sql)
for name in plain indexed plain-wal indexed-wal deindexed-wal; do
    for mode in readonly readwrite; do
        copy="/work/$name-$mode.db"
        cp "/fixtures/$name.db" "$copy"
        set --
        if [ "$mode" = readonly ]; then set -- --readonly; fi
        printf "\n%s %s:\n" "$name" "$mode"
        status=0
        /tursodb --quiet --vfs syscall --output-mode list "$@" "$copy" "$query" || status=$?
        printf "exit=%s\n" "$status"
        if cmp -s "/fixtures/$name.db" "$copy"; then echo bytes=unchanged; else echo bytes=changed; fi
    done
done

printf "\nvector bytes after libSQL removes the index:\n"
/tursodb --quiet --vfs syscall --readonly --output-mode list /work/deindexed-wal-readonly.db \
    "SELECT id, hex(embedding) FROM nodes ORDER BY id;" >/work/turso-vectors.txt
cmp /fixtures/expected-vectors.txt /work/turso-vectors.txt
printf "match\n"

printf "\nexperimental sparse index using the existing dense vectors:\n"
cp /fixtures/deindexed-wal.db /work/experimental.db
status=0
/tursodb --quiet --vfs syscall --experimental-index-method /work/experimental.db \
    "CREATE INDEX nodes_vector_idx ON nodes USING toy_vector_sparse_ivf (embedding);" || status=$?
printf "rebuild_exit=%s\n" "$status"

probe_recovery() {
    journal=$1
    begin=$2
    printf "\nwriter contention and process crash in %s:\n" "$journal"
    copy="/work/crash-$journal.db"
    fifo="/work/writer-$journal.sql"
    log="/work/writer-$journal.out"
    cp /fixtures/plain-wal.db "$copy"
    mkfifo "$fifo"
    /tursodb --quiet --vfs syscall --output-mode list "$copy" <"$fifo" >"$log" 2>&1 &
    writer=$!
    exec 3>"$fifo"
    printf "PRAGMA journal_mode = %s;\nCREATE TABLE recovery (id INTEGER PRIMARY KEY);\nINSERT INTO recovery VALUES (1);\n%s;\nINSERT INTO recovery VALUES (2);\nSELECT 987654321;\n" "$journal" "$begin" >&3
    attempt=0
    until grep -qx 987654321 "$log"; do
        attempt=$((attempt + 1))
        if [ "$attempt" -ge 50 ]; then cat "$log" >&2; kill "$writer"; exit 1; fi
        sleep 0.1
    done
    status=0
    timeout 5 /tursodb --quiet --vfs syscall "$copy" "INSERT INTO recovery VALUES (3);" || status=$?
    printf "contending_writer_exit=%s\n" "$status"
    kill -KILL "$writer"
    wait "$writer" 2>/dev/null || true
    exec 3>&-
    /tursodb --quiet --vfs syscall --output-mode list "$copy" "SELECT id FROM recovery ORDER BY id;"
}

probe_recovery wal "BEGIN IMMEDIATE"
probe_recovery mvcc "BEGIN CONCURRENT"

printf "\nTurso MVCC snapshot from an open connection:\n"
printf "%s\n" "PRAGMA journal_mode=mvcc;" "CREATE TABLE witness (id INTEGER PRIMARY KEY);" \
    "INSERT INTO witness VALUES (1);" "VACUUM INTO '\''/work/snapshot.db'\'';" \
    "INSERT INTO witness VALUES (2);" > /work/snapshot.sql
/tursodb --quiet --vfs syscall --output-mode list /work/snapshot-source.db < /work/snapshot.sql
check="SELECT count(*), max(id) FROM witness;"
test "$(/tursodb --quiet --vfs syscall --output-mode list /work/snapshot.db "$check")" = "1|1"
test "$(/tursodb --quiet --vfs syscall --output-mode list /work/snapshot-source.db "$check")" = "2|2"
echo "MVCC snapshot retained the first commit and excluded the later commit."
'

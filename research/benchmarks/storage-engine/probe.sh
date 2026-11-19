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
for name in plain indexed plain-wal indexed-wal; do
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

printf "\nwriter contention and process crash on plain-wal:\n"
cp /fixtures/plain-wal.db /work/crash.db
mkfifo /work/writer.sql
/tursodb --quiet --vfs syscall --output-mode list /work/crash.db </work/writer.sql >/work/writer.out 2>&1 &
writer=$!
exec 3>/work/writer.sql
printf "CREATE TABLE recovery (id INTEGER PRIMARY KEY);\nINSERT INTO recovery VALUES (1);\nBEGIN IMMEDIATE;\nINSERT INTO recovery VALUES (2);\nSELECT 987654321;\n" >&3
attempt=0
until grep -qx 987654321 /work/writer.out; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then cat /work/writer.out >&2; kill "$writer"; exit 1; fi
    sleep 0.1
done
status=0
timeout 5 /tursodb --quiet --vfs syscall /work/crash.db "INSERT INTO recovery VALUES (3);" || status=$?
printf "contending_writer_exit=%s\n" "$status"
kill -KILL "$writer"
wait "$writer" 2>/dev/null || true
exec 3>&-
/tursodb --quiet --vfs syscall --output-mode list /work/crash.db "SELECT id FROM recovery ORDER BY id;"
'

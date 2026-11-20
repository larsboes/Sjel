#!/bin/sh
# Rehearse a Turso version on copies of every owner's latest receipt-backed WAL backup.
# Each archive is restored in isolation; the probe writes through the candidate engine,
# and SQLite then checks integrity, foreign keys and an unchanged full dump.
set -eu
umask 077

if [ "$#" -ne 2 ]; then
    echo 'usage: sh engine-upgrade-rehearsal.sh <turso version> <new private scratch directory>' >&2
    exit 2
fi
version=$1
source_dir=$(cd "$(dirname "$0")" && pwd -P)
tools=$(cd "$source_dir/../../../tools" && pwd -P)
. "$tools/lib/paths.sh"
receipts="$SJEL_OVERLAY_ROOT/backup/receipts"
systems="$SJEL_OVERLAY_ROOT/config/systems.local.toml"
mkdir -m 700 "$2"
cd "$2"
work=$(pwd -P)

mkdir probe probe/src
cp "$source_dir/engine-upgrade-probe.rs" probe/src/main.rs
cat > probe/Cargo.toml <<EOF
[package]
name = "engine-upgrade-probe"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
turso = { version = "=$version", default-features = false }
tokio = { version = "1", features = ["macros", "rt"] }

[workspace]
EOF
(cd probe && CARGO_TARGET_DIR="$work/probe/target" cargo build --release --quiet)
probe="$work/probe/target/release/engine-upgrade-probe"

target_path() {
    awk -v section="[$1]" '$0 == section {found = 1; next} /^\[/ {found = 0}
        found && $1 == "path" {gsub(/"/, "", $3); print $3; exit}' "$systems"
}

failures=0
for receipt in "$receipts"/*-wal-backup.json; do
    cap=$(basename "$receipt" .json)
    archive="$(target_path "$(jq -er '.target' "$receipt")")/$cap/$(jq -er '.tarball' "$receipt")"
    restored="$work/$cap"
    "$tools/restore.sh" "$cap" "$archive" --receipt "$receipt" --destination "$restored" >/dev/null
    find "$restored" -name '*.db' -type f | while read -r db; do
        name="$cap:${db#"$restored"/}"
        sqlite3 -readonly "file:$db?immutable=1" .dump > "$db.before.sql"
        if ! "$probe" "$db" > "$db.probe.log" 2>&1; then
            echo "FAIL $name: the candidate engine could not write; see $db.probe.log"
            exit 1
        fi
        sqlite3 -readonly "file:$db?immutable=1" .dump > "$db.after.sql"
        checks=$(sqlite3 -readonly "file:$db?immutable=1" 'PRAGMA integrity_check; PRAGMA foreign_key_check;')
        if [ "$checks" != ok ]; then
            echo "FAIL $name: SQLite reports: $checks"
            exit 1
        fi
        if ! cmp -s "$db.before.sql" "$db.after.sql"; then
            echo "FAIL $name: the owner contents changed after the probe"
            exit 1
        fi
        echo "ok   $name: $(grep -c '^CREATE TABLE' "$db.after.sql") tables unchanged, integrity and foreign keys ok"
    done || failures=$((failures + 1))
done
[ "$failures" -eq 0 ] || { echo "$failures owner archive(s) failed with Turso $version"; exit 1; }
echo "Turso $version wrote to every restored owner file without changing its contents."

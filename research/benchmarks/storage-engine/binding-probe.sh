#!/bin/sh
set -eu

if ! { [ "$#" -eq 2 ] || { [ "$#" -eq 3 ] && [ "$3" = workload ]; } || { [ "$#" -eq 4 ] && [ "$3" = inventory ]; }; }; then
    echo "usage: sh binding-probe.sh <clean Turso v0.8.3 checkout> <new scratch directory> [workload | inventory <wal-restored.db>]" >&2
    exit 2
fi

upstream=$(cd "$1" && pwd -P)
source_dir=$(cd "$(dirname "$0")" && pwd -P)
if [ "$(git -C "$upstream" rev-parse HEAD)" != 607bff9fd0e547a25a97b3bf06e83bcc5d99d53c ] ||
    [ -n "$(git -C "$upstream" status --porcelain --untracked-files=no)" ]; then
    echo "expected the clean Turso v0.8.3 source revision" >&2
    exit 1
fi

if [ "${3:-}" = inventory ]; then
    inventory_source=$(cd "$(dirname "$4")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$4")")
    test ! -s "$inventory_source-wal"
fi

umask 077
mkdir -m 700 "$2"
cd "$2"
mkdir src .cargo
ln -s "$upstream" turso-source
cp "$upstream/Cargo.lock" Cargo.lock
cp "$upstream/.cargo/config.toml" .cargo/config.toml
cat > Cargo.toml <<'EOF'
[package]
name = "turso-binding-probe"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
turso = { path = "turso-source/bindings/rust", default-features = false }
EOF

if [ "${3:-}" = workload ]; then
    cp "$source_dir/workload-probe.rs" src/main.rs
    cat >> Cargo.toml <<'EOF'
libsql = { version = "=0.10.0", default-features = false, features = ["core"] }
tokio = { version = "=1.54.1", features = ["macros", "rt-multi-thread", "sync", "time"] }

[profile.release]
opt-level = 2
EOF
    rustup run stable cargo build --offline --release
    target_dir=${CARGO_TARGET_DIR:-target}
    "$target_dir/release/turso-binding-probe" "$PWD/databases"
    cd databases
    for database in turso-wal turso-mvcc-converted; do
        sqlite3 "$database.db" ".backup '$database-restored.db'"
        result=$(sqlite3 -readonly "file:$database-restored.db?immutable=1" \
            'PRAGMA quick_check; SELECT n, (SELECT count(*) FROM edges), (SELECT count(*) FROM embeddings) FROM counters;')
        if [ "$result" != "ok
800|84232|25000" ]; then
            echo "$database: unexpected backup results: $result" >&2
            exit 1
        fi
        echo "$database: SQLite backup restored with expected counts"
    done
    exit 0
fi

if [ "${3:-}" = inventory ]; then
    cp "$source_dir/inventory-binding-probe.rs" src/main.rs
    source_sha=$(shasum -a 256 "$inventory_source")
    cp "$inventory_source" owner.db
    chmod 600 owner.db
else
    cp "$source_dir/binding-probe.rs" src/main.rs
fi
cat >> Cargo.toml <<'EOF'
tokio = { version = "=1.54.1", features = ["macros", "rt"] }
EOF

rustup run stable cargo build --offline
target_dir=${CARGO_TARGET_DIR:-target}
if [ "${3:-}" = inventory ]; then
    "$target_dir/debug/turso-binding-probe" "$PWD/owner.db"
    sqlite3 -bail owner.db '.backup restored.db'
    read_edits() {
        sqlite3 -readonly "$1" "SELECT revision FROM inventory_item WHERE id='sjel-turso-rehearsal-item';
            SELECT revision FROM inventory_sync_operation WHERE operation_id='sjel-turso-native-op';
            SELECT revision FROM inventory_sync_operation WHERE operation_id='sjel-turso-backup-overlap';"
    }
    for database in online.db restored.db; do
        test ! -s "$database-wal"
        snapshot="file:$database?immutable=1"
        test "$(sqlite3 -readonly "$snapshot" 'PRAGMA integrity_check')" = ok
        test -z "$(sqlite3 -readonly "$snapshot" 'PRAGMA foreign_key_check')"
        test "$(sqlite3 -readonly "$snapshot" "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name GLOB 'inventory_*';")" = 10
    done
    test "$(read_edits 'file:online.db?immutable=1')" = '4
4'
    test "$(read_edits 'file:restored.db?immutable=1')" = '5
4
5'
    sequences='SELECT name, seq FROM sqlite_sequence ORDER BY name;'
    test "$(sqlite3 -readonly 'file:online.db?immutable=1' "$sequences")" = \
        "$(sqlite3 -readonly 'file:restored.db?immutable=1' "$sequences")"
    test "$(shasum -a 256 "$inventory_source")" = "$source_sha"
    echo 'SQLite verified the committed-only snapshot and restored both acknowledged revisions after shutdown.'
    exit 0
fi
"$target_dir/debug/turso-binding-probe" "$PWD/databases"
rustup run stable cargo build --offline --locked --target aarch64-apple-ios

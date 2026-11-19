#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: sh binding-probe.sh <clean Turso v0.8.3 checkout> <new scratch directory>" >&2
    exit 2
fi

upstream=$(cd "$1" && pwd -P)
source_dir=$(cd "$(dirname "$0")" && pwd -P)
if [ "$(git -C "$upstream" rev-parse HEAD)" != 607bff9fd0e547a25a97b3bf06e83bcc5d99d53c ] ||
    [ -n "$(git -C "$upstream" status --porcelain --untracked-files=no)" ]; then
    echo "expected the clean Turso v0.8.3 source revision" >&2
    exit 1
fi

mkdir -m 700 "$2"
cd "$2"
mkdir src .cargo
ln -s "$upstream" turso-source
cp "$upstream/Cargo.lock" Cargo.lock
cp "$upstream/.cargo/config.toml" .cargo/config.toml
cp "$source_dir/binding-probe.rs" src/main.rs
cat > Cargo.toml <<'EOF'
[package]
name = "turso-binding-probe"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
turso = { path = "turso-source/bindings/rust", default-features = false }
tokio = { version = "=1.54.1", features = ["macros", "rt"] }
EOF

rustup run stable cargo build --offline
target_dir=${CARGO_TARGET_DIR:-target}
"$target_dir/debug/turso-binding-probe" "$PWD/databases"
rustup run stable cargo build --offline --locked --target aarch64-apple-ios

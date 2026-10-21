#!/bin/bash
# tools/install-sqld.sh — install the pinned `sqld` (libsql-server) binary.
#
# `sqld` is the primary a phone (or a second Mac) holds an embedded replica of. It is not on
# Homebrew and not on crates.io: Turso publishes release binaries only, so nothing else on this
# machine would ever move it. This script is the one thing that does, and it is deliberately
# narrow — one version, one digest, one platform, each measured.
#
# WHY THE DIGEST IS A LITERAL HERE. A download that is verified against a digest fetched from the
# same place as the archive protects against a truncated transfer and nothing else: whoever can
# replace the archive can replace its checksum. The digest below came from the release page and was
# confirmed against the archive this machine installed (2026-10-21), so bumping it is a deliberate
# two-step: read the new page, then run the script and watch it fail if you mistyped.
#
# `sqld` 0.24.32 is from 2025-02-14 and upstream has been quiet since. It syncs correctly with the
# `libsql 0.10.0` crate that `sjel-turso` links (measured 2026-10-21: pull, forwarded write, second
# replica), which is the compatibility fact that matters and the reason this is not chasing a
# newer tag.
#
# Usage:
#   tools/install-sqld.sh            # install if missing or a different version
#   tools/install-sqld.sh --force    # reinstall even when the pinned version is present
#   tools/install-sqld.sh --check    # report the installed version; write nothing
#
# bash 3.2-safe (CONTRIBUTING.md#portable-shell).
set -euo pipefail

VERSION="0.24.32"
TAG="libsql-server-v${VERSION}"

# Measured 2026-10-21 against
# github.com/tursodatabase/libsql/releases/download/libsql-server-v0.24.32/
#   libsql-server-aarch64-apple-darwin.tar.xz
SHA256="ced2a9d65a5d4b6bd72c67e98ad6c63139e2a139d91769f07fdd15be935381dd"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ASSET="libsql-server-aarch64-apple-darwin.tar.xz" ;;
  Linux-x86_64) ASSET="libsql-server-x86_64-unknown-linux-gnu.tar.xz" ;;
  Linux-aarch64) ASSET="libsql-server-aarch64-unknown-linux-gnu.tar.xz" ;;
  *)
    echo "install-sqld: no measured release asset for $(uname -s)-$(uname -m)." >&2
    echo "Add its name and digest above from https://github.com/tursodatabase/libsql/releases/tag/$TAG — until then this machine cannot host a primary." >&2
    exit 2
    ;;
esac

BIN_DIR="${SJEL_BIN_DIR:-$HOME/.local/bin}"
TARGET="$BIN_DIR/sqld"
URL="https://github.com/tursodatabase/libsql/releases/download/$TAG/$ASSET"

installed_version() {
  [ -x "$TARGET" ] || return 0
  # `sqld --version` prints `sqld sqld 0.24.32 (40c272de 2025-02-14)`: the last field is a date,
  # so the version is the first bare dotted number rather than any fixed position.
  "$TARGET" --version 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+\.[0-9]+\.[0-9]+$/) { print $i; exit }}'
}

case "${1:-}" in
  "") ;;
  --force) FORCE=1 ;;
  --check)
    version="$(installed_version)"
    if [ -z "$version" ]; then
      echo "sqld: not installed (want $VERSION in $BIN_DIR)"
      exit 1
    fi
    echo "sqld: $version installed at $TARGET (pinned: $VERSION)"
    [ "$version" = "$VERSION" ] || exit 1
    exit 0
    ;;
  *) echo "usage: tools/install-sqld.sh [--force|--check]" >&2; exit 2 ;;
esac

if [ -z "${FORCE:-}" ] && [ "$(installed_version)" = "$VERSION" ]; then
  echo "sqld: $VERSION already installed at $TARGET — nothing to do"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "sqld: downloading $ASSET ($TAG)"
curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/asset.tar.xz" "$URL"

actual="$(shasum -a 256 "$tmp/asset.tar.xz" | awk '{print $1}')"
if [ "$actual" != "$SHA256" ]; then
  echo "install-sqld: digest mismatch — refusing to install." >&2
  echo "  expected $SHA256" >&2
  echo "  got      $actual" >&2
  echo "An archive that does not match the digest this repository pinned is not the release this" >&2
  echo "repository reviewed. If upstream re-cut $TAG, that is a decision, not an update." >&2
  exit 1
fi

tar -xJf "$tmp/asset.tar.xz" -C "$tmp"
staged="$tmp/libsql-server-${ASSET%.tar.xz}/sqld"
[ -f "$staged" ] || staged="$(find "$tmp" -type f -name sqld -maxdepth 3 | head -1)"
if [ ! -f "$staged" ]; then
  echo "install-sqld: the archive held no sqld binary" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
# Atomic replace: a half-written binary that a running server's launchd unit re-execs is worse
# than no upgrade at all.
install -m 0755 "$staged" "$tmp/sqld.new"
mv -f "$tmp/sqld.new" "$TARGET"

version="$(installed_version)"
echo "sqld: $version installed at $TARGET"
if [ "$version" != "$VERSION" ]; then
  echo "install-sqld: the binary reports $version but $VERSION was pinned — check the asset" >&2
  exit 1
fi

#!/bin/bash
# Builds visocr, the Apple Vision batch OCR reader rung 2 of the extraction
# ladder shells out to (libs/extraction/src/vision.rs).
#
# usage: tools/visocr/build.sh [--install <dir>]
#
# Default output is target/tools/visocr, which .gitignore already covers via
# `**/target/`. That is enough to run the corpus gate, because
# libs/extraction/src/vision.rs takes SJEL_VISOCR_BIN as a full path. `--install
# <dir>` copies it to a directory on PATH instead, for a caller that resolves the
# bare name. visocr has no toolchain.toml entry: that file declares what a
# machine must have, and no capability execs this binary yet — see the comment
# where the entry would stand.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$TOOLS_DIR/lib/paths.sh"      # SJEL_ROOT
source "$TOOLS_DIR/lib/platform.sh"   # SJEL_OS, from this machine's machine.toml

INSTALL_DIR=""
case "$#:${1:-}" in
  0:) ;;
  2:--install) INSTALL_DIR="$2" ;;
  *) echo "usage: tools/visocr/build.sh [--install <dir>]" >&2; exit 1 ;;
esac

# Refused rather than attempted. Vision is an operating-system framework, not a
# library that can be vendored, so there is nothing to fall back to here — and
# the ladder is built to survive that: rung 2 reports itself unavailable and the
# walk continues (upstreams.toml [ocrs] names the Linux case).
if [ "$SJEL_OS" != "macos" ]; then
  echo "visocr: this machine declares os = \"$SJEL_OS\"; Apple Vision is a macOS framework and has no port." >&2
  echo "        Rung 2 of the extraction ladder is simply absent here, and libs/extraction says so at runtime." >&2
  exit 1
fi

if ! command -v xcrun >/dev/null 2>&1; then
  echo "visocr: xcrun not found. Install the Xcode Command Line Tools: xcode-select --install" >&2
  exit 1
fi

# CARGO_TARGET_DIR is honoured so the Swift binary lands beside the Rust output when the
# target dir is relocated; same pattern as tools/storage/storage.
OUT_DIR="${CARGO_TARGET_DIR:-$SJEL_ROOT/target}/tools"
mkdir -p "$OUT_DIR"
xcrun swiftc -O "$TOOLS_DIR/visocr/visocr.swift" -o "$OUT_DIR/visocr"
echo "visocr: built $OUT_DIR/visocr"

if [ -n "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR"
  cp "$OUT_DIR/visocr" "$INSTALL_DIR/visocr"
  echo "visocr: installed $INSTALL_DIR/visocr"
  command -v visocr >/dev/null 2>&1 || \
    echo "visocr: $INSTALL_DIR is not on PATH, so a caller resolving the bare name still misses it; point SJEL_VISOCR_BIN at $INSTALL_DIR/visocr instead" >&2
fi

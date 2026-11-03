#!/bin/bash
# Re-renders the frozen corpus pages from their Typst sources.
#
# DELIBERATE, NOT AUTOMATIC. The PNGs beside this script are committed bytes,
# and every result under ../results/ scored THOSE bytes. Running this replaces
# them, which invalidates every earlier result: a font that resolves differently
# on another machine changes glyph shapes, and an OCR score compared across two
# different renderings is not a comparison at all. Nothing in `cargo test` or in
# `cargo run --bin extraction-gate` calls this.
#
# Run it only when a judgement changes, and say so in the run record that
# follows, with the typst version that produced the new bytes.
#
# typst is the pinned renderer (upstreams.toml [typst]). 150 ppi and grayscale
# because the corpus is text on white: colour and a higher density cost bytes in
# a tracked directory and buy an OCR engine nothing.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PPI=150

command -v typst >/dev/null 2>&1 || { echo "render.sh: typst not on PATH (brew install typst)" >&2; exit 1; }
command -v sips  >/dev/null 2>&1 || { echo "render.sh: sips not on PATH (macOS built-in)" >&2; exit 1; }

echo "render.sh: $(typst --version)"
for source in "$HERE"/*.typ; do
  name="$(basename "$source" .typ)"
  typst compile --ppi "$PPI" "$source" "$HERE/$name.png"
  # Text on white needs no colour channels. sips rewrites in place.
  sips --setProperty format png --matchTo '/System/Library/ColorSync/Profiles/Generic Gray Profile.icc' \
       "$HERE/$name.png" --out "$HERE/$name.png" >/dev/null
  echo "  $name.png  $(wc -c < "$HERE/$name.png" | tr -d ' ') bytes"
done

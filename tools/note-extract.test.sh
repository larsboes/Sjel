#!/bin/bash
# Synthetic archive decoding and read-only CLI checks.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PYTHONDONTWRITEBYTECODE=1 uv run --no-project python -m unittest discover \
  -s "$ROOT/crates/extraction/tests" -p 'test_notability.py'

#!/bin/bash
# Synthetic plugin inventory fixtures; no vault or network access.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
uv run --no-project "$ROOT/tools/vault-plugin-audit.test.py"

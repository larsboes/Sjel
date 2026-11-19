#!/usr/bin/env bash
# check-work.sh — validate canonical task, roadmap and release records.
set -euo pipefail
exec bun --no-env-file "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/work.ts" check

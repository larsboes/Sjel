#!/bin/bash
# check-doc-links.sh — relative-link gate for Markdown (CI: repo gates).
# The logic is in check-doc-links.ts; this entry point keeps the gate registry's shape.
set -e
exec bun "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-doc-links.ts" "$@"

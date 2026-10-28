#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
SCRATCH="$(mktemp -d /tmp/sjel-vault-plugins-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/overlay/config" "$SCRATCH/vault/.obsidian/plugins"

if env -u VAULT_ROOT SJEL_OVERLAY_ROOT="$SCRATCH/overlay" SJEL_PERSONAL_ROOT="$SCRATCH/overlay" \
  "$ROOT/tools/vault-plugins" status >"$SCRATCH/out" 2>"$SCRATCH/err"; then
  echo 'FAIL: unresolved vault must fail' >&2
  exit 1
fi
grep -q 'missing .*knowledge.toml' "$SCRATCH/err"
if grep -q 'Vault root:' "$SCRATCH/out"; then
  echo 'FAIL: unresolved vault selected a fallback' >&2
  exit 1
fi

VAULT_ROOT="$SCRATCH/vault" SJEL_OVERLAY_ROOT="$SCRATCH/overlay" \
  "$ROOT/tools/vault-plugins" status >"$SCRATCH/out"
grep -qF "Vault root: $SCRATCH/vault" "$SCRATCH/out"

printf 'vault_root = "%s"\n' "$SCRATCH/vault" >"$SCRATCH/overlay/config/knowledge.toml"
env -u VAULT_ROOT SJEL_OVERLAY_ROOT="$SCRATCH/overlay" \
  "$ROOT/tools/vault-plugins" status >"$SCRATCH/out"
grep -qF "Vault root: $SCRATCH/vault" "$SCRATCH/out"
echo 'vault-plugins: resolver checks passed'

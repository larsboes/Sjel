#!/bin/bash
# A verified prebuilt storage binary bypasses compilation and preserves the CLI contract.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/tools/storage" "$scratch/tools/lib" "$scratch/bin"
cp "$ROOT/tools/storage/storage" "$scratch/tools/storage/storage"
printf 'SJEL_ROOT=%s\n' "$scratch" > "$scratch/tools/lib/paths.sh"
cat > "$scratch/bin/cargo" <<'SH'
#!/bin/sh
exit 99
SH
cat > "$scratch/storage" <<'SH'
#!/bin/sh
printf '<%s>\n' "$@"
exit 7
SH
chmod +x "$scratch/bin/cargo" "$scratch/storage"
status=0
out="$(PATH="$scratch/bin:$PATH" SJEL_STORAGE_BIN="$scratch/storage" bash "$scratch/tools/storage/storage" report 'two words' 2>&1)" || status=$?
[ "$status" -eq 7 ]
[ "$out" = "$(printf '<report>\n<two words>')" ]
for invalid in "$scratch/missing" "$scratch/bin" "$scratch/tools/lib/paths.sh"; do
  status=0
  out="$(PATH="$scratch/bin:$PATH" SJEL_STORAGE_BIN="$invalid" bash "$scratch/tools/storage/storage" 2>&1)" || status=$?
  [ "$status" -eq 2 ]
  case "$out" in *'not an executable file'*) ;; *) exit 1 ;; esac
done
echo 'storage launcher: prebuilt arguments, exit status and invalid overrides pass'

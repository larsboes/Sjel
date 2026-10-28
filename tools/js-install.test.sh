#!/bin/bash
# Regression tests for tools/js-install. A fake `bun` on PATH records its arguments and
# environment, so no case touches the network or a real tree.
set -uo pipefail

TOOL="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/js-install"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

BIN="$SCRATCH/bin"; mkdir -p "$BIN"
LOG="$SCRATCH/bun.log"
cat >"$BIN/bun" <<'FAKE'
#!/bin/bash
[ "${1:-}" = "--version" ] && { echo "9.9.9"; exit 0; }
echo "args=$* registry=${BUN_CONFIG_REGISTRY:-none}" >>"$FAKE_LOG"
case "${FAKE_MODE:-ok}" in
  rewrite) echo '{"rewritten": true}' >bun.lock ;;
  unparseable) echo "warn: Ignoring lockfile" >&2 ;;
  fail) echo "boom" >&2; exit 1 ;;
esac
exit 0
FAKE
chmod +x "$BIN/bun"
export PATH="$BIN:$PATH" FAKE_LOG="$LOG"

TREE="$SCRATCH/tree"; mkdir -p "$TREE"
echo '{}' >"$TREE/bun.lock"
OVERLAY="$SCRATCH/overlay"; mkdir -p "$OVERLAY/config"

fails=0
check() { if eval "$2"; then :; else echo "FAIL: $1"; fails=$((fails + 1)); fi; }

: >"$LOG"
SJEL_OVERLAY_ROOT="$OVERLAY" "$TOOL" "$TREE" >/dev/null 2>&1
check "no mirror: frozen lockfile, default registry" \
  "grep -q 'args=install --frozen-lockfile --ignore-scripts registry=none' '$LOG'"

printf 'npm = "https://mirror.example/npm/"\n' >"$OVERLAY/config/registries.toml"
: >"$LOG"
SJEL_OVERLAY_ROOT="$OVERLAY" "$TOOL" "$TREE" >/dev/null 2>&1
check "mirror: --no-save through the overlay's registry" \
  "grep -q 'args=install --no-save --ignore-scripts registry=https://mirror.example/npm/' '$LOG'"

SJEL_OVERLAY_ROOT="$OVERLAY" FAKE_MODE=rewrite "$TOOL" "$TREE" >/dev/null 2>&1
check "a rewritten lockfile fails" "[ $? -ne 0 ]"
echo '{}' >"$TREE/bun.lock"

SJEL_OVERLAY_ROOT="$OVERLAY" FAKE_MODE=unparseable "$TOOL" "$TREE" >/dev/null 2>&1
check "an unreadable lockfile fails rather than resolving afresh" "[ $? -ne 0 ]"

SJEL_OVERLAY_ROOT="$OVERLAY" FAKE_MODE=fail "$TOOL" "$TREE" >/dev/null 2>&1
check "a failed install fails" "[ $? -ne 0 ]"

NOLOCK="$SCRATCH/nolock"; mkdir -p "$NOLOCK"
: >"$LOG"
SJEL_OVERLAY_ROOT="$OVERLAY" "$TOOL" "$NOLOCK" >/dev/null 2>&1
check "a tree without bun.lock is refused" "[ $? -ne 0 ] && [ ! -s '$LOG' ]"

SJEL_OVERLAY_ROOT="$OVERLAY" "$TOOL" "$TREE" >/dev/null 2>&1
check "a clean install passes" "[ $? -eq 0 ]"

if [ "$fails" -gt 0 ]; then
  echo "✗ tools/js-install: $fails case(s) failed"
  exit 1
fi
echo "✓ tools/js-install: all cases pass"

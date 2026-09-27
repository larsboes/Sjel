#!/bin/bash
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SJEL_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"
SCRIPT="$TOOLS_DIR/agent-integrations.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export SJEL_TEST_UV_LOG="$tmp/uv.log"
export PATH="$tmp/bin:/usr/bin:/bin:/usr/sbin:/sbin"
mkdir -p "$HOME/.config/opencode" "$tmp/bin"

cat > "$tmp/bin/uv" <<'STUB'
#!/bin/bash
set -euo pipefail
printf '%s|%s\n' "$PWD" "$*" >> "$SJEL_TEST_UV_LOG"
if [ "${SJEL_TEST_UV_FAIL:-0}" = 1 ]; then
  exit 19
fi
platform="${*: -1}"
if [ "$platform" = opencode ]; then
  mkdir -p .opencode/skills/graphify .opencode/plugins
  printf '%s\n' '// graphify fixture skill' > .opencode/skills/graphify/.keep
  printf '%s\n' '// graphify fixture plugin' > .opencode/plugins/graphify.js
fi
STUB
chmod +x "$tmp/bin/uv"

cat > "$tmp/bin/opencode" <<'STUB'
#!/bin/bash
if [ "$1" = "--version" ] || [ "$1" = "-V" ]; then
  echo "opencode 0.0.0"
else
  exit 0
fi
STUB
chmod +x "$tmp/bin/opencode"

# No version is read from anywhere, and that is the assertion: upstreams.toml carries no
# `pin` since Q77 (2026-09-02), so the installer is driven at whatever `uv tool run
# --from graphifyy` resolves today.
if grep -q '^pin = ' "$SJEL_ROOT/upstreams.toml"; then
  echo "upstreams.toml still declares a pin — this script must not resolve one" >&2
  exit 1
fi

list_output="$($SCRIPT list)"
case "$list_output" in
  *"Configured on this machine:"*) ;;
  *) echo "list did not distinguish configured state" >&2; exit 1 ;;
esac

machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|runnable|' || {
  echo "status --machine did not report opencode as runnable" >&2
  exit 1
}

$SCRIPT install opencode
plugin="$HOME/.config/opencode/plugins/graphify.js"
test -s "$plugin"
grep -q 'graphify' "$plugin"
grep -Fq "tool run --from graphifyy graphify install --platform opencode" "$SJEL_TEST_UV_LOG"

scratch="$(head -n 1 "$SJEL_TEST_UV_LOG" | cut -d '|' -f 1)"
test "$scratch" != "$SJEL_ROOT"
test ! -e "$scratch"

first="$(cksum "$plugin")"
$SCRIPT install opencode
test "$(cksum "$plugin")" = "$first"

machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|integrated|' || {
  echo "status --machine did not report opencode as integrated after install" >&2
  exit 1
}

# The marker records the DATE the files were taken from upstream, so it must be one.
marker="$HOME/.config/opencode/.graphify-upstream-installed"
grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' "$marker" || {
  echo "the install marker does not carry an install date: $(cat "$marker")" >&2
  exit 1
}

# Installed files with no marker are of unknown provenance — an older mechanism, a hand copy,
# or an install that died before writing it — and re-running the installer is the answer to
# all three. That is what 'stale' means since Q77; it compared against a pin before.
rm -f "$marker"
machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|stale|' || {
  echo "status --machine did not report unmarked integration files as stale" >&2
  exit 1
}

printf '%s\n' '// known-good graphify plugin' > "$plugin"
before="$(cksum "$plugin")"
if SJEL_TEST_UV_FAIL=1 $SCRIPT install opencode >/dev/null 2>&1; then
  echo "failed upstream install unexpectedly succeeded" >&2
  exit 1
fi
test "$(cksum "$plugin")" = "$before"

if $SCRIPT install unknown-harness >/dev/null 2>&1; then
  echo "unknown harness unexpectedly succeeded" >&2
  exit 1
fi

echo "agent-integrations tests passed"

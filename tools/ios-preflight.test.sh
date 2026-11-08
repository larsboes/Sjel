#!/bin/bash
# iOS launchers check device access before compiling or reading signing settings.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/home"

cat > "$work/bin/uname" <<'EOF'
#!/bin/bash
echo Darwin
EOF
cat > "$work/bin/xcrun" <<'EOF'
#!/bin/bash
echo "xcrun $*" >> "$TEST_LOG"
case "$TEST_DEVICES" in
  blocked) echo 'device service connection refused' >&2; exit 1 ;;
  empty) echo '== Devices ==' ;;
  available) echo '    iPhone (00000000-0000-0000-0000-000000000001) (Shutdown)' ;;
esac
EOF
cat > "$work/bin/cargo" <<'EOF'
#!/bin/bash
echo 'cargo' >> "$TEST_LOG"
exit 47
EOF
cat > "$work/bin/xcodegen" <<'EOF'
#!/bin/bash
echo 'xcodegen' >> "$TEST_LOG"
exit 48
EOF
chmod +x "$work/bin/"*
export PATH="$work/bin:$PATH" HOME="$work/home" TEST_LOG="$work/commands"

run() {
  : > "$TEST_LOG"
  status=0
  output="$(SJEL_IOS_SIM="${selected:-}" bash "$root/apps/gpui/ios/$1" "${@:2}" 2>&1)" || status=$?
}

expect() {
  if [ "$status" -ne "$1" ] || ! printf '%s\n' "$output" | grep -Fq "$2"; then
    echo "FAIL: expected exit $1 and '$2', got exit $status: $output" >&2
    exit 1
  fi
  if grep -Eq '^(cargo|xcodegen)$' "$TEST_LOG"; then
    echo 'FAIL: device preflight started a build' >&2
    exit 1
  fi
}

export TEST_DEVICES=blocked
run run-sim
expect 1 'CoreSimulator is unavailable'
run run-device 00000000-0000-0000-0000-000000000001
expect 1 'device access failed'
grep -Fq -- '--timeout 10' "$TEST_LOG"

export TEST_DEVICES=empty
run run-sim
expect 1 'no available iPhone simulator'

export TEST_DEVICES=available
selected=00000000-0000-0000-0000-000000000002
run run-sim
expect 1 'selected simulator is unavailable'

selected=
run run-sim
[ "$status" -eq 47 ] || { echo "FAIL: an available simulator did not reach Cargo: $output" >&2; exit 1; }
[ "$(head -1 "$TEST_LOG")" = 'xcrun simctl list devices available' ]
[ "$(tail -1 "$TEST_LOG")" = cargo ]

echo 'iOS preflight: blocked services, missing devices, invalid selection and build ordering passed'

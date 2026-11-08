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
  available) printf '%s\n' '-- iOS 27.0 --' '    iPhone (00000000-0000-0000-0000-000000000001) (Shutdown)' ;;
  mixed) printf '%s\n' '-- iOS 26.2 --' '    iPhone (00000000-0000-0000-0000-000000000002) (Shutdown)' '-- iOS 27.0 --' '    iPhone (00000000-0000-0000-0000-000000000003) (Shutdown)' '-- iOS 27.2 --' '    iPhone (00000000-0000-0000-0000-000000000001) (Shutdown)' ;;
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
export IPHONEOS_DEPLOYMENT_TARGET=17.0
launcher_root="$root"

run() {
  : > "$TEST_LOG"
  status=0
  output="$(SJEL_IOS_SIM="${selected:-}" bash "$launcher_root/apps/gpui/ios/$1" "${@:2}" 2>&1)" || status=$?
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

export TEST_DEVICES=mixed
selected=00000000-0000-0000-0000-000000000003
run run-sim
expect 1 'newest supported iOS runtime'
selected=
run run-sim
[ "$status" -eq 47 ] || { echo "FAIL: newest runtime did not reach Cargo: $output" >&2; exit 1; }

launcher_root="$work/project"
mkdir -p "$launcher_root/apps/gpui/ios/host"
cp "$root/apps/gpui/ios/run-sim" "$launcher_root/apps/gpui/ios/run-sim"
cp "$root/apps/gpui/ios/host/project.yml" "$launcher_root/apps/gpui/ios/host/project.yml"
cat > "$work/bin/cargo" <<'EOF'
#!/bin/bash
if [ "$IPHONEOS_DEPLOYMENT_TARGET" != 27.0 ]; then echo 'wrong iOS deployment target' >&2; exit 66; fi
if [ "$1" = metadata ]; then printf '{"target_directory":"%s"}\n' "$HOME/target"; fi
EOF
cat > "$work/bin/xcodegen" <<'EOF'
#!/bin/bash
exit 0
EOF
cat > "$work/bin/xcodebuild" <<'EOF'
#!/bin/bash
echo "xcodebuild $*" >> "$TEST_LOG"
previous=
for arg in "$@"; do
  if [ "$previous" = -resultBundlePath ]; then result="$arg"; mkdir -p "$arg"; fi
  previous="$arg"
done
if [ "$previous" = test ]; then
  if [ "$TEST_XCODE_STATUS" = light-fails ]; then
    case "$result" in */light/*) exit 65 ;; *) exit 0 ;; esac
  fi
  exit "$TEST_XCODE_STATUS"
fi
EOF
cat > "$work/bin/xcrun" <<'EOF'
#!/bin/bash
echo "xcrun $*" >> "$TEST_LOG"
if [ "$1" = simctl ] && [ "$2" = list ]; then
  echo '-- iOS 27.2 --'
  echo '    iPhone (00000000-0000-0000-0000-000000000001) (Booted)'
elif [ "$1" = simctl ] && [ "$2" = ui ] && [ "$#" -eq 4 ]; then
  echo light
elif [ "$1" = xcresulttool ]; then
  previous=
  for arg in "$@"; do
    if [ "$previous" = --output-path ]; then mkdir -p "$arg"; touch "$arg/screen.png"; fi
    previous="$arg"
  done
fi
EOF
chmod +x "$work/bin/"*
export SJEL_IOS_RESULTS="$work/results" TEST_XCODE_STATUS=65
run run-sim --smoke
[ "$status" -eq 65 ] || { echo "FAIL: a failed screen test lost its status: $output" >&2; exit 1; }
[ -f "$SJEL_IOS_RESULTS/light/screenshots/screen.png" ]
[ -f "$SJEL_IOS_RESULTS/dark/screenshots/screen.png" ]
grep -Fq -- '-only-testing:SjelIOSUITests/NshScreens' "$TEST_LOG"
grep -Fq -- 'appearance dark' "$TEST_LOG"
[ "$(tail -1 "$TEST_LOG")" = 'xcrun simctl ui 00000000-0000-0000-0000-000000000001 appearance light' ]

export TEST_XCODE_STATUS=light-fails
run run-sim --smoke
[ "$status" -eq 65 ] || { echo 'FAIL: a passing dark run hid a failed light run' >&2; exit 1; }

export TEST_XCODE_STATUS=0
run run-sim --smoke
[ "$status" -eq 0 ] || { echo "FAIL: successful smoke test failed: $output" >&2; exit 1; }
run run-sim --tap
[ "$status" -eq 0 ] || { echo "FAIL: full touch suite failed: $output" >&2; exit 1; }
grep -Fq -- '-only-testing:SjelIOSUITests ' "$TEST_LOG"

echo 'iOS preflight: device access, build ordering, smoke selection and failure artifacts passed'

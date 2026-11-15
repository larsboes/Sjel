#!/bin/bash
# Exercise desktop build selection and unchanged installation without compiling or launching.
# SJEL_TEST_NATIVE_CODESIGN=1 verifies real ad-hoc signatures on macOS.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export FIXTURE="$work"
mkdir -p "$work/repo/apps/gpui/Resources" "$work/repo/tools/lib" "$work/bin" "$work/tmp"
# Keep platform registration inside the fixture on every host.
sed "s|/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister|$work/bin/lsregister|" \
  "$root/apps/gpui/install" > "$work/repo/apps/gpui/install"
cp "$root/tools/lib/local-build.sh" "$root/tools/lib/replace-directory.sh" "$work/repo/tools/lib/"
printf 'icon\n' > "$work/repo/apps/gpui/Resources/AppIcon.icns"
printf 'notices\n' > "$work/repo/apps/gpui/Resources/MapLibre-LICENSES.txt"
printf '#!/bin/sh\nexit 0\n' > "$work/artifact"
chmod +x "$work/artifact"
if [ "${SJEL_TEST_NATIVE_CODESIGN:-0}" = 1 ]; then
  [ "$(uname -s)" = Darwin ] || { echo 'native codesign tests require macOS' >&2; exit 2; }
  cp /usr/bin/true "$work/artifact"
fi
cat > "$work/repo/tools/lib/paths.sh" <<'SH'
SJEL_OVERLAY_ROOT="$FIXTURE/overlay"
toml_get_in() { printf '27.0\n'; }
SH
cat > "$work/bin/uname" <<'SH'
#!/bin/sh
printf 'Darwin\n'
SH
cat > "$work/bin/profile-cli" <<'SH'
#!/bin/sh
exit "${ADMISSION:-3}"
SH
cat > "$work/bin/git" <<'SH'
#!/bin/sh
case "$*" in
  *rev-parse*) echo abc1234 ;;
  *describe*) echo "${FIXTURE_VERSION:-v0.0.1}" ;;
  *) exit 1 ;;
esac
SH
cat > "$work/bin/cargo" <<'SH'
#!/bin/bash
set -eu
if [ "$1" = metadata ]; then
  printf '{"target_directory":"%s"}\n' "$CARGO_TARGET_DIR"
  exit 0
fi
printf 'cargo %s jobs=%s tests=%s\n' "$*" "${CARGO_BUILD_JOBS:-unset}" "${RUST_TEST_THREADS:-unset}" >> "$FIXTURE/calls"
profile=""
previous=""
for arg in "$@"; do
  [ "$previous" != --profile ] || profile="$arg"
  previous="$arg"
done
[ -n "$profile" ]
mkdir -p "$CARGO_TARGET_DIR/$profile"
cp "$FIXTURE/artifact" "$CARGO_TARGET_DIR/$profile/sjel-gpui"
SH
cat > "$work/bin/codesign" <<'SH'
#!/bin/bash
set -eu
bundle="${!#}"
if [ "${SJEL_TEST_NATIVE_CODESIGN:-0}" = 1 ]; then
  if [ "$1" = --force ]; then
    echo sign >> "$FIXTURE/calls"
    [ "${FAIL_SIGN:-0}" != 1 ] || exit 1
    exec /usr/bin/codesign --force --sign - --identifier dev.sjel.desktop --options runtime "$bundle"
  fi
  exec /usr/bin/codesign "$@"
fi
signature() { (cd "$1" && find Contents -type f -exec shasum -a 256 {} \; | LC_ALL=C sort); }
if [ "$1" = --force ]; then
  echo sign >> "$FIXTURE/calls"
  [ "${FAIL_SIGN:-0}" != 1 ] || exit 1
  signature "$bundle" > "$bundle/.test-signature"
else
  [ -f "$bundle/.test-signature" ] || exit 1
  actual="$(signature "$bundle")"
  [ "$actual" = "$(< "$bundle/.test-signature")" ]
fi
SH
cat > "$work/bin/osascript" <<'SH'
#!/bin/sh
echo osascript >> "$FIXTURE/calls"
case "$*" in
  *'is running'*) if [ -f "$FIXTURE/running" ]; then echo true; else echo false; fi ;;
  *'to quit'*) rm -f "$FIXTURE/running" ;;
esac
SH
cat > "$work/bin/open" <<'SH'
#!/bin/sh
echo open >> "$FIXTURE/calls"
SH
cat > "$work/bin/lsregister" <<'SH'
#!/bin/sh
echo register >> "$FIXTURE/calls"
SH
chmod +x "$work/bin/"*
export PATH="$work/bin:$PATH" TMPDIR="$work/tmp" SJEL_CLI_BIN="$work/bin/profile-cli"
export CARGO_TARGET_DIR="$work/cache" SJEL_DESKTOP_APP_DEST="$work/Installed.app"
export SJEL_CODESIGN_IDENTITY=fixture-identity
unset SJEL_ALLOW_LOCAL_BUILD CARGO_BUILD_JOBS RUST_TEST_THREADS SJEL_DESKTOP_BUILD_PROFILE
unset SJEL_DESKTOP_PREBUILT SJEL_DESKTOP_PREBUILT_COMMIT SJEL_GPUI_NATIVE_MAPS SJEL_BUILD_MESSAGE_FORMAT
installer="$work/repo/apps/gpui/install"
# A signing lookup in build-only mode is an unintended side effect.
cat > "$work/bin/security" <<'SH'
#!/bin/sh
echo security >> "$FIXTURE/calls"
exit 1
SH
chmod +x "$work/bin/security"
fail() { echo "FAIL: $*" >&2; exit 1; }
run() { bash "$installer" "$@" > "$work/output" 2>&1 || { read_output; fail 'installer failed'; }; }
read_output() { while IFS= read -r line; do printf '%s\n' "$line" >&2; done < "$work/output"; }
contains() { grep -qF -- "$2" "$1" || fail "missing $2 in $1"; }
SJEL_CODESIGN_IDENTITY='' run --build-only
contains "$work/output" 'built (desktop-dev)'
[ ! -e "$SJEL_DESKTOP_APP_DEST" ] || fail 'build-only installed a bundle'
! grep -qE '^(security|sign|register|osascript|open)$' "$work/calls" || fail 'build-only touched signing or app lifecycle'
run
contains "$work/calls" 'build --locked --profile desktop-dev -p sjel-gpui jobs=2 tests=2'
contains "$SJEL_DESKTOP_APP_DEST/Contents/Info.plist" '<key>SJEL_DESKTOP_BUILD_PROFILE</key><string>desktop-dev</string>'
cp "$work/calls" "$work/before"
touch "$work/running"
run
contains "$work/output" 'unchanged (desktop-dev)'
[ -f "$work/running" ] || fail 'unchanged install quit the app'
[ "$(grep -c '^sign$' "$work/calls")" = 1 ] || fail 'unchanged install signed again'
[ "$(grep -c '^register$' "$work/calls")" = 1 ] || fail 'unchanged install registered again'
! grep -q '^open$' "$work/calls" || fail 'unchanged install opened the app'
if [ "${SJEL_TEST_NATIVE_CODESIGN:-0}" = 1 ]; then
  cp /usr/bin/false "$work/artifact"
else
  printf '# changed\n' >> "$work/artifact"
fi
run
[ ! -f "$work/running" ] || fail 'changed install did not quit the old app'
contains "$work/calls" open
# Tampering must invalidate a matching input receipt.
printf 'tampered\n' > "$SJEL_DESKTOP_APP_DEST/Contents/Resources/AppIcon.icns"
run
cmp -s "$work/repo/apps/gpui/Resources/AppIcon.icns" "$SJEL_DESKTOP_APP_DEST/Contents/Resources/AppIcon.icns" || fail 'tampered bundle was kept'
printf 'new icon\n' > "$work/repo/apps/gpui/Resources/AppIcon.icns"
run --no-restart
cmp -s "$work/repo/apps/gpui/Resources/AppIcon.icns" "$SJEL_DESKTOP_APP_DEST/Contents/Resources/AppIcon.icns" || fail 'changed resource was ignored'
FIXTURE_VERSION=v0.0.2 run --no-restart
contains "$SJEL_DESKTOP_APP_DEST/Contents/Info.plist" '<key>CFBundleVersion</key><string>v0.0.2</string>'
run --release --native-maps --no-restart
contains "$work/calls" 'build --locked --profile release -p sjel-gpui --features native-maps'
contains "$SJEL_DESKTOP_APP_DEST/Contents/Info.plist" '<key>SJEL_DESKTOP_BUILD_PROFILE</key><string>release</string>'
[ -f "$SJEL_DESKTOP_APP_DEST/Contents/Resources/MapLibre-LICENSES.txt" ] || fail 'native notices missing'
run --dev --no-restart
[ ! -f "$SJEL_DESKTOP_APP_DEST/Contents/Resources/MapLibre-LICENSES.txt" ] || fail 'obsolete resources retained'
SJEL_CODESIGN_IDENTITY=other-identity run --dev --no-restart
! grep -qF unchanged "$work/output" || fail 'changed signing identity was ignored'
# A failed signing step must not replace the installed bundle.
cp "$SJEL_DESKTOP_APP_DEST/Contents/Resources/BuildInputs.sha256" "$work/previous-receipt"
if FAIL_SIGN=1 bash "$installer" --release --no-restart > "$work/output" 2>&1; then fail 'failed signing succeeded'; fi
cmp -s "$work/previous-receipt" "$SJEL_DESKTOP_APP_DEST/Contents/Resources/BuildInputs.sha256" || fail 'failed signing replaced installed bundle'
# Prebuilt artifacts do not need Cargo or build admission.
builds="$(grep -c '^cargo ' "$work/calls")"
SJEL_DESKTOP_PREBUILT="$work/artifact" SJEL_DESKTOP_PREBUILT_COMMIT=prebuilt ADMISSION=2 run --no-restart
[ "$(grep -c '^cargo ' "$work/calls")" = "$builds" ] || fail 'prebuilt packaging invoked Cargo'
contains "$SJEL_DESKTOP_APP_DEST/Contents/Info.plist" '<key>SjelBuildCommit</key><string>prebuilt</string>'
contains "$SJEL_DESKTOP_APP_DEST/Contents/Info.plist" '<key>SJEL_DESKTOP_BUILD_PROFILE</key><string>release</string>'
if SJEL_DESKTOP_BUILD_PROFILE=invalid bash "$installer" > "$work/output" 2>&1; then fail 'invalid profile accepted'; fi
if ADMISSION=2 bash "$installer" > "$work/output" 2>&1; then fail 'unknown profile compiled'; fi
[ "$(grep -c '^cargo ' "$work/calls")" = "$builds" ] || fail 'unknown profile invoked Cargo'
mkdir -p "$CARGO_TARGET_DIR/desktop-dev/.fingerprint"
if CARGO_TARGET_DIR="$CARGO_TARGET_DIR/desktop-dev" bash "$installer" --build-only > "$work/output" 2>&1; then
  fail 'a profile directory was accepted as a target root'
fi
contains "$work/output" 'not a cache root'
rmdir "$CARGO_TARGET_DIR/desktop-dev/.fingerprint"
touch "$CARGO_TARGET_DIR/desktop-dev/.cargo-lock"
if CARGO_TARGET_DIR="$CARGO_TARGET_DIR/desktop-dev" bash "$installer" --build-only > "$work/output" 2>&1; then
  fail 'a profile without .fingerprint was accepted as a target root'
fi
contains "$work/output" 'not a cache root'
[ "$(grep -c '^cargo ' "$work/calls")" = "$builds" ] || fail 'an invalid cache root invoked compilation'
echo 'gpui-install.test.sh: build profiles, battery defaults, no-op integrity and failure preservation passed'

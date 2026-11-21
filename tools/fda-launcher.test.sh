#!/bin/bash
# Isolated helper installation, access gating, and rollback; no real signing or launchd.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOL="$ROOT/tools/fda-launcher/install"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export HOME="$SCRATCH/home & space"
export SJEL_OVERLAY_ROOT="$SCRATCH/overlay"
export SJEL_PERSONAL_ROOT="$SJEL_OVERLAY_ROOT"
export SJEL_ROOT="$ROOT"
export CARGO_TARGET_DIR="$SCRATCH/target"
export SJEL_ALLOW_LOCAL_BUILD=1
export MOCK_ROOT="$SCRATCH"
export PATH="$SCRATCH/mock:$PATH"
mkdir -p "$SCRATCH/mock" "$HOME/Library/Mail" "$HOME/Library/LaunchAgents" "$SJEL_OVERLAY_ROOT/config" "$SJEL_OVERLAY_ROOT/bin"
printf 'os = "macos"\ncontainer_runtime = "docker"\ncapabilities = []\n' > "$SJEL_OVERLAY_ROOT/config/machine.toml"
fail() { echo "FAIL: $*" >&2; exit 1; }

cat > "$SCRATCH/mock/uname" <<'SH'
#!/bin/bash
echo Darwin
SH
cat > "$SCRATCH/mock/security" <<'SH'
#!/bin/bash
[ ! -f "$MOCK_ROOT/no-identity" ] || exit 0
printf '  1) 0123456789012345678901234567890123456789 "Test signing identity"\n'
SH
cat > "$SCRATCH/mock/codesign" <<'SH'
#!/bin/bash
case "$1" in
  -dv)
    case "$2" in
      *.app) echo 'Identifier=dev.sjel.access' >&2 ;;
      *) echo 'Identifier=dev.sjel.fda-launcher' >&2 ;;
    esac
    printf '%131072s\n' '' >&2
    ;;
  --force) [ ! -f "$MOCK_ROOT/sign-failure" ] ;;
  --verify) exit 0 ;;
  *) exit 2 ;;
esac
SH
cat > "$SCRATCH/mock/cargo" <<'SH'
#!/bin/bash
[ ! -f "$MOCK_ROOT/build-failure" ] || exit 17
mkdir -p "$CARGO_TARGET_DIR/release"
printf '#!/bin/bash\nexec "$@"\n' > "$CARGO_TARGET_DIR/release/sjel-fda-launcher"
chmod +x "$CARGO_TARGET_DIR/release/sjel-fda-launcher"
echo build >> "$MOCK_ROOT/builds"
SH
cat > "$SCRATCH/mock/plutil" <<'PY'
#!/usr/bin/env -S uv run --script
import plistlib
from pathlib import Path
import sys
args = sys.argv[1:]
path = Path(args[-1])
data = plistlib.loads(path.read_bytes())
if args[0] == "-extract":
    key = args[1]
    if key == "Program":
        if "Program" not in data:
            sys.exit(1)
        print(data["Program"])
    else:
        print(data["ProgramArguments"][0])
else:
    data["ProgramArguments"][0] = args[3]
    path.write_bytes(plistlib.dumps(data))
PY
cat > "$SCRATCH/mock/sleep" <<'SH'
#!/bin/bash
exit 0
SH
cat > "$SCRATCH/mock/launchctl" <<'SH'
#!/bin/bash
case "$1" in
  submit)
    [ ! -f "$MOCK_ROOT/probe-submit-failure" ] || exit 1
    [ ! -f "$MOCK_ROOT/probe-not-started" ] || exit 0
    shift
    while [ "$1" != -- ]; do
      case "$1" in -o) out="$2" ;; -e) err="$2" ;; esac
      shift 2
    done
    shift
    if [ -f "$MOCK_ROOT/deny-access" ]; then
      printf 'sjel-fda-probe-started\nsjel-fda-access-denied\n' > "$out"
      exit 0
    fi
    "$@" > "$out" 2> "$err"
    ;;
  print)
    if [ "${2##*/}" = com.apple.user-only ]; then
      case "$2" in user/*) ;; *) exit 1 ;; esac
      printf 'program = /usr/libexec/test-agent\n'
    elif [ "${2##*/}" = old.custom.label ]; then
      printf 'program = %s\n' "$(cat "$MOCK_ROOT/extra-loaded")"
    else
      printf 'program = %s\n' "$(cat "$MOCK_ROOT/loaded")"
    fi
    ;;
  list)
    printf 'PID\tStatus\tLabel\n1\t0\tcom.sjel.example\n'
    [ ! -f "$MOCK_ROOT/extra-loaded" ] || printf '2\t0\told.custom.label\n'
    [ ! -f "$MOCK_ROOT/user-domain-job" ] || printf '3\t0\tcom.apple.user-only\n'
    ;;
  load) plutil -extract ProgramArguments.0 raw -o - "$2" > "$MOCK_ROOT/loaded" ;;
  remove|unload) exit 0 ;;
  *) exit 2 ;;
esac
SH
cat > "$SCRATCH/mock/cli" <<'SH'
#!/bin/bash
[ "$1" = service-runner.sh ] && [ "$2" = install-persistence ] || exit 2
[ "${SJEL_FDA_MIGRATING:-}" = 1 ] || exit 2
[ "$("$TEST_TOOL" path)" = "$EXPECTED_HELPER" ] || exit 2
if [ -f "$MOCK_ROOT/migration-failure" ]; then
  echo broken > "$HOME/Library/LaunchAgents/com.sjel.example.plist"
  exit 1
fi
plutil -replace ProgramArguments.0 -string "$EXPECTED_HELPER" "$HOME/Library/LaunchAgents/com.sjel.example.plist"
printf '%s\n' "$EXPECTED_HELPER" > "$MOCK_ROOT/loaded"
SH
chmod +x "$SCRATCH/mock/"*
export SJEL_CLI_BIN="$SCRATCH/mock/cli"
export TEST_TOOL="$TOOL"
BUNDLE="$HOME/Library/Application Support/Sjel/helpers/Sjel.app"
export EXPECTED_HELPER="$BUNDLE/Contents/MacOS/sjel-fda-launcher"
FLAT="${BUNDLE%/*}/sjel-fda-launcher"
[ "$("$TOOL" app-path)" = "$BUNDLE" ] || fail "managed app path must not depend on overlay"
if "$TOOL" status >/dev/null; then fail "absent helper reported current"; fi
"$TOOL" install
"$TOOL" status
"$TOOL" install
[ "$(wc -l < "$SCRATCH/builds" | tr -d ' ')" = 1 ] || fail "current helper rebuilt"
"$TOOL" verify
[ -f "$BUNDLE/Contents/Info.plist" ] || fail "app metadata missing"
[ -f "$BUNDLE/Contents/Resources/AppIcon.icns" ] || fail "app icon missing"
if "$TOOL" path > /dev/null 2>&1; then fail "fresh install routed jobs before migration"; fi

# A modified or unmanaged installation is not silently overwritten.
echo changed >> "$EXPECTED_HELPER"
if "$TOOL" install >/dev/null 2>&1; then fail "modified helper overwritten"; fi
rm -rf "$BUNDLE"
rm "$BUNDLE.receipt"
mkdir -p "$BUNDLE"
if "$TOOL" install >/dev/null 2>&1; then fail "unmanaged helper overwritten"; fi
rmdir "$BUNDLE"
ln -s "$SCRATCH/no-target" "$BUNDLE"
if "$TOOL" install >/dev/null 2>&1; then fail "helper symlink overwritten"; fi
rm "$BUNDLE"
touch "$SCRATCH/no-identity"
if "$TOOL" install >/dev/null 2>&1; then fail "ad-hoc signing accepted"; fi
[ ! -e "$EXPECTED_HELPER" ] || fail "unsigned helper installed"
rm "$SCRATCH/no-identity"
touch "$SCRATCH/sign-failure"
if "$TOOL" install >/dev/null 2>&1; then fail "signing failure accepted"; fi
[ ! -e "$EXPECTED_HELPER" ] || fail "failed signature installed"
rm "$SCRATCH/sign-failure"
"$TOOL" install

# Use a real plist parser, but fake the service manager and all signing/build operations.
legacy="$SJEL_OVERLAY_ROOT/bin/sjel-fda-launcher"
cp "$EXPECTED_HELPER" "$legacy"
cp "$EXPECTED_HELPER" "$FLAT"
[ "$("$TOOL" path)" = "$FLAT" ] || fail "install changed routing before proof"
plist="$HOME/Library/LaunchAgents/com.sjel.example.plist"
printf '<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>com.sjel.example</string><key>ProgramArguments</key><array><string>placeholder</string></array></dict></plist>\n' > "$plist"
plutil -replace ProgramArguments.0 -string "$legacy" "$plist"
printf '%s\n' "$legacy" > "$SCRATCH/loaded"
touch "$SCRATCH/deny-access"
if "$TOOL" migrate > "$SCRATCH/probe-result" 2>&1; then fail "unproved access migrated"; fi
[ -f "$legacy" ] || fail "old helper deleted before proof"
[ "$(plutil -extract ProgramArguments.0 raw -o - "$plist")" = "$legacy" ] || fail "job changed before proof"
grep -Fq 'access was denied' "$SCRATCH/probe-result" || fail "access denial not distinguished"
rm "$SCRATCH/deny-access"
touch "$SCRATCH/probe-submit-failure"
if "$TOOL" migrate > "$SCRATCH/submit-result" 2>&1; then fail "failed launchd submission migrated"; fi
grep -Fq 'launchd could not start the probe' "$SCRATCH/submit-result" || fail "submission failure was silent"
[ "$("$TOOL" path)" = "$FLAT" ] || fail "routing changed after failed submission"
rm "$SCRATCH/probe-submit-failure"
touch "$SCRATCH/probe-not-started"
if "$TOOL" migrate > "$SCRATCH/probe-not-started-result" 2>&1; then fail "unfinished probe migrated"; fi
grep -Fq 'not known to be denied' "$SCRATCH/probe-not-started-result" || fail "unfinished probe mistaken for denial"
[ -f "$legacy" ] || fail "old helper deleted after unfinished probe"
[ "$(plutil -extract ProgramArguments.0 raw -o - "$plist")" = "$legacy" ] || fail "job changed after unfinished probe"
rm "$SCRATCH/probe-not-started"
touch "$SCRATCH/migration-failure"
if "$TOOL" migrate > "$SCRATCH/migration-result" 2>&1; then fail "failed migration accepted"; fi
[ -f "$legacy" ] || fail "old helper deleted after failed migration"
[ "$(plutil -extract ProgramArguments.0 raw -o - "$plist")" = "$legacy" ] || fail "job not restored after failed migration"
rm "$SCRATCH/migration-failure"
# A Program-only job and a loaded non-Sjel label must each block deletion.
other="$HOME/Library/LaunchAgents/other.consumer.plist"
printf '<?xml version="1.0"?><plist version="1.0"><dict><key>Program</key><string>placeholder</string></dict></plist>\n' > "$other"
uv run --no-project python -c 'import plistlib,sys; p=sys.argv[1]; d=plistlib.load(open(p,"rb")); d["Program"]=sys.argv[2]; plistlib.dump(d,open(p,"wb"))' "$other" "$legacy"
if "$TOOL" migrate > "$SCRATCH/program-only-result" 2>&1; then fail "Program-only consumer ignored"; fi
[ -f "$legacy" ] || fail "old helper deleted with Program-only consumer"
[ "$(plutil -extract ProgramArguments.0 raw -o - "$plist")" = "$legacy" ] || fail "job was not rolled back when another consumer blocked migration"
rm "$other"
printf '%s\n' "$legacy" > "$SCRATCH/extra-loaded"
if "$TOOL" migrate > "$SCRATCH/extra-loaded-result" 2>&1; then fail "loaded non-Sjel consumer ignored"; fi
[ -f "$legacy" ] || fail "old helper deleted with loaded non-Sjel consumer"
rm "$SCRATCH/extra-loaded"
plutil -replace ProgramArguments.0 -string "$FLAT" "$plist"
printf '%s\n' "$FLAT" > "$SCRATCH/loaded"
touch "$SCRATCH/user-domain-job"
"$TOOL" migrate
[ "$("$TOOL" path)" = "$EXPECTED_HELPER" ] || fail "verified app not selected for future jobs"
[ -x "$FLAT" ] || fail "previous managed helper was not retained"
[ ! -e "$legacy" ] || fail "overlay helper retained after verified migration"
[ "$(plutil -extract ProgramArguments.0 raw -o - "$plist")" = "$EXPECTED_HELPER" ] || fail "job did not move"
"$TOOL" migrate
printf 'FDA helper lifecycle tests passed\n'

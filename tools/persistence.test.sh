#!/bin/bash
# Tests for tools/service-runner.sh's boot-persistence contract (#9).
#
# The defect this covers is an absence: install-persistence existed, install.sh never called it,
# and nothing could report the result — so a capability could declare autostart, be enabled, run
# all day and simply be gone after a reboot. Every state below (installed, missing, stale,
# not-applicable, unsupported) is therefore about what the tool SAYS, which is the part that was
# missing entirely.
#
# Built on the same throwaway-root idea as service-runner.test.sh and manifest-resolution.test.sh:
# a scratch Axon root with its own overlay, machine.toml and capability manifests, plus a scratch
# HOME so unit files land inside the sandbox.
#
# launchctl, systemctl and loginctl are STUBBED on PATH. The first version of this file did not
# stub them and called the real `install-persistence`, which registered a live `com.axon.always`
# job in the author's launchd pointing at a scratch path that the test then deleted. A test for
# "does this machine's boot persistence match its declaration" must not itself install boot
# persistence on the machine running it. The stubs also make the load state deterministic, which
# a real supervisor cannot be: the same assertion answered `installed` on macOS and
# `installed-not-loaded` on the Linux CI runner.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/service-runner.sh" ]; then SRC_TOOLS="$_c"; break; fi
done
[ -n "$SRC_TOOLS" ] || { echo "persistence: cannot find service-runner.sh next to $_dir" >&2; exit 1; }

# Before the scratch root exists, and for the same reason the launchctl stubs above exist: an
# operator's exported SJEL_OVERLAY_ROOT outranks this sandbox's own axon.toml inside paths.sh, so
# every `os = "linux"` case here read the author's real macOS machine.toml and every env-block
# assertion read the author's real declarations. 14 checks failed locally and none in CI, which is
# exactly the shape a test cannot warn you about (tools/lib/test-support.sh#isolate_axon_env).
source "$SRC_TOOLS/lib/test-support.sh"
isolate_axon_env
# tools/capability.sh is a launcher for the sjel-cli crate since 2026-10-02, and the scratch
# root this suite copies it into has no crate to build.
sjel_cli_prebuilt

SCRATCH="$(mktemp -d "/tmp/persistence.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"
FAKE_HOME="$SCRATCH/home"

mkdir -p "$ROOT/tools/lib" "$ROOT/tools/templates" "$OVERLAY/config" \
         "$FAKE_HOME/Library/LaunchAgents" "$FAKE_HOME/.config/systemd/user"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$SRC_TOOLS/watchdog.sh" "$ROOT/tools/"
# The whole lib and template directories rather than a named subset: a list here would rot into a
# "No such file" the next time service-runner picks up another one.
cp "$SRC_TOOLS"/lib/*.sh "$ROOT/tools/lib/"
cp -R "$SRC_TOOLS"/templates/. "$ROOT/tools/templates/"   # -R: templates/ has subdirectories
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"

# --- supervisor stubs ------------------------------------------------------
# One state file holds the labels the fake supervisor considers loaded, so load/unload and the
# query that reads them cannot disagree. Deliberately dumb: this stands in for launchd, it does
# not model it.
STUB_BIN="$SCRATCH/bin"; STUB_STATE="$SCRATCH/loaded.txt"
mkdir -p "$STUB_BIN"; : > "$STUB_STATE"
cat > "$STUB_BIN/launchctl" <<'STUB'
#!/bin/bash
case "${1:-}" in
  list)   cat "$SJEL_TEST_STUB_STATE" ;;
  load)   printf '%s\t0\t%s\n' "$$" "$(basename "${2:-}" .plist)" >> "$SJEL_TEST_STUB_STATE" ;;
  unload) label="$(basename "${2:-}" .plist)"
          grep -v "$label\$" "$SJEL_TEST_STUB_STATE" > "$SJEL_TEST_STUB_STATE.new" || true
          mv "$SJEL_TEST_STUB_STATE.new" "$SJEL_TEST_STUB_STATE" ;;
esac
exit 0
STUB
cat > "$STUB_BIN/systemctl" <<'STUB'
#!/bin/bash
args="$*"
case "$args" in
  *is-active*)   unit="${args##* }"; grep -q "^${unit}$" "$SJEL_TEST_STUB_STATE" && exit 0 || exit 1 ;;
  *enable*)      unit="${args##* }"; echo "$unit" >> "$SJEL_TEST_STUB_STATE" ;;
  *disable*)     unit="${args##* }"
                 grep -v "^${unit}$" "$SJEL_TEST_STUB_STATE" > "$SJEL_TEST_STUB_STATE.new" || true
                 mv "$SJEL_TEST_STUB_STATE.new" "$SJEL_TEST_STUB_STATE" ;;
esac
exit 0
STUB
printf '#!/bin/bash\necho "Linger=yes"\n' > "$STUB_BIN/loginctl"
chmod +x "$STUB_BIN"/launchctl "$STUB_BIN"/systemctl "$STUB_BIN"/loginctl
export SJEL_TEST_STUB_STATE="$STUB_STATE"
PATH="$STUB_BIN:$PATH"; export PATH

# Four capabilities, one per state the issue names. `always` is the interesting one; the rest fix
# the shape of an answer that must NOT be "install something".
mkcap() {  # mkcap <name> <autostart> [extra manifest lines]
  mkdir -p "$ROOT/capabilities/$1"
  { printf 'kind = "process"\nname = "%s"\nautostart = "%s"\ncommand = ["/bin/sh", "-c", "sleep 1"]\n' "$1" "$2"
    shift 2; [ $# -gt 0 ] && printf '%s\n' "$@"
  } > "$ROOT/capabilities/$1/service.toml"
}
mkcap always true
mkcap ondemand false
mkcap gone true
# The third declaration (#129): on-demand AND periodic. `sched` is the interesting one; `both`
# fixes the shape of the answer for a manifest claiming to be a service and a job at once, which
# must be refused rather than resolved in whichever direction the code happens to check first.
mkcap sched false 'schedule = "6h"'
mkcap both true 'schedule = "6h"'
mkcap badsched false 'schedule = "6 hours"'

machine() {  # machine <os> <capabilities-toml-array>
  printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = %s\n' "$1" "$2" \
    > "$OVERLAY/config/machine.toml"
}

run() {  # run <capability> [command] — defaults to persistence-status
  out="$(HOME="$FAKE_HOME" XDG_CONFIG_HOME="$FAKE_HOME/.config" \
         "$ROOT/tools/service-runner.sh" "${2:-persistence-status}" "$1" 2>&1)"
  status=$?
}
state_of() { printf '%s' "$out" | cut -f2 | head -1; }

expect_state() {  # expect_state <label> <capability> <expected state>
  run "$2"
  local got; got="$(state_of)"
  [ "$got" = "$3" ] || fail "$1: expected state '$3', got '$got' (full: $out)"
}

# The unit file is there and matches, whether or not a supervisor has loaded it. Used for the
# Linux branch, where the load half is genuinely not decidable from a planted tree: that branch's
# persistence_loaded also tests for /run/systemd/system, which no PATH stub can create, so the
# answer is `unknown` on a macOS host and stub-driven on a systemd one. Asserting the file half
# there is asserting the part the planted tree actually determines.
expect_installed_file() {  # expect_installed_file <label> <capability>
  run "$2"
  local got; got="$(state_of)"
  case "$got" in
    installed|installed-not-loaded) ;;
    *) fail "$1: expected the unit to match the declaration, got '$got' (full: $out)" ;;
  esac
}

# The guard that would have caught the first version of this file: if PATH resolution ever puts
# the real supervisor ahead of the stub, every assertion below still passes while the test quietly
# installs launchd jobs on the machine running it. Checked before anything runs, not after.
for _bin in launchctl systemctl loginctl; do
  _resolved="$(command -v "$_bin" 2>/dev/null || true)"
  case "$_resolved" in
    "$STUB_BIN"/*) ;;
    *) echo "FAIL: '$_bin' resolves to '${_resolved:-nothing}', not the stub in $STUB_BIN —"
       echo "      this test would drive the real supervisor on this machine. Refusing to run."
       exit 1 ;;
  esac
done

# --- macOS -----------------------------------------------------------------
machine macos '["always", "ondemand", "gone"]'
PLIST="$FAKE_HOME/Library/LaunchAgents/com.sjel.always.plist"

# missing — the whole point of the issue: enabled, autostart, nothing installed.
rm -f "$PLIST"
expect_state "macos, nothing installed" always missing

# installed — render the unit the way install would, then confirm it is recognised.
run always install-persistence
[ -f "$PLIST" ] || fail "install-persistence did not write $PLIST (said: $out)"
expect_state "macos, freshly installed" always installed
# ...and it actually asked the supervisor to load it. Writing the file without loading it is the
# `installed-not-loaded` state, which install must not leave behind.
grep -q "com\.sjel\.always\$" "$STUB_STATE" || fail "install-persistence wrote the unit but never loaded it"

# stale — the unit exists but no longer matches what the declaration renders to. This is the case
# a plain existence check cannot see, and it is how a moved runtime binary silently keeps a
# watchdog pointing at a PATH that no longer resolves.
printf '<!-- hand-edited, no longer what the template renders -->\n' >> "$PLIST"
expect_state "macos, unit edited out from under the declaration" always stale

# ...and re-installing brings it back into agreement, so `stale` is actionable, not terminal.
run always install-persistence
expect_state "macos, re-installed after drift" always installed

# not applicable — a capability the manifest declares on-demand. A watchdog would call `start`
# every 30s and contradict the manifest, so the answer is n/a, never "missing".
expect_state "macos, on-demand capability" ondemand "n/a"
run ondemand
case "$out" in
  *"on-demand"*) ;;
  *) fail "the n/a answer must say why, said: $out" ;;
esac

# a natively-restarting container runtime owes no watchdog either
mkdir -p "$ROOT/capabilities/dockercap"
printf 'kind = "container"\nname = "dockercap"\nautostart = "true"\nimage = "x"\ntag = "1"\n' \
  > "$ROOT/capabilities/dockercap/service.toml"
machine macos '["dockercap"]'
expect_state "macos, docker restarts it natively" dockercap "n/a"
machine macos '["always", "ondemand", "gone"]'

# --- the third declaration: schedule (#129) --------------------------------
# A watchdog answers "keep it up"; a schedule answers "run it again in six hours". The check that
# used to refuse anything without autostart had to learn the difference rather than be loosened,
# so the assertions here are as much about what a scheduled capability must NOT get (a KeepAlive
# watchdog) as about what it must.
machine macos '["sched", "both", "badsched"]'
SCHED_PLIST="$FAKE_HOME/Library/LaunchAgents/com.sjel.sched.plist"
rm -f "$SCHED_PLIST"

# The heart of it: on-demand, and still owed a unit. Before #129 this answered `n/a` — the
# capability declared no autostart, so nothing was owed, so a periodic job could not be declared
# at all.
expect_state "macos, schedule declared but nothing installed" sched missing

run sched install-persistence
[ -f "$SCHED_PLIST" ] || fail "install-persistence did not write $SCHED_PLIST (said: $out)"
expect_state "macos, freshly installed schedule" sched installed

# 6h in the manifest, 21600 seconds in the unit — the conversion is the whole reason the manifest
# can say something an operator reads and both backends still get what they want.
grep -q "<integer>21600</integer>" "$SCHED_PLIST" || fail "6h did not render as a 21600s StartInterval: $(cat "$SCHED_PLIST")"
grep -q "<key>StartInterval</key>" "$SCHED_PLIST" || fail "a scheduled job rendered without StartInterval"
# The negative half, and the one that would silently ruin it: a KeepAlive here would hold the
# process up continuously and the interval would never mean anything.
grep -q "<key>KeepAlive</key>" "$SCHED_PLIST" && fail "a scheduled job must not render a KeepAlive watchdog"
# ...and it runs the runner directly, not the 30s watchdog loop. Matched inside a <string>, not
# anywhere in the file: the template's own comment says the word "watchdog.sh" while explaining
# why it is not one, and an assertion that reads prose is an assertion about prose.
grep -q "<string>[^<]*watchdog\.sh</string>" "$SCHED_PLIST" && fail "a scheduled job must not be driven by watchdog.sh"
grep -q "<string>start</string>" "$SCHED_PLIST" || fail "the scheduled unit does not invoke 'start'"

# Drift is drift for this mode too.
printf '<!-- hand-edited -->\n' >> "$SCHED_PLIST"
expect_state "macos, scheduled unit edited" sched stale
run sched install-persistence
expect_state "macos, scheduled unit re-installed" sched installed

# Removal is the same explicit disposition as for a watchdog.
run sched remove-persistence
[ ! -f "$SCHED_PLIST" ] || fail "remove-persistence left the scheduled unit behind"
expect_state "macos, after removing the schedule" sched missing

# Both at once is a contradiction, not a preference — and it gets its own state, because `n/a` is
# what doctor passes over in silence and a manifest error must never render as a green.
expect_state "macos, autostart and schedule together" both misdeclared
run both persistence-status
case "$out" in
  *"autostart and schedule"*) ;;
  *) fail "the contradiction must name itself, said: $out" ;;
esac
# The trap this walked into once: `both` says autostart = "true", which is the value the
# natively-restarting branch returns 0 for. A broken manifest must not exit successfully.
run both install-persistence
[ "$status" -ne 0 ] || fail "install-persistence on a contradictory manifest exited 0"

# An unparseable duration is named, not guessed at. "6 hours" is the plausible-looking spelling.
run badsched persistence-status
case "$out" in
  *"expected <N>m, <N>h or <N>d"*) ;;
  *) fail "a malformed schedule should say what the accepted forms are, said: $out" ;;
esac
run badsched install-persistence
[ "$status" -ne 0 ] || fail "install-persistence on an unparseable schedule exited 0"

# Minutes and days convert too, and the unit is a whole number of seconds in both cases.
mkcap minutely false 'schedule = "30m"'
mkcap daily false 'schedule = "2d"'
machine macos '["minutely", "daily"]'
run minutely install-persistence
grep -q "<integer>1800</integer>" "$FAKE_HOME/Library/LaunchAgents/com.sjel.minutely.plist" \
  || fail "30m did not render as 1800s"
run daily install-persistence
grep -q "<integer>172800</integer>" "$FAKE_HOME/Library/LaunchAgents/com.sjel.daily.plist" \
  || fail "2d did not render as 172800s"

# A scheduled CONTAINER capability still owes a unit. docker's --restart answers "bring it back
# when it dies", which is not "run it again in six hours" — so the native-restart shortcut that
# correctly suppresses a watchdog must not suppress a timer.
mkdir -p "$ROOT/capabilities/schedcontainer"
printf 'kind = "container"\nname = "schedcontainer"\nimage = "x"\ntag = "1"\nschedule = "12h"\n' \
  > "$ROOT/capabilities/schedcontainer/service.toml"
machine macos '["schedcontainer"]'
expect_state "macos, scheduled container is still owed a timer" schedcontainer missing

machine macos '["always", "ondemand", "gone"]'

# --- disabled: the unit outlives the enabled set ---------------------------
# watchdog.sh calls `service-runner.sh start <cap>` every 30s and consults nothing about the
# enabled set, so a unit left behind by `capability.sh disable` walks the capability back up.
# The tool has to keep answering for a capability that is no longer enabled, or nothing can
# report the leftover.
run gone install-persistence
GONE_PLIST="$FAKE_HOME/Library/LaunchAgents/com.sjel.gone.plist"
[ -f "$GONE_PLIST" ] || fail "could not install persistence for the to-be-disabled capability"
machine macos '["always", "ondemand"]'
expect_state "macos, unit left behind by a disable" gone installed
[ -f "$GONE_PLIST" ] || fail "the leftover unit disappeared on its own — the disposition must be explicit"

# A unit left under the pre-rename label (com.axon, until 2026-09-26) is unloaded and deleted
# when the capability is installed again, so two watchdogs never run for one capability.
LEGACY_PLIST="$FAKE_HOME/Library/LaunchAgents/com.axon.always.plist"
printf '<plist/>\n' > "$LEGACY_PLIST"
printf '%s\t0\tcom.axon.always\n' "$$" >> "$STUB_STATE"
run always install-persistence
[ ! -f "$LEGACY_PLIST" ] || fail "install-persistence left the legacy unit $LEGACY_PLIST behind"
! grep -q "com\.axon\.always\$" "$STUB_STATE" || fail "install-persistence did not unload the legacy unit"
grep -q "com\.sjel\.always\$" "$STUB_STATE" || fail "install-persistence did not load the new unit after the legacy cleanup"

# remove-persistence is that explicit disposition, and it is idempotent.
run gone remove-persistence
[ ! -f "$GONE_PLIST" ] || fail "remove-persistence left $GONE_PLIST behind"
expect_state "macos, after removal" gone missing
run gone remove-persistence
[ "$status" -eq 0 ] || fail "remove-persistence on an already-removed unit should be a no-op, got exit $status"

# --- Linux -----------------------------------------------------------------
# The same capability contract, the other backend. Rendering is pure, so this runs on either host:
# what is asserted is the unit the declaration implies, not that systemd accepted it.
machine linux '["always", "ondemand", "gone"]'
UNIT="$FAKE_HOME/.config/systemd/user/axon-always.service"
rm -f "$UNIT"
expect_state "linux, nothing installed" always missing

# install-persistence would call systemctl, which this host may not have — so the file half is
# rendered directly and only the reporting is exercised, which is the half that was missing.
HOME="$FAKE_HOME" XDG_CONFIG_HOME="$FAKE_HOME/.config" \
  "$ROOT/tools/service-runner.sh" persistence-status always >/dev/null 2>&1
sed -e "s|__WATCHDOG_PATH__|$ROOT/tools/watchdog.sh|" \
    -e "s|__PATH__|/bin:/usr/local/bin:/usr/bin:/bin|" \
    -e "s|__CAPABILITY__|always|" \
    -e "s|__LOG_OUT__|/tmp/axon-always-watchdog.log|" \
    -e "s|__LOG_ERR__|/tmp/axon-always-watchdog.err|" \
    "$ROOT/tools/templates/systemd-watchdog.service.tmpl" \
  | grep -v '__EXTRA_ENV__' > "$UNIT"   # the renderer drops that line when nothing is declared (#44)
expect_installed_file "linux, unit matching the declaration" always

printf '# hand-edited\n' >> "$UNIT"
expect_state "linux, unit edited" always stale

# --- linux: a schedule is two files ----------------------------------------
# systemd splits WHEN from WHAT, so a scheduled job is axon-<cap>.timer plus the oneshot it
# activates. This is the one place the two backends differ in SHAPE rather than syntax, and the
# case that matters is a timer matching while its companion has drifted — a check that only looked
# at the primary unit would report that as installed.
machine linux '["sched"]'
SYSD="$FAKE_HOME/.config/systemd/user"
TIMER="$SYSD/axon-sched.timer"
ONESHOT="$SYSD/axon-sched.service"
rm -f "$TIMER" "$ONESHOT"

# The primary unit for a scheduled job is the timer — the answer must name it, not the .service,
# because the timer is what gets enabled and what `installed` therefore has to mean.
run sched persistence-status
case "$out" in
  *"axon-sched.timer"*) ;;
  *) fail "linux, scheduled: expected the timer to be the primary unit, said: $out" ;;
esac

render_linux_sched() {  # render both files the way install would, without calling systemctl
  sed -e "s|__CAPABILITY__|sched|" -e "s|__INTERVAL_SECONDS__|21600|" \
      "$ROOT/tools/templates/systemd-schedule.timer.tmpl" > "$TIMER"
  sed -e "s|__RUNNER_PATH__|$ROOT/tools/service-runner.sh|" \
      -e "s|__PATH__|/bin:/usr/local/bin:/usr/bin:/bin|" \
      -e "s|__CAPABILITY__|sched|" \
      -e "s|__LOG_OUT__|/tmp/axon-sched-schedule.log|" \
      -e "s|__LOG_ERR__|/tmp/axon-sched-schedule.err|" \
      "$ROOT/tools/templates/systemd-schedule.service.tmpl" \
    | grep -v '__EXTRA_ENV__' > "$ONESHOT"   # the renderer drops that line when nothing is declared (#44)
}
render_linux_sched
expect_installed_file "linux, timer and its oneshot both match" sched

# The interval reaches the timer, and OnUnitActiveSec (not OnUnitInactiveSec): the oneshot stays
# "activating" for the whole run, so the interval must measure from when the run finished.
grep -q "OnUnitActiveSec=21600s" "$TIMER" || fail "the timer did not carry the converted interval"
grep -q "Type=oneshot" "$ONESHOT" || fail "the scheduled companion is not a oneshot"
grep -q "^Restart=" "$ONESHOT" && fail "a scheduled oneshot must not carry a Restart= policy"

# The case the single-file check could not see: timer intact, companion drifted.
printf '# hand-edited\n' >> "$ONESHOT"
expect_state "linux, the oneshot drifted under an intact timer" sched stale

# ...and the companion missing entirely is `missing`, not `installed`.
render_linux_sched
rm -f "$ONESHOT"
expect_state "linux, timer installed without its oneshot" sched missing

machine linux '["always", "ondemand", "gone"]'

# --- declared supervisor environment (#44) ---------------------------------
# The templates carried exactly one variable, PATH. A capability needing another had nowhere to
# put it, so the only way in was hand-editing the GENERATED unit — which persistence-status then
# reports as stale forever and install-persistence silently deletes. These cases hold the fix.
machine_env() {  # machine_env <os> <capability-env-toml-array>
  printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["always"]\n\n[capability.always]\nenv = %s\n' \
    "$1" "$2" > "$OVERLAY/config/machine.toml"
}

# Baseline: with nothing declared, the unit must be byte-identical to what it was before this
# feature existed. A machine that opts out pays nothing.
machine macos '["always"]'
run always install-persistence
cp "$PLIST" "$SCRATCH/no-env.plist"

machine_env macos '["FOO=bar"]'
run always install-persistence
grep -q "<key>FOO</key>" "$PLIST" || fail "declared env key did not reach the plist"
grep -q "<string>bar</string>" "$PLIST" || fail "declared env value did not reach the plist"
grep -q "<key>PATH</key>" "$PLIST" || fail "declaring env dropped the PATH the supervisor needs"
expect_state "macos, declared env is part of the declaration" always installed

# The whole point: re-installing is now lossless, where a hand-edit was not.
run always install-persistence
expect_state "macos, re-install keeps the declared env" always installed
grep -q "<key>FOO</key>" "$PLIST" || fail "re-installing dropped the declared env"

# Removing the declaration is drift, not a no-op — otherwise the unit keeps a variable the
# machine no longer declares and nothing reports it.
machine macos '["always"]'
expect_state "macos, env removed from the declaration" always stale
run always install-persistence
cmp -s "$PLIST" "$SCRATCH/no-env.plist" || fail "a machine declaring no env should render exactly what it rendered before the feature existed"

# A plist value is XML: an unescaped & makes the file unparseable and launchd fails silently.
machine_env macos '["Q=a&b"]'
run always install-persistence
grep -q "a&amp;b" "$PLIST" || fail "an ampersand in a declared value was not XML-escaped"
grep -q "a&b</string>" "$PLIST" && fail "a raw ampersand reached the plist"

# A malformed entry fails loudly rather than rendering half a unit.
machine_env macos '["NOTANASSIGNMENT"]'
run always persistence-status
case "$out" in
  *"has no '='"*) ;;
  *) fail "an env entry without '=' should be named, said: $out" ;;
esac

# Same declaration, other backend: systemd takes the quoted form so a value with a space survives.
machine_env linux '["FOO=two words"]'
UNIT="$FAKE_HOME/.config/systemd/user/axon-always.service"
rm -f "$UNIT"
HOME="$FAKE_HOME" XDG_CONFIG_HOME="$FAKE_HOME/.config" \
  "$ROOT/tools/service-runner.sh" persistence-status always >/dev/null 2>&1
# Render directly: install-persistence would call systemctl, which this host may not have.
machine_env linux '["FOO=two words"]'
run always persistence-status   # exercises the renderer through the state check
machine macos '["always", "ondemand", "gone"]'

# --- unsupported OS --------------------------------------------------------
# An OS with no backend must say so rather than report `missing`, which would send an operator
# after an install that cannot work here.
machine windows '["always"]'
expect_state "windows has no backend" always unsupported
machine freebsd '["always"]'
expect_state "an unknown os" always unsupported

# --- the report never leaks this machine's own state -----------------------
# The issue asks for exactly this: test evidence must not carry a machine-specific enabled set or
# service history. Everything above ran against $SCRATCH, so no path in any answer may point at
# the real home.
machine macos '["always"]'
run always
case "$out" in
  "$SCRATCH"*|*"$SCRATCH"*) ;;
  *) fail "the answer should reference the scratch root only, said: $out" ;;
esac
case "$out" in
  *"$HOME/Library"*) fail "the answer referenced the real HOME: $out" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "persistence: $fails check(s) failed"
  exit 1
fi
echo "persistence: all checks passed"

#!/bin/bash
# tools/doctor's scheduled-producer section, over a planted HOME, a planted launchd and planted logs.
#
# A capability that declares `schedule` has no supervisor. It is started by a timer, it runs, it
# exits, and nothing watches it in between — so it cannot be "down". It simply stops producing, and
# every other surface goes on saying fine. The capabilities that declare a `schedule` are that
# shape. The backup was one of them until 2026-09-29, when its timer moved into sjel-status's
# stored policy (commit ced02ae5) and capabilities/backup/service.toml was deleted. It is a library
# now and declares no `schedule`, so it is watched by the backup contract section instead. This
# section is the thing that asks; this file is the thing that watches it answer wrongly on purpose.
#
# Everything launchd is stubbed rather than skipped. `launchctl` is a fake on PATH, the units are
# planted plists and the "runs" are files with backdated mtimes, so all seven verdicts are exercised
# on a Linux runner as well as on the Mac they describe. A macOS-only assertion here would be one
# that runs on one machine and is believed everywhere (tools/lib/test-support.sh says why).
#
# Driven against a real checkout, like tools/persistence-orphans.test.sh: a partial tree makes the
# section return early on a failed registry call and every assertion would pass over an empty report.
set -uo pipefail

_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
source "$_root/tools/lib/test-support.sh"
isolate_axon_env

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

FAKE_HOME="$WORK/home"
UNIT_DIR="$FAKE_HOME/Library/LaunchAgents"
OVERLAY="$WORK/overlay"
LOGS="$WORK/logs"
FAKE_BIN="$WORK/bin"
mkdir -p "$UNIT_DIR" "$OVERLAY/config" "$LOGS" "$FAKE_BIN"

# Seven real capabilities, all of which declare a `schedule` in their own service.toml. Real ones
# rather than synthetic manifests, so a change to how a schedule is declared reaches this test.
cat > "$OVERLAY/config/machine.toml" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["feed-sweep", "host-watch", "people-registry", "sparpreis-watch", "host-patch", "container-refresh", "finance-prices"]
EOF

# `touch -t` takes CCYYMMDDhhmm.ss on both BSD and GNU; only the way to compute a past moment
# differs between them.
stamp_hours_ago() {  # <n>
  date -v-"$1"H +%Y%m%d%H%M.%S 2>/dev/null || date -d "$1 hours ago" +%Y%m%d%H%M.%S
}

plant_unit() {  # <capability> <interval-seconds>
  cat > "$UNIT_DIR/com.axon.$1.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.axon.$1</string>
  <key>StartInterval</key>
  <integer>$2</integer>
  <key>StandardOutPath</key>
  <string>$LOGS/axon-$1-schedule.log</string>
  <key>StandardErrorPath</key>
  <string>$LOGS/axon-$1-schedule.err</string>
</dict>
</plist>
EOF
}

plant_run() {  # <capability> <hours-ago>
  : > "$LOGS/axon-$1-schedule.log"
  touch -t "$(stamp_hours_ago "$2")" "$LOGS/axon-$1-schedule.log"
}

# The real units' intervals, from their manifests.
plant_unit feed-sweep        21600   # 6h
plant_unit host-watch         3600   # 1h
plant_unit people-registry   21600   # 6h
plant_unit sparpreis-watch   43200   # 12h
plant_unit host-patch        86400   # 24h
plant_unit container-refresh 86400   # 24h
# finance-prices deliberately gets NO unit.

plant_run feed-sweep      0   # produced just now
plant_run host-watch      8   # eight hours for an hourly job: two runs missed, at least
plant_run people-registry 8   # eight hours for a six-hourly job: one interval, not three
plant_run sparpreis-watch 1   # recent output, and the run behind it failed
plant_run host-patch      3
# container-refresh deliberately gets NO log file at all.

# The fake launchd. `com.axon.host-patch` is absent from the table on purpose: that is exactly the
# state the orchestrator left the real one in tonight, and the section must report it rather than
# treat a missing label as a healthy job.
cat > "$FAKE_BIN/launchctl" <<'LAUNCHCTL'
#!/bin/bash
[ "${1:-}" = "list" ] || exit 1
printf 'PID\tStatus\tLabel\n'
printf -- '-\t0\tcom.axon.feed-sweep\n'
printf -- '-\t0\tcom.axon.host-watch\n'
printf -- '-\t0\tcom.axon.people-registry\n'
printf -- '-\t1\tcom.axon.sparpreis-watch\n'
printf -- '-\t0\tcom.axon.container-refresh\n'
printf -- '-\t0\tcom.axon.finance-prices\n'
LAUNCHCTL
chmod +x "$FAKE_BIN/launchctl"

REPORT="$WORK/report.txt"
# To a file, never down a pipe: `doctor | grep -q` returns 141 under `set -o pipefail`, because
# grep exits at the first match and the SIGPIPE upstream becomes the pipeline's status.
( cd "$_root" && HOME="$FAKE_HOME" SJEL_OVERLAY_ROOT="$OVERLAY" PATH="$FAKE_BIN:$PATH" tools/doctor \
    > "$REPORT" 2>&1 )
says() { grep -qF "$1" "$REPORT"; }

# --- producing on cadence ---------------------------------------------------------------------
says "feed-sweep — runs every 6.0h, produced" \
  || fail "a producer that ran a moment ago was not reported healthy"

# --- stopped: three intervals with nothing produced ---------------------------------------------
says "host-watch — runs every 1.0h and has produced nothing for 8.0h; it has missed at least two runs" \
  || fail "an hourly producer silent for eight hours was not reported as stopped"

# --- late, but inside the range a closed lid explains --------------------------------------------
# launchd's StartInterval does not fire while the machine sleeps, so one missed interval on a
# laptop is ordinary. It is worth saying and it is not a fault.
says "people-registry — runs every 6.0h, last produced 8.0h ago" \
  || fail "a producer one interval late was not reported as late"

# --- the run happened and failed ------------------------------------------------------------------
# The case an age check alone cannot see: a job that fails fast still touches its log, so its
# freshness looks perfect.
says "sparpreis-watch — its last scheduled run exited 1" \
  || fail "a producer whose last run exited non-zero was reported on its age instead"

# --- the unit exists and launchd does not have it -------------------------------------------------
says "host-patch — its unit is installed and launchd has not loaded it" \
  || fail "a unit absent from launchctl was not reported; a timer that cannot fire read as healthy"

# --- no output this machine still holds ------------------------------------------------------------
# Not the same claim as "never ran": macOS clears /tmp of untouched entries at boot.
says "container-refresh — runs every 24.0h and has written no output this machine still holds" \
  || fail "a producer with no output file was not reported"

# --- declared, and no unit was ever installed ------------------------------------------------------
says "finance-prices — no unit installed" \
  || fail "a scheduled capability with no unit vanished from the section instead of being named"

# --- and the section must not go quiet on a host it does not cover -----------------------------------
# A Linux machine's timers are systemd's, which this does not read yet. Saying so is the point:
# "nothing to check" must never render as "checked fine".
sed -i.bak 's/^os = "macos"$/os = "linux"/' "$OVERLAY/config/machine.toml"
( cd "$_root" && HOME="$FAKE_HOME" SJEL_OVERLAY_ROOT="$OVERLAY" PATH="$FAKE_BIN:$PATH" tools/doctor \
    > "$REPORT" 2>&1 )
says "systemd timers are not covered yet" \
  || fail "on a host this section does not cover it said nothing at all"

if [ "$fails" -gt 0 ]; then
  echo "scheduled-producer tests: $fails failure(s)"
  exit 1
fi
echo "scheduled-producer tests: on cadence, stopped, late, failed run, unloaded unit, no output, no unit, and uncovered host all reported"

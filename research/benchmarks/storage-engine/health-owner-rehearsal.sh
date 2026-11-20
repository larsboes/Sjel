#!/bin/sh
# Rehearse the native Health owner on a scratch overlay: serve the owner file, import a synthetic
# export through the CLI and the owner socket, then back the file up through an owner snapshot
# and restore it in isolation.
set -eu
umask 077

if [ "$#" -ne 2 ]; then
    echo 'usage: sh health-owner-rehearsal.sh <native health binary> <new private scratch directory>' >&2
    exit 2
fi
binary=$(cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$1")")
tools=$(cd "$(dirname "$0")/../../../tools" && pwd -P)
test -x "$binary"
mkdir -m 700 "$2"
cd "$2"
work=$(pwd -P)
overlay="$work/overlay"
cap=health-wal-backup
mkdir -p "$overlay/config" "$overlay/capabilities/$cap" "$overlay/data/health" "$work/target"
printf '%s\n' health-owner-rehearsal > "$work/fixture-bearer"
printf 'SJEL_INBOUND_TOKEN_FILE=%s/fixture-bearer\n' "$work" > "$overlay/config/deployment.env"
case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo 'unsupported rehearsal host' >&2; exit 1 ;;
esac
printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["%s"]\n' "$os" "$cap" > "$overlay/config/machine.toml"
printf '[rehearsal-target]\nkind = "local"\npath = "%s/target"\n' "$work" > "$overlay/config/systems.local.toml"
cat > "$overlay/capabilities/$cap/service.toml" <<'MANIFEST'
kind = "data"
name = "health-wal-backup"
backup_owner_snapshot = "health"
backup_sqlite_online = "data/health/snapshots/health.db"
backup_target = "rehearsal-target"
MANIFEST
cat > export.xml <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<HealthData locale="en_US">
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="Phone" unit="count" startDate="2026-11-18 08:00:00 +0100" endDate="2026-11-18 08:10:00 +0100" value="1200"/>
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="Phone" unit="count" startDate="2026-11-19 08:00:00 +0100" endDate="2026-11-19 08:10:00 +0100" value="800"/>
 <Record type="HKQuantityTypeIdentifierBodyMass" sourceName="Scale" unit="kg" startDate="2026-11-19 07:00:00 +0100" endDate="2026-11-19 07:00:00 +0100" value="80"/>
</HealthData>
XML

export SJEL_OVERLAY_ROOT="$overlay" SJEL_PERSONAL_ROOT="$overlay"
export SJEL_HEALTH_DB_PATH="$overlay/data/health/health.db" SJEL_DB_PATH="$work/unused-shared.db"
export SJEL_HEALTH_BIN="$binary"
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
SJEL_PORT="$port" "$binary" serve > service.log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :' EXIT
trap 'exit 1' HUP INT TERM
tries=0
until curl -qfsS --noproxy '*' --max-time 2 "http://127.0.0.1:$port/ready" >/dev/null 2>&1; do
    kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 50 ] || { echo "See $work/service.log" >&2; exit 1; }
    tries=$((tries + 1))
    sleep 0.1
done

"$binary" import export.xml
"$binary" metrics > metrics.json
jq -e '[.metrics[].metric] | sort == ["body_mass_kg", "steps"]' metrics.json >/dev/null
jq -e '.metrics[] | select(.metric == "steps") | .days == 2' metrics.json >/dev/null
test ! -e "$work/unused-shared.db"
echo 'The CLI imported and read through the owner socket; the shared database was not opened.'

"$tools/backup.sh" --no-prune "$cap"
receipt="$overlay/backup/receipts/$cap.json"
archive="$work/target/$cap/$(jq -er '.tarball' "$receipt")"
"$tools/restore.sh" "$cap" "$archive" --receipt "$receipt" --destination "$work/restored"
restored="$work/restored/data/health/snapshots/health.db"
test "$(sqlite3 -readonly "file:$restored?immutable=1" 'SELECT count(*) FROM health_days')" = 3
test "$(sqlite3 -readonly "file:$restored?immutable=1" 'SELECT count(*) FROM health_imports')" = 1
test "$(sqlite3 -readonly "file:$restored?immutable=1" 'PRAGMA integrity_check')" = ok
echo 'The owner snapshot backup restored three daily values and one import in isolation.'

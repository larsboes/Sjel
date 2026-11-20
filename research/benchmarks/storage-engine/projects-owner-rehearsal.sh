#!/bin/sh
# Verify both archive legs and all four Projects tables without using the live store.
set -eu
umask 077

if [ "$#" -ne 3 ]; then
    echo 'usage: sh projects-owner-rehearsal.sh <libSQL projects binary> <native projects-server binary> <new private scratch directory>' >&2
    exit 2
fi
absolute_file() {
    printf '%s/%s\n' "$(cd "$(dirname "$1")" && pwd -P)" "$(basename "$1")"
}
legacy=$(absolute_file "$1")
owner=$(absolute_file "$2")
tools=$(cd "$(dirname "$0")/../../../tools" && pwd -P)
test -x "$legacy"
test -x "$owner"
mkdir -m 700 "$3"
cd "$3"
work=$(pwd -P)
overlay="$work/overlay"
mkdir -p "$overlay/config" "$overlay/data/sjel" "$overlay/data/projects" "$work/archives"
case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo 'unsupported rehearsal host' >&2; exit 1 ;;
esac
printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = ["store", "projects", "projects-wal-backup"]\n' "$os" > "$overlay/config/machine.toml"
printf '[backup-target]\nkind = "local"\npath = "%s/archives"\n' "$work" > "$overlay/config/systems.local.toml"
fixture=projects-owner-rehearsal
printf '%s\n' "$fixture" > fixture-bearer
printf 'SJEL_INBOUND_TOKEN_FILE=%s/fixture-bearer\n' "$work" > "$overlay/config/deployment.env"

shared="$overlay/data/sjel/sjel.db"
staged="$work/projects.db"
export SJEL_OVERLAY_ROOT="$overlay" SJEL_PERSONAL_ROOT="$overlay"
export SJEL_DB_PATH="$shared" SJEL_PROJECTS_DB_PATH="$staged" SJEL_PROJECTS_BIN="$owner"

"$legacy" new-project Rehearsal
"$legacy" add Rehearsal 'Prepare fixture'
sqlite3 -bail "$shared" <<'SQL'
PRAGMA foreign_keys=ON;
BEGIN;
CREATE TABLE unrelated_witness (id INTEGER);
INSERT INTO unrelated_witness VALUES (1);
INSERT INTO projects_mail_proposals
    (id, source_id, line_id, event, project, payload, version, status)
    VALUES (37, 'fixture', '1', 'fixture', 'Rehearsal', '{}', 3, 'approved_applied');
INSERT INTO projects_mail_applications (proposal_id, target_path)
    SELECT 37, path FROM projects_records WHERE kind = 'task';
INSERT INTO projects_inventory_links (record_path, item_ref, verified_revision, reviewed_at)
    SELECT path, 'inventory:item:fixture', 11, '2026-01-01T00:00:00Z'
    FROM projects_records WHERE kind = 'task';
COMMIT;
SQL

restore_backup() {
    cap="$1"
    destination="$2"
    "$tools/backup.sh" --no-prune "$cap"
    receipt="$overlay/backup/receipts/$cap.json"
    archive="$work/archives/$cap/$(jq -er '.tarball' "$receipt")"
    "$tools/restore.sh" "$cap" "$archive" --receipt "$receipt" --destination "$destination"
}
restore_backup store "$work/shared-restore"
python3 "$tools/owner-store-split.py" projects "$work/shared-restore/data/sjel/sjel.db" "$staged"
unrelated=$(sqlite3 -readonly "file:$staged?immutable=1" "SELECT name FROM sqlite_schema WHERE name = 'unrelated_witness'")
test -z "$unrelated"

export SJEL_DB_PATH="$work/missing-shared.db"
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
base="http://127.0.0.1:$port"
SJEL_PORT="$port" "$owner" > service.log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :' EXIT
trap 'exit 1' HUP INT TERM
tries=0
until curl -qfsS --noproxy '*' --max-time 2 "$base/ready" >/dev/null 2>&1; do
    kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 50 ] || { echo "See $work/service.log" >&2; exit 1; }
    tries=$((tries + 1))
    sleep 0.1
done

# The first owner archive represents the cutover, before further writes.
restore_backup projects-wal-backup "$work/owner-restore"
restored="$work/owner-restore/data/projects-wal-backup/projects.db"
python3 - "$work/shared-restore/data/sjel/sjel.db" "$restored" <<'PY'
import sqlite3
import sys
from pathlib import Path

with (
    sqlite3.connect(Path(sys.argv[1]).as_uri() + '?immutable=1', uri=True) as source,
    sqlite3.connect(Path(sys.argv[2]).as_uri() + '?immutable=1', uri=True) as restored,
):
    for table, count in (
        ('projects_records', 2),
        ('projects_inventory_links', 1),
        ('projects_mail_proposals', 1),
        ('projects_mail_applications', 1),
    ):
        data = f'SELECT * FROM {table} ORDER BY 1, 2'
        expected = source.execute(data).fetchall()
        assert len(expected) == count, f'incomplete fixture: {table}'
        assert restored.execute(data).fetchall() == expected, f'row mismatch: {table}'
PY

# Drive the owner with the shared path still absent. Headers stay on stdin.
call() {
    printf 'Authorization: Bearer %s\n' "$fixture" |
        curl -qfsS --noproxy '*' --max-time 5 -H @- "$@"
}
call "$base/api/tasks" | jq -e 'any(.tasks[]; .title == "Prepare fixture")' >/dev/null
call "$base/api/tasks" -H 'Content-Type: application/json' \
    --data '{"project":"Rehearsal","title":"Owner write"}' > write.json
call "$base/api/tasks" | jq -e 'any(.tasks[]; .title == "Owner write")' >/dev/null
test ! -e "$SJEL_DB_PATH"
echo 'Both receipt-backed restores matched all four Projects tables. The isolated owner read and wrote with the shared database absent.'

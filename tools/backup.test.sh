#!/bin/bash
# Synthetic backup ordering/retention matrix. No private overlay, live service, or SSH host.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

SCRATCH="$(mktemp -d /tmp/axon-backup-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT
FIXTURE="$SCRATCH/fixture"
OVERLAY="$SCRATCH/overlay"
MOCK_BIN="$SCRATCH/bin"
MOCK_LOG="$SCRATCH/operations.log"
MOCK_RESUME_COUNT="$SCRATCH/resume-count"
mkdir -p "$FIXTURE/tools/lib" "$FIXTURE/capabilities/vaultwarden" \
  "$OVERLAY/config" "$OVERLAY/data/vaultwarden/data" "$OVERLAY/data/vaultwarden/tls" \
  "$MOCK_BIN" "$SCRATCH/remote/vaultwarden"

cp "$ROOT/tools/backup.sh" "$FIXTURE/tools/backup.sh"
cp "$ROOT/tools/lib/toml.sh" "$FIXTURE/tools/lib/toml.sh"
# The real resolver, not a stub. backup.sh refuses a capability another deployment provides
# (retired-tracker#169), and every case below depends on that refusal NOT firing — a stub that
# always answered "local" would keep them green through a change that broke the check.
cp "$ROOT/tools/lib/external-ref.sh" "$FIXTURE/tools/lib/external-ref.sh"
# The skip guard: a platform-dependent assertion may be given up on a developer machine and never
# in CI. Sourced from the real tree, not the fixture — it governs this test, not the script under
# test.
source "$ROOT/tools/lib/test-support.sh"

cat > "$FIXTURE/tools/lib/paths.sh" <<PATHS
#!/bin/bash
SJEL_ROOT="$FIXTURE"
SJEL_PERSONAL_ROOT="$OVERLAY"
SJEL_MACHINE_TOML="$OVERLAY/config/machine.toml"
export SJEL_ROOT SJEL_PERSONAL_ROOT SJEL_MACHINE_TOML
source "$FIXTURE/tools/lib/toml.sh"
PATHS
cat > "$FIXTURE/tools/lib/platform.sh" <<'PLATFORM'
#!/bin/bash
SJEL_CONTAINER_RUNTIME="docker"
export SJEL_CONTAINER_RUNTIME
PLATFORM
cat > "$FIXTURE/tools/lib/bw-agent.sh" <<'AGENT'
#!/bin/bash
:
AGENT

cat > "$FIXTURE/tools/service-runner.sh" <<'RUNNER'
#!/bin/bash
set -u
action="$1"
printf 'service:%s\n' "$action" >> "$MOCK_LOG"
case "$action" in
  stop)
    [ "${MOCK_FAIL_STOP:-0}" -eq 0 ] || exit 1
    ;;
  resume)
    count=0
    [ ! -f "$MOCK_RESUME_COUNT" ] || count="$(cat "$MOCK_RESUME_COUNT")"
    count=$((count + 1))
    printf '%s\n' "$count" > "$MOCK_RESUME_COUNT"
    [ "$count" -gt "${MOCK_RESUME_FAILS:-0}" ] || exit 1
    ;;
esac
RUNNER

cat > "$FIXTURE/capabilities/vaultwarden/service.toml" <<'MANIFEST'
name = "vaultwarden"
image = "vaultwarden/server"
tag = "alpine"
env_file = "config/vaultwarden.env"
backup_paths = ["data/vaultwarden/data", "data/vaultwarden/tls"]
backup_sqlite = "data/vaultwarden/data/db.sqlite3"
backup_target = "synthetic-target"
backup_retain = "2"
MANIFEST
cat > "$OVERLAY/config/machine.toml" <<'MACHINE'
os = "linux"
container_runtime = "docker"
capabilities = ["vaultwarden"]
MACHINE
cat > "$OVERLAY/config/systems.local.toml" <<SYSTEMS
[synthetic-target]
host = "synthetic.invalid"
ssh_user = "backup-test"
backup_root = "$SCRATCH/remote"
SYSTEMS
printf 'VALID DATABASE COPY\n' > "$OVERLAY/data/vaultwarden/data/db.sqlite3"
printf 'attachment\n' > "$OVERLAY/data/vaultwarden/data/attachment.bin"
printf 'certificate\n' > "$OVERLAY/data/vaultwarden/tls/cert.pem"

cat > "$MOCK_BIN/rsync" <<'RSYNC'
#!/bin/bash
printf 'rsync:%s\n' "$2" >> "$MOCK_LOG"
[ "${MOCK_SIGNAL_PARENT:-0}" = "0" ] || {
  kill -"$MOCK_SIGNAL_PARENT" "$PPID"
  sleep 1
  exit 143
}
[ "${MOCK_FAIL_RSYNC:-0}" -eq 0 ] || exit 23
# Real rsync's two shapes, kept apart here on purpose. `src/` means "the contents of
# this directory" and fails with exit 23 when src is a file; `src` with no slash copies
# the thing itself. A mock that treated both as a directory copy would report the file
# form green against a script that cannot do it.
case "$2" in
  */)
    src="${2%/}"; dest="${3%/}"
    [ -d "$src" ] || { echo "rsync: $2: (l)stat: Not a directory" >&2; exit 23; }
    mkdir -p "$dest"
    /bin/cp -R "$src/." "$dest/"
    ;;
  *)
    /bin/cp "$2" "$3"
    ;;
esac
RSYNC
# One mock, three verbs, dispatched on the SQL because backup.sh now calls sqlite3 to TAKE a
# copy as well as to verify one. A mock that answered "ok" to everything would report the
# live-copy path green while it took no copy at all.
cat > "$MOCK_BIN/sqlite3" <<'SQLITE'
#!/bin/bash
db="$1"; sql="${2:-}"
case "$sql" in
  .backup*)
    printf 'sqlite:backup\n' >> "$MOCK_LOG"
    [ "${MOCK_FAIL_BACKUP:-0}" -eq 0 ] || exit 1
    dst="${sql#.backup \'}"; dst="${dst%\'}"
    /bin/cp "$db" "$dst"
    ;;
  *integrity_check*)
    printf 'sqlite:integrity\n' >> "$MOCK_LOG"
    if [ "${MOCK_FAIL_SQLITE:-0}" -eq 1 ]; then
      echo corrupt
      exit 1
    fi
    echo ok
    ;;
  *group_concat*) printf 'sqlite:count-sql\n' >> "$MOCK_LOG"; echo "select 0 as c" ;;
  *sqlite_master*) printf 'sqlite:tables\n' >> "$MOCK_LOG"; echo "${MOCK_TABLE_COUNT:-46}" ;;
  *sum\(c\)*)     printf 'sqlite:rows\n' >> "$MOCK_LOG"; echo "${MOCK_ROW_COUNT:-469598}" ;;
  *) echo 0 ;;
esac
SQLITE
cat > "$MOCK_BIN/cp" <<'COPY'
#!/bin/bash
if [ "${MOCK_FAIL_COLD_COPY:-0}" -eq 1 ] && [ "$1" = "$MOCK_SQLITE_SOURCE" ]; then
  printf 'copy:cold\n' >> "$MOCK_LOG"
  exit 1
fi
/bin/cp "$@"
COPY
cat > "$MOCK_BIN/ssh" <<'SSH'
#!/bin/bash
last=""
for arg in "$@"; do last="$arg"; done
case " $* " in
  *" -O exit "*) printf 'ssh:close\n' >> "$MOCK_LOG"; exit 0 ;;
esac
printf 'ssh:%s\n' "$last" >> "$MOCK_LOG"
/bin/sh -c "$last"
SSH
cat > "$MOCK_BIN/date" <<'DATE'
#!/bin/bash
if [ "$#" -eq 2 ] && [ "$1" = "-u" ] && [ "$2" = "+%Y%m%dT%H%M%SZ" ]; then
  printf '%s\n' "$MOCK_TIMESTAMP"
else
  /bin/date "$@"
fi
DATE
# The runtime, mocked so the digest the manifest records is one this file chose. A real docker
# would make the assertion below depend on whatever the operator happens to be running, and a
# hermetic test that talks to the live daemon is not hermetic — backup.sh reaches for the
# runtime by name whenever the capability declares an image, so this mock is what keeps it here.
#
# Two shapes, exactly the two backup.sh sends: `inspect <name> --format {{.Image}}` for the
# container's local image id, then `image inspect <id>` for the registry digest.
cat > "$MOCK_BIN/docker" <<'DOCKER'
#!/bin/bash
printf 'docker:%s\n' "$1" >> "$MOCK_LOG"
case "$1" in
  inspect)
    [ "$2" = "${MOCK_DOCKER_CONTAINER:-vaultwarden}" ] || exit 1
    printf '%s\n' "${MOCK_DOCKER_IMAGE_ID:-sha256:0f0f0f0f}"
    ;;
  image)
    # An image with no registry digest prints nothing and exits 0, which is what the real
    # `{{if .RepoDigests}}` template does — the case backup.sh falls back to the image id for.
    printf '%s\n' "${MOCK_DOCKER_REPO_DIGEST-vaultwarden/server@sha256:d1d1d1d1}"
    ;;
  *) exit 1 ;;
esac
DOCKER
chmod +x "$FIXTURE/tools/backup.sh" "$FIXTURE/tools/service-runner.sh" "$MOCK_BIN/"*

export PATH="$MOCK_BIN:$PATH"
export MOCK_LOG MOCK_RESUME_COUNT
export MOCK_SQLITE_SOURCE="$OVERLAY/data/vaultwarden/data/db.sqlite3"
BACKUP="$FIXTURE/tools/backup.sh"
fails=0

fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
expect_pass() {
  name="$1"; shift
  if output="$("$@" 2>&1)"; then :; else
    fail "$name should pass"
    echo "$output"
  fi
}
expect_fail_with() {
  name="$1"; expected="$2"; shift 2
  output="$("$@" 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$expected"; then
    fail "$name should fail with: $expected"
    echo "$output"
  fi
}
expect_fail() {
  name="$1"; shift
  if "$@" >/dev/null 2>&1; then fail "$name should fail"; fi
}
line_of() {
  pattern="$1"
  grep -n -m1 "$pattern" "$MOCK_LOG" | cut -d: -f1
}
assert_order() {
  before="$(line_of "$1")"; after="$(line_of "$2")"
  [ -n "$before" ] && [ -n "$after" ] && [ "$before" -lt "$after" ] \
    || fail "expected '$1' before '$2'"
}
reset_run() {
  : > "$MOCK_LOG"
  rm -f "$MOCK_RESUME_COUNT"
  unset MOCK_FAIL_RSYNC MOCK_FAIL_SQLITE MOCK_FAIL_STOP MOCK_FAIL_COLD_COPY \
    MOCK_FAIL_BACKUP MOCK_RESUME_FAILS MOCK_SIGNAL_PARENT
}
reset_remote() {
  rm -rf "$SCRATCH/remote/vaultwarden"
  mkdir -p "$SCRATCH/remote/vaultwarden"
}
make_old_archive() {
  name="$1"; stamp="$2"
  printf 'old archive\n' > "$SCRATCH/remote/vaultwarden/$name"
  touch -t "$stamp" "$SCRATCH/remote/vaultwarden/$name"
}

file_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}
source_before="$(file_sha256 "$OVERLAY/data/vaultwarden/data/db.sqlite3")"

# Pull mode produces a raw archive on stdout, keeps diagnostics on stderr, and does
# not resolve or contact a push target. Removing systems.local.toml proves destination
# coordinates are not a hidden stream-mode precondition.
reset_remote
reset_run
export MOCK_TIMESTAMP=20251231T010101Z
mv "$OVERLAY/config/systems.local.toml" "$OVERLAY/config/systems.local.toml.saved"
stream_archive="$SCRATCH/vaultwarden-stream.tar.gz"
stream_log="$SCRATCH/vaultwarden-stream.log"
if "$BACKUP" --stream vaultwarden > "$stream_archive" 2> "$stream_log"; then :; else
  fail "stream backup should pass without target coordinates"
fi
mv "$OVERLAY/config/systems.local.toml.saved" "$OVERLAY/config/systems.local.toml"
tar -tzf "$stream_archive" >/dev/null 2>&1 || fail "stream stdout was not a valid gzip archive"
tar -tzf "$stream_archive" | grep -q './axon-backup.toml' \
  || fail "stream archive omitted its backup contract"
grep -q 'stream vaultwarden archive' "$stream_log" \
  || fail "stream diagnostics were not written to stderr"

# Which build wrote these bytes. `tag` stopped answering that on 2026-09-02 (Q77) — the
# manifest declares "alpine", a rolling channel that reads the same for every archive this
# capability will ever produce — so backup.sh records the running container's digest beside it.
stream_meta="$(tar -xOzf "$stream_archive" ./axon-backup.toml)"
printf '%s' "$stream_meta" | grep -q 'image_digest = "vaultwarden/server@sha256:d1d1d1d1"' \
  || fail "the archive does not record the digest the container was running: $stream_meta"

# An image the daemon holds no registry digest for (built locally, or pulled before it recorded
# one) falls back to the local image id. Unknown would be worse than local-only.
reset_run
MOCK_DOCKER_REPO_DIGEST="" "$BACKUP" --stream vaultwarden > "$SCRATCH/vaultwarden-localid.tar.gz" 2>/dev/null \
  || fail "stream backup failed with an image that has no registry digest"
printf '%s' "$(tar -xOzf "$SCRATCH/vaultwarden-localid.tar.gz" ./axon-backup.toml)" \
  | grep -q 'image_digest = "sha256:0f0f0f0f"' \
  || fail "an image with no registry digest did not fall back to the local image id"
if grep -q '^ssh:' "$MOCK_LOG"; then fail "stream mode contacted a push target"; fi
[ ! -e "$OVERLAY/backup/receipts/vaultwarden.json" ] \
  || fail "stream mode fabricated a push receipt"
assert_order 'service:stop' 'sqlite:integrity'
assert_order 'sqlite:integrity' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] \
  || fail "stream mode did not resume exactly once"

# A write that cannot complete must make the producer fail, not exit 0 over an archive the
# consumer only partly received. /dev/full accepts opens and fails every write with ENOSPC, which
# is the short-write path forced deterministically -- the case that reached this suite once as a
# flake, where the archive arrived without its manifest member and the producer still exited 0.
#
# A closing pipe (`| head -c 32`) was tried first and does not work: the fixture archive fits in
# the pipe buffer, so cat completes before the reader goes away and there is no error to see. That
# it looked like a valid test is exactly why it is written down here rather than left out.
if [ -c /dev/full ]; then
  reset_remote
  reset_run
  export MOCK_TIMESTAMP=20251231T020202Z
  short_log="$SCRATCH/vaultwarden-short.log"
  if "$BACKUP" --stream vaultwarden > /dev/full 2> "$short_log"; then
    fail "stream exited 0 although every write to the consumer failed"
  fi
  grep -q 'failed part-way' "$short_log" \
    || fail "an unwritable stream did not say so; log: $(cat "$short_log")"
else
  # Not "skipped, presumably fine": on a developer machine this prints what is being given up,
  # and in CI it fails, because a platform-dependent assertion that stops running everywhere is
  # the failure this guard exists for (tools/lib/test-support.sh).
  skippable "no /dev/full on this host, so the short-write path cannot be forced"
fi

# Recovery mode is additive and the stopped-state order spans every host path plus
# SQLite verification. Resume precedes all SSH work.
reset_remote
make_old_archive vaultwarden-20240101T010101Z.tar.gz 202401010101
make_old_archive vaultwarden-20250101T010101Z.tar.gz 202501010101
reset_run
export MOCK_TIMESTAMP=20260101T010101Z
expect_pass "no-prune coherent backup" "$BACKUP" --no-prune vaultwarden
count="$(find "$SCRATCH/remote/vaultwarden" -maxdepth 1 -name 'vaultwarden-*.tar.gz' | wc -l | tr -d ' ')"
[ "$count" = 3 ] || fail "no-prune kept $count archives, expected 3"
[ -f "$SCRATCH/remote/vaultwarden/vaultwarden-20240101T010101Z.tar.gz" ] \
  || fail "no-prune removed the oldest archive"
grep -q '"retention_applied": false' "$OVERLAY/backup/receipts/vaultwarden.json" \
  || fail "no-prune receipt did not record retention_applied=false"
if grep -q 'tail -n +' "$MOCK_LOG"; then fail "no-prune issued the remote prune command"; fi
assert_order 'service:stop' 'rsync:.*data/vaultwarden/data'
assert_order 'rsync:.*data/vaultwarden/data' 'sqlite:integrity'
assert_order 'sqlite:integrity' 'service:resume'
assert_order 'service:resume' 'ssh:mkdir'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "successful run did not resume exactly once"

# Normal mode preserves the declared retention behavior.
reset_remote
make_old_archive vaultwarden-20230101T010101Z.tar.gz 202301010101
make_old_archive vaultwarden-20240101T010101Z.tar.gz 202401010101
make_old_archive vaultwarden-20250101T010101Z.tar.gz 202501010101
reset_run
export MOCK_TIMESTAMP=20260102T010101Z
expect_pass "normal retention backup" "$BACKUP" vaultwarden
count="$(find "$SCRATCH/remote/vaultwarden" -maxdepth 1 -name 'vaultwarden-*.tar.gz' | wc -l | tr -d ' ')"
[ "$count" = 2 ] || fail "normal retention kept $count archives, expected 2"
grep -q '"retention_applied": true' "$OVERLAY/backup/receipts/vaultwarden.json" \
  || fail "normal receipt did not record retention_applied=true"
grep -q 'tail -n +' "$MOCK_LOG" || fail "normal mode did not issue the remote prune command"

# A path-copy failure happens under the hold and the EXIT trap resumes before returning.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260103T010101Z MOCK_FAIL_RSYNC=1
expect_fail "rsync failure resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'service:stop' 'rsync:'
assert_order 'rsync:' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "rsync failure did not attempt one resume"

# Signals take the same loud, deterministic exit path as command failures. A TERM while
# the first path is being copied must resume and must not ship a partial snapshot.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260103T020202Z MOCK_SIGNAL_PARENT=TERM
expect_fail "interruption resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'service:stop' 'rsync:'
assert_order 'rsync:' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "interruption did not attempt one resume"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "interruption shipped an archive"

# Integrity failure also resumes, and nothing reaches the remote target.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260104T010101Z MOCK_FAIL_SQLITE=1
if "$BACKUP" --no-prune vaultwarden >/dev/null 2>&1; then fail "SQLite integrity failure should fail"; fi
assert_order 'sqlite:integrity' 'service:resume'
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "SQLite failure shipped an archive"

# The cold database copy has its own failure boundary after path staging.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260104T020202Z MOCK_FAIL_COLD_COPY=1
expect_fail "cold-copy failure resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'rsync:.*data/vaultwarden/tls' 'copy:cold'
assert_order 'copy:cold' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "cold-copy failure did not attempt one resume"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "cold-copy failure shipped an archive"

# A resume failure is loud and gets one second attempt from the EXIT trap. The backup
# remains failed even if a later operator action can recover the service.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260105T010101Z MOCK_RESUME_FAILS=2
expect_fail_with "resume failure is explicit" "CRITICAL: failed to resume" "$BACKUP" --no-prune vaultwarden
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 2 ] || fail "resume failure did not receive two explicit attempts"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "resume failure shipped an archive"

source_after="$(file_sha256 "$OVERLAY/data/vaultwarden/data/db.sqlite3")"
[ "$source_before" = "$source_after" ] || fail "synthetic live database was modified"

# --- backup_sqlite_online: the copy taken while every reader still has the file open -------
# capabilities/store's contract (PRD Q45). The two properties that separate it from the cold
# one above are asserted directly, because getting either wrong is silent: it must NOT stop
# anything, and it must take the copy through sqlite3 rather than reading the file itself.
mkdir -p "$FIXTURE/capabilities/store" "$OVERLAY/data/axon" "$SCRATCH/remote/store"
cat > "$FIXTURE/capabilities/store/service.toml" <<'MANIFEST'
kind = "data"
name = "store"
backup_sqlite_online = "data/axon/axon.db"
backup_target = "synthetic-target"
backup_retain = "2"
MANIFEST
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["vaultwarden", "store"]\n' \
  > "$OVERLAY/config/machine.toml"
printf 'SHARED DATABASE\n' > "$OVERLAY/data/axon/axon.db"
printf 'write-ahead log\n' > "$OVERLAY/data/axon/axon.db-wal"

reset_run
export MOCK_TIMESTAMP=20260106T030303Z
expect_pass "live sqlite backup" "$BACKUP" store
if grep -q '^service:' "$MOCK_LOG"; then
  fail "the live copy held the service down; log said: $(tr '\n' '|' < "$MOCK_LOG")"
fi
assert_order 'sqlite:backup' 'sqlite:integrity'
[ "$(grep -c '^sqlite:backup$' "$MOCK_LOG")" = 1 ] || fail "the copy was not taken through sqlite3 .backup"

store_archive="$(find "$SCRATCH/remote/store" -name 'store-*.tar.gz' | head -1)"
[ -n "$store_archive" ] || fail "no store archive was shipped"
if [ -n "$store_archive" ]; then
  meta="$(tar -xOzf "$store_archive" ./axon-backup.toml)"
  printf '%s' "$meta" | grep -q 'sqlite_online = "data/axon/axon.db"' \
    || fail "the archive contract does not name the live database"
  # No image, so no digest. The field is omitted rather than written empty: an empty string
  # would claim the answer is known to be nothing, and this capability has no container at all.
  printf '%s' "$meta" | grep -q 'image_digest' \
    && fail "a capability with no image recorded an image_digest anyway"
  # The counts a restore compares against. Without them a replayed-into-nothing archive
  # would pass integrity_check and read as a successful restore.
  printf '%s' "$meta" | grep -q 'sqlite_tables = "46"' || fail "table count not recorded"
  printf '%s' "$meta" | grep -q 'sqlite_rows = "469598"' || fail "row count not recorded"
  # The WAL sidecar is not staged: `.backup` checkpoints into the copy, so an archive
  # carrying one would mean the copy was a file read after all.
  tar -tzf "$store_archive" | grep -q 'axon.db-wal' && fail "a WAL sidecar reached the archive"
fi
grep -q '"contents": "sqlite_online"' "$OVERLAY/backup/receipts/store.json" \
  || fail "the receipt does not describe a live database copy"

# The falsifier for the branch above: a failed .backup must not ship an archive.
reset_run
rm -rf "$SCRATCH/remote/store"; mkdir -p "$SCRATCH/remote/store"
export MOCK_TIMESTAMP=20260107T030303Z MOCK_FAIL_BACKUP=1
expect_fail_with "a failed live copy is fatal" "sqlite3 .backup failed" "$BACKUP" store
[ -z "$(find "$SCRATCH/remote/store" -mindepth 1 -print -quit)" ] \
  || fail "a failed live copy shipped an archive"
unset MOCK_FAIL_BACKUP

# A manifest cannot claim both contracts: one says a run stops the capability, the other
# says it must not.
cat > "$FIXTURE/capabilities/store/service.toml" <<'MANIFEST'
kind = "data"
name = "store"
backup_sqlite = "data/axon/axon.db"
backup_sqlite_online = "data/axon/axon.db"
backup_target = "synthetic-target"
MANIFEST
reset_run
expect_fail_with "contradictory contracts are refused" "declare one" "$BACKUP" store

# --- backup_paths naming a single FILE ------------------------------------------------
# capabilities/finance's contract. Its canonical truth is one journal directory plus two
# individual files, and neither file has a directory of its own that would not also drag
# in every other capability's configuration. Before this, a declared file made rsync fail
# with "Not a directory" after mkdir -p had already created a directory with the file's
# name in the staging tree.
mkdir -p "$FIXTURE/capabilities/paperwork" "$OVERLAY/data/paperwork/journal"
printf 'journal entry\n' > "$OVERLAY/data/paperwork/journal/main.journal"
printf '{"budget": 1}\n' > "$OVERLAY/config/paperwork.json"
cat > "$FIXTURE/capabilities/paperwork/service.toml" <<'MANIFEST'
name = "paperwork"
backup_paths = ["data/paperwork/journal", "config/paperwork.json"]
backup_target = "synthetic-target"
MANIFEST
cat > "$OVERLAY/config/machine.toml" <<'MACHINE'
os = "linux"
container_runtime = "docker"
capabilities = ["vaultwarden", "store", "paperwork"]
MACHINE
reset_run
paperwork_archive="$SCRATCH/paperwork.tar.gz"
if "$BACKUP" --stream paperwork > "$paperwork_archive" 2>/dev/null; then :; else
  fail "a declared file in backup_paths should back up"
fi
members="$(tar -tzf "$paperwork_archive" 2>/dev/null)"
printf '%s' "$members" | grep -q '^\./config/paperwork\.json$' \
  || fail "the declared file is missing from the archive, or arrived as a directory"
printf '%s' "$members" | grep -q '^\./data/paperwork/journal/main\.journal$' \
  || fail "the declared directory stopped being copied by contents"

# The falsifier: a declared path that exists as neither is still refused before any copy.
cat > "$FIXTURE/capabilities/paperwork/service.toml" <<'MANIFEST'
name = "paperwork"
backup_paths = ["config/paperwork-that-is-not-there.json"]
backup_target = "synthetic-target"
MANIFEST
reset_run
expect_fail_with "a missing declared file is refused" "declared backup path is missing" \
  "$BACKUP" --stream paperwork

# --- a local destination, and the detector that watches it for eviction ---------------------
# `kind = "local"` is the destination this deployment actually ships to — an iCloud Drive folder
# — and the whole branch was untested here, including the check that is supposed to notice when
# the cloud takes an archive back. That check looked for `.<name>.icloud` placeholders and
# answered 0 against a target holding three evicted archives (measured 2026-09-08): CloudDocs
# marks an evicted file with SF_DATALESS under its own name and writes no placeholder.
mkdir -p "$FIXTURE/capabilities/ledger" "$OVERLAY/data/ledger"
printf 'one line of ledger\n' > "$OVERLAY/data/ledger/book.txt"
LOCAL_DEST="$SCRATCH/localdest"
mkdir -p "$LOCAL_DEST"
cat >> "$OVERLAY/config/systems.local.toml" <<SYSTEMS

[local-target]
kind = "local"
path = "$LOCAL_DEST"
SYSTEMS
cat > "$FIXTURE/capabilities/ledger/service.toml" <<'MANIFEST'
name = "ledger"
backup_paths = ["data/ledger"]
backup_target = "local-target"
MANIFEST
cat > "$OVERLAY/config/machine.toml" <<'MACHINE'
os = "linux"
container_runtime = "docker"
capabilities = ["vaultwarden", "store", "paperwork", "ledger"]
MACHINE

reset_run
export MOCK_TIMESTAMP=20260101T000000Z
local_log="$SCRATCH/ledger-clean.log"
"$BACKUP" ledger > "$local_log" 2>&1 || fail "a local destination should accept a backup"
[ -f "$LOCAL_DEST/ledger/ledger-20260101T000000Z.tar.gz" ] \
  || fail "the archive did not land at the local destination under its final name"
grep -q 'evicted placeholders' "$local_log" \
  && fail "a destination holding only real files reported eviction"

# A legacy provider's placeholder — the one form the old check could see. Kept, so the fix does
# not trade one blind spot for another.
: > "$LOCAL_DEST/ledger/.ledger-20251231T235959Z.tar.gz.icloud"
reset_run
export MOCK_TIMESTAMP=20260101T000100Z
local_log="$SCRATCH/ledger-stub.log"
"$BACKUP" ledger > "$local_log" 2>&1 || fail "a local destination should accept a backup"
grep -q '1 archive(s) at this destination are evicted placeholders' "$local_log" \
  || fail "the .icloud placeholder form stopped being detected"
rm -f "$LOCAL_DEST/ledger/.ledger-20251231T235959Z.tar.gz.icloud"

# The form that actually occurs, and the reason this section exists. SF_DATALESS is set by the
# file provider and cannot be planted: `chflags dataless <file>` exits 0 and sets nothing
# (measured 2026-09-08 on macOS 26). So `find` is stood in for, with `uname` forced to Darwin so
# both assertions below run identically on a Linux runner and a Mac. The mocks live in their own
# directory and are prepended for one invocation only — backup.sh calls `find` and `uname`
# nowhere else, and a mock on the shared PATH would follow every earlier case in this file.
BLIND_BIN="$SCRATCH/blind-bin"
mkdir -p "$BLIND_BIN"
cat > "$BLIND_BIN/uname" <<'UNAME'
#!/bin/bash
[ "${1:-}" = "-s" ] && { echo Darwin; exit 0; }
exec /usr/bin/uname "$@"
UNAME
# Answers the support probe (-maxdepth 0) yes and the listing with one dataless archive.
cat > "$BLIND_BIN/find" <<'FINDSEEING'
#!/bin/bash
case " $* " in
  *" -maxdepth 0 "*) exit 0 ;;
  *" -flags "*) echo "$2/ledger/ledger-19700101T000000Z.tar.gz"; exit 0 ;;
esac
exec /usr/bin/find "$@"
FINDSEEING
# A find with no -flags primary at all: the detector must SAY it cannot see, never count zero.
cat > "$BLIND_BIN/find.blind" <<'FINDBLIND'
#!/bin/bash
for a in "$@"; do
  [ "$a" = "-flags" ] && { echo "find: -flags: unknown primary or operator" >&2; exit 1; }
done
exec /usr/bin/find "$@"
FINDBLIND
chmod +x "$BLIND_BIN/uname" "$BLIND_BIN/find" "$BLIND_BIN/find.blind"

reset_run
export MOCK_TIMESTAMP=20260101T000200Z
local_log="$SCRATCH/ledger-dataless.log"
PATH="$BLIND_BIN:$PATH" "$BACKUP" ledger > "$local_log" 2>&1 \
  || fail "a local destination should accept a backup"
grep -q '1 archive(s) at this destination are evicted placeholders' "$local_log" \
  || fail "a dataless archive at the destination was not counted as evicted"

mv "$BLIND_BIN/find.blind" "$BLIND_BIN/find"
reset_run
export MOCK_TIMESTAMP=20260101T000300Z
local_log="$SCRATCH/ledger-blind.log"
PATH="$BLIND_BIN:$PATH" "$BACKUP" ledger > "$local_log" 2>&1 \
  || fail "a local destination should accept a backup"
grep -q 'does not understand -flags' "$local_log" \
  || fail "a find that cannot see dataless files reported nothing instead of saying so"

if [ "$fails" -gt 0 ]; then
  echo "backup tests: $fails failure(s)"
  exit 1
fi
echo "backup tests: stream, coherent hold, live copy, file paths, no-prune, retention, resume failures, and local-destination eviction passed"

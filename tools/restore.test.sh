#!/bin/bash
# Synthetic restore matrix: no private paths, values, archives, or live runtimes.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
RESTORE="$ROOT/tools/restore.sh"
SCRATCH="$(mktemp -d /tmp/sjel-restore-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT
MOCK_BIN="$SCRATCH/bin"
mkdir -p "$MOCK_BIN"

# The counts are what a restore is checked against, so the mock answers them separately
# from the integrity check — one that said "ok" to everything would report a database that
# came back empty as a verified restore, which is the case these tests exist for.
#
# The first argument may be a `file:...?mode=ro` URI: restore.sh opens the copy read-only so
# a verification run cannot write to what it is verifying. Strip it back to a path before
# reading the planted content.
cat > "$MOCK_BIN/sqlite3" <<'MOCK'
#!/bin/sh
db="$1"; sql="${2:-}"
path="${db#file:}"; path="${path%%\?*}"
case "$sql" in
  *integrity_check*)
    if grep -q BROKEN "$path"; then
      echo corrupt
      exit 1
    fi
    echo ok
    ;;
  *group_concat*) echo "select 0 as c" ;;
  *sqlite_master*) echo "${MOCK_SQLITE_TABLES-46}" ;;
  *"sum(c)"*)     echo "${MOCK_SQLITE_ROWS-469598}" ;;
  *) echo ok ;;
esac
MOCK
chmod +x "$MOCK_BIN/sqlite3"
export PATH="$MOCK_BIN:$PATH"

fails=0
expect_pass() {
  name="$1"; shift
  if output="$("$@" 2>&1)"; then :; else
    echo "FAIL: $name should pass"
    echo "$output"
    fails=$((fails + 1))
  fi
}
expect_fail_with() {
  name="$1"; expected="$2"; shift 2
  output="$("$@" 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$expected"; then
    echo "FAIL: $name should fail with: $expected"
    echo "$output"
    fails=$((fails + 1))
  fi
}

make_vault_tree() { # <stage> <sqlite-content> [omit-tls]
  stage="$1"; content="$2"; omit_tls="${3:-no}"
  mkdir -p "$stage/data/vaultwarden/data"
  printf '%s\n' "$content" > "$stage/data/vaultwarden/data/db.sqlite3"
  if [ "$omit_tls" = no ]; then
    mkdir -p "$stage/data/vaultwarden/tls"
    printf '%s\n' certificate > "$stage/data/vaultwarden/tls/cert.pem"
  fi
  cat > "$stage/axon-backup.toml" <<'META'
format = "1"
capability = "vaultwarden"
created_at = "2026-01-02T030405Z"
image = "vaultwarden/server"
tag = "alpine"
sqlite = "data/vaultwarden/data/db.sqlite3"
sqlite_online = ""
backup_paths = ["data/vaultwarden/data", "data/vaultwarden/tls"]
backup_container_paths = []
META
}

vault_stage="$SCRATCH/vault-stage"
mkdir -p "$vault_stage"
make_vault_tree "$vault_stage" VALID
tar czf "$SCRATCH/vaultwarden.tar.gz" -C "$vault_stage" .
vault_bytes="$(wc -c < "$SCRATCH/vaultwarden.tar.gz" | tr -d ' ')"
if command -v shasum >/dev/null 2>&1; then
  vault_sha="$(shasum -a 256 "$SCRATCH/vaultwarden.tar.gz" | awk '{print $1}')"
else
  vault_sha="$(sha256sum "$SCRATCH/vaultwarden.tar.gz" | awk '{print $1}')"
fi
cat > "$SCRATCH/vaultwarden-receipt.json" <<RECEIPT
{"capability":"vaultwarden","tarball":"vaultwarden.tar.gz","bytes":$vault_bytes,"sha256":"$vault_sha"}
RECEIPT
expect_pass "cold path and SQLite restore" "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" \
  --receipt "$SCRATCH/vaultwarden-receipt.json" --destination "$SCRATCH/vault-out"

# Which build wrote these bytes. `tag` stopped answering that on 2026-09-02 (Q77): the
# fixture above declares "alpine", which is a channel and reads the same for every archive the
# capability will ever produce. tools/backup.sh records the running container's digest instead,
# and restore prints it — a fact about the archive, never a gate, because the image moves daily
# and a restore refused for that would be refused exactly when it is wanted.
printf '%s' "$output" | grep -qF "image channel: vaultwarden/server:alpine" \
  || { echo "FAIL: restore did not name the image channel the archive was taken from"; echo "$output"; fails=$((fails + 1)); }
printf '%s' "$output" | grep -qF "running digest at backup: unrecorded" \
  || { echo "FAIL: an archive with no image_digest must say the digest is unrecorded, not omit the question"; echo "$output"; fails=$((fails + 1)); }

digest_stage="$SCRATCH/digest-stage"
mkdir -p "$digest_stage"
make_vault_tree "$digest_stage" VALID
DIGEST_REF="vaultwarden/server@sha256:1111111111111111111111111111111111111111111111111111111111111111"
printf 'image_digest = "%s"\n' "$DIGEST_REF" >> "$digest_stage/axon-backup.toml"
tar czf "$SCRATCH/vault-digest.tar.gz" -C "$digest_stage" .
expect_pass "archive carrying a running digest" "$RESTORE" vaultwarden "$SCRATCH/vault-digest.tar.gz" \
  --destination "$SCRATCH/vault-digest-out"
printf '%s' "$output" | grep -qF "running digest at backup: $DIGEST_REF" \
  || { echo "FAIL: restore did not print the digest the archive recorded"; echo "$output"; fails=$((fails + 1)); }

cat > "$SCRATCH/wrong-digest.json" <<RECEIPT
{"capability":"vaultwarden","tarball":"vaultwarden.tar.gz","bytes":$vault_bytes,"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}
RECEIPT
expect_fail_with "receipt digest mismatch" "SHA-256 does not match the receipt" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" --receipt "$SCRATCH/wrong-digest.json" \
  --destination "$SCRATCH/digest-out"

head -c 24 "$SCRATCH/vaultwarden.tar.gz" > "$SCRATCH/truncated.tar.gz"
expect_fail_with "truncated archive" "truncated or not a readable" \
  "$RESTORE" vaultwarden "$SCRATCH/truncated.tar.gz" --destination "$SCRATCH/truncated-out"

missing_stage="$SCRATCH/missing-stage"
mkdir -p "$missing_stage"
make_vault_tree "$missing_stage" VALID yes
tar czf "$SCRATCH/missing.tar.gz" -C "$missing_stage" .
expect_fail_with "missing declared member" "required backup path is missing" \
  "$RESTORE" vaultwarden "$SCRATCH/missing.tar.gz" --destination "$SCRATCH/missing-out"

expect_fail_with "wrong capability" "archive belongs to capability 'vaultwarden', not 'store'" \
  "$RESTORE" store "$SCRATCH/vaultwarden.tar.gz" --destination "$SCRATCH/wrong-out"

broken_stage="$SCRATCH/broken-stage"
mkdir -p "$broken_stage"
make_vault_tree "$broken_stage" BROKEN
tar czf "$SCRATCH/broken.tar.gz" -C "$broken_stage" .
expect_fail_with "failed SQLite integrity" "failed integrity_check" \
  "$RESTORE" vaultwarden "$SCRATCH/broken.tar.gz" --destination "$SCRATCH/broken-out"

live_candidate="$ROOT/restore-test-must-not-exist"
expect_fail_with "live checkout destination" "may not be inside the Sjel checkout" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" --destination "$live_candidate"
[ ! -e "$live_candidate" ] || { echo "FAIL: refused live destination was created"; fails=$((fails + 1)); }

container_payload="$SCRATCH/container-payload"
container_stage="$SCRATCH/container-stage"
mkdir -p "$container_payload" "$container_stage"
printf '%s\n' required > "$container_payload/required.txt"
chmod 750 "$container_payload/required.txt"
tar czpf "$container_stage/container-config.tar.gz" -C "$container_payload" .
cat > "$container_stage/axon-backup.toml" <<'META'
format = "1"
capability = "home-assistant"
created_at = "2026-01-02T030405Z"
image = "ghcr.io/home-assistant/home-assistant"
tag = "stable"
sqlite = ""
sqlite_online = ""
backup_paths = []
backup_container_paths = ["/config"]
META
tar czf "$SCRATCH/home-assistant.tar.gz" -C "$container_stage" .
expect_pass "container-path restore" "$RESTORE" home-assistant "$SCRATCH/home-assistant.tar.gz" --destination "$SCRATCH/container-out"
case "$(uname -s)" in
  Darwin) restored_mode="$(stat -f %Lp "$SCRATCH/container-out/container/config/required.txt")" ;;
  *) restored_mode="$(stat -c %a "$SCRATCH/container-out/container/config/required.txt")" ;;
esac
[ "$restored_mode" = 750 ] || { echo "FAIL: container-path mode was $restored_mode, expected 750"; fails=$((fails + 1)); }

store_stage="$SCRATCH/store-stage"
mkdir -p "$store_stage/data/axon"
printf '%s\n' 'SHARED DATABASE' > "$store_stage/data/axon/axon.db"
cat > "$store_stage/axon-backup.toml" <<'META'
format = "1"
capability = "store"
created_at = "2026-01-02T030405Z"
image = ""
tag = ""
sqlite = ""
sqlite_online = "data/axon/axon.db"
sqlite_tables = "46"
sqlite_rows = "469598"
backup_paths = []
backup_container_paths = []
META
tar czf "$SCRATCH/store.tar.gz" -C "$store_stage" .
expect_pass "live-copy restore" "$RESTORE" store "$SCRATCH/store.tar.gz" \
  --destination "$SCRATCH/store-out"

# Read-only, and it matters: a verification that journalled the copy on open would modify
# the artifact it exists to check, and the next run would verify a different file.
output="$("$RESTORE" store "$SCRATCH/store.tar.gz" --destination "$SCRATCH/store-ro-out" 2>&1)"
printf '%s' "$output" | grep -qF "46 table(s), 469598 row(s)" || {
  echo "FAIL: restore should report what it counted"; echo "$output"; fails=$((fails + 1)); }

legacy_stage="$SCRATCH/legacy-stage"
mkdir -p "$legacy_stage/data/vaultwarden/data" "$legacy_stage/data/vaultwarden/tls"
printf '%s\n' VALID > "$legacy_stage/data/vaultwarden/data/db.sqlite3"
printf '%s\n' certificate > "$legacy_stage/data/vaultwarden/tls/cert.pem"
tar czf "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" -C "$legacy_stage" .
expect_fail_with "legacy archive is explicit" "pass --allow-legacy" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" --destination "$SCRATCH/legacy-refused"
expect_pass "known legacy archive" "$RESTORE" vaultwarden "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" \
  --destination "$SCRATCH/legacy-out" --allow-legacy

# A copy that opens and passes integrity_check proves the pages are consistent and nothing
# about whether the rows came back — an EMPTY database is internally perfect. These pin the
# contents check that closes that, which is the one thing a restore rehearsal is for.
MOCK_SQLITE_ROWS=469000 expect_fail_with "short row count is caught" \
  "restored 469000 row(s), archive recorded 469598" \
  "$RESTORE" store "$SCRATCH/store.tar.gz" --destination "$SCRATCH/store-short-out"

MOCK_SQLITE_TABLES=45 expect_fail_with "missing table is caught" \
  "restored 45 table(s), archive recorded 46" \
  "$RESTORE" store "$SCRATCH/store.tar.gz" --destination "$SCRATCH/store-missing-out"

# An archive from before the counts were recorded. It still has to contain something.
no_counts_stage="$SCRATCH/store-nocounts-stage"
mkdir -p "$no_counts_stage/data/axon"
printf '%s\n' 'SHARED DATABASE' > "$no_counts_stage/data/axon/axon.db"
{
  printf 'format = "1"\ncapability = "store"\ncreated_at = "2026-01-02T030405Z"\n'
  printf 'image = ""\ntag = ""\nsqlite = ""\nsqlite_online = "data/axon/axon.db"\n'
  printf 'backup_paths = []\nbackup_container_paths = []\n'
} > "$no_counts_stage/axon-backup.toml"
tar czf "$SCRATCH/store-nocounts.tar.gz" -C "$no_counts_stage" .
MOCK_SQLITE_TABLES=0 expect_fail_with "an empty database is caught without a recorded expectation" \
  "the copy came back empty" \
  "$RESTORE" store "$SCRATCH/store-nocounts.tar.gz" --destination "$SCRATCH/store-empty-out"
expect_pass "archive without recorded contents still restores" \
  "$RESTORE" store "$SCRATCH/store-nocounts.tar.gz" --destination "$SCRATCH/store-nocounts-out"

# A corrupt copy fails on the integrity check, before any count is compared.
broken_store="$SCRATCH/store-broken-stage"
mkdir -p "$broken_store/data/axon"
printf '%s\n' 'BROKEN' > "$broken_store/data/axon/axon.db"
cp "$store_stage/axon-backup.toml" "$broken_store/axon-backup.toml"
tar czf "$SCRATCH/store-broken.tar.gz" -C "$broken_store" .
expect_fail_with "a corrupt live copy is caught" "failed integrity_check" \
  "$RESTORE" store "$SCRATCH/store-broken.tar.gz" --destination "$SCRATCH/store-broken-out"

# --- retired contract (PRD Q45) --------------------------------------------------------------
# A pg_dumpall archive was verified by replaying it into a disposable PostgreSQL container.
# The capability and its image pin are gone, so this checkout cannot rehearse one — and
# rehearsing against some other Postgres would be a different check wearing this one's name.
# Refused BEFORE a destination exists, which is the property the old runtime preflight had
# (#163): the operation that proves a backup is recoverable must not announce that it cannot
# after creating a directory and extracting into it.
pg_stage="$SCRATCH/pg-stage"
mkdir -p "$pg_stage"
printf '%s\n' 'select 1;' > "$pg_stage/pg_dumpall.sql"
{
  printf 'format = "1"\ncapability = "store"\ncreated_at = "2026-01-02T030405Z"\n'
  printf 'image = ""\ntag = ""\nsqlite = ""\nsqlite_online = "data/axon/axon.db"\n'
  printf 'backup_paths = []\nbackup_container_paths = []\n'
} > "$pg_stage/axon-backup.toml"
tar czf "$SCRATCH/pg-retired.tar.gz" -C "$pg_stage" .
PG_DEST="$SCRATCH/pg-retired-out"
expect_fail_with "a PostgreSQL archive is refused by name" "was retired on 2026-08-27" \
  "$RESTORE" store "$SCRATCH/pg-retired.tar.gz" --destination "$PG_DEST"
[ ! -e "$PG_DEST" ] || {
  echo "FAIL: the destination was created before the archive was refused"; fails=$((fails + 1)); }

if [ "$fails" -gt 0 ]; then
  echo "restore tests: $fails failure(s)"
  exit 1
fi
echo "restore tests: all synthetic backup forms and failure cases passed"

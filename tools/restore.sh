#!/bin/bash
# Restore and verify one tools/backup.sh archive without writing into live Axon state.
# The extracted tree remains for inspection; a database in it is opened read-only and
# checked against the contents the archive recorded. Bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"
# `does this stream contain X` without the answer depending on where the match sits (#42).
source "$TOOLS_DIR/lib/pipe.sh"

usage() {
  echo "usage: restore.sh <capability> <archive.tar.gz> [--receipt <receipt.json>] [--destination <empty-dir>] [--allow-legacy]" >&2
  exit 1
}

[ $# -ge 2 ] || usage
CAP="$1"
ARCHIVE="$2"
shift 2

DEST_ARG=""
RECEIPT=""
ALLOW_LEGACY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --receipt)
      [ $# -ge 2 ] || usage
      RECEIPT="$2"; shift 2
      ;;
    --destination)
      [ $# -ge 2 ] || usage
      DEST_ARG="$2"; shift 2
      ;;
    --allow-legacy)
      ALLOW_LEGACY=1; shift
      ;;
    *) usage ;;
  esac
done

case "$CAP" in
  ''|*[!A-Za-z0-9._-]*) echo "restore.sh: invalid capability name" >&2; exit 1 ;;
esac

MANIFEST="$SJEL_ROOT/capabilities/$CAP/service.toml"
[ -f "$MANIFEST" ] || { echo "restore.sh: no $MANIFEST" >&2; exit 1; }
[ -f "$ARCHIVE" ] || { echo "restore.sh: archive not found: $ARCHIVE" >&2; exit 1; }
ARCHIVE="$(cd "$(dirname "$ARCHIVE")" && pwd -P)/$(basename "$ARCHIVE")"

WORK="$(mktemp -d "/tmp/axon-restore-check.XXXXXX")"
DEST=""
DEST_AUTO=0
RESTORE_COMPLETE=0
cleanup() {
  rm -rf "$WORK"
  if [ "$DEST_AUTO" -eq 1 ] && [ "$RESTORE_COMPLETE" -ne 1 ] && [ -n "$DEST" ]; then
    rm -rf "$DEST"
  fi
}
trap cleanup EXIT

fail() { echo "restore.sh: $*" >&2; exit 1; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    fail "need shasum or sha256sum to identify the archive"
  fi
}

ARCHIVE_BYTES="$(wc -c < "$ARCHIVE" | tr -d ' ')"
ARCHIVE_SHA256="$(sha256_file "$ARCHIVE")"
if [ -n "$RECEIPT" ]; then
  [ -f "$RECEIPT" ] || fail "receipt not found: $RECEIPT"
  command -v jq >/dev/null 2>&1 || fail "jq is required to verify a receipt"
  RECEIPT_CAP="$(jq -r '.capability // empty' "$RECEIPT")"
  RECEIPT_ARCHIVE="$(jq -r '.tarball // empty' "$RECEIPT")"
  RECEIPT_BYTES="$(jq -r '.bytes // empty' "$RECEIPT")"
  RECEIPT_SHA256="$(jq -r '.sha256 // empty' "$RECEIPT")"
  [ "$RECEIPT_CAP" = "$CAP" ] || fail "receipt belongs to another capability"
  [ "$RECEIPT_ARCHIVE" = "$(basename "$ARCHIVE")" ] || fail "receipt names another archive"
  [ "$RECEIPT_BYTES" = "$ARCHIVE_BYTES" ] || fail "archive byte count does not match the receipt"
  if [ -n "$RECEIPT_SHA256" ]; then
    [ "$RECEIPT_SHA256" = "$ARCHIVE_SHA256" ] || fail "archive SHA-256 does not match the receipt"
  else
    echo "restore.sh: warning: legacy receipt has no SHA-256; size is the only receipt-level check" >&2
  fi
fi

# Validate before extraction. Link and special-file members are rejected because a
# syntactically relative filename can still escape through a preceding symlink.
TAR_SEQ=0
LAST_TAR_LIST=""
validate_tar() { # <archive> <description>
  archive="$1"; description="$2"
  TAR_SEQ=$((TAR_SEQ + 1))
  list="$WORK/tar-$TAR_SEQ.list"
  verbose="$WORK/tar-$TAR_SEQ.verbose"
  tar -tzf "$archive" > "$list" 2>/dev/null || fail "$description is truncated or not a readable gzip tar archive"
  [ -s "$list" ] || fail "$description is empty"
  while IFS= read -r member; do
    normalized="${member#./}"
    case "$normalized" in
      ''|.) ;;
      /*|..|../*|*/../*) fail "$description contains an unsafe member path" ;;
    esac
  done < "$list"
  tar -tvzf "$archive" > "$verbose" 2>/dev/null || fail "$description metadata cannot be read"
  if LC_ALL=C grep -Eq '^[lhbcps]' "$verbose"; then
    fail "$description contains a link or special file; refusing unsafe extraction"
  fi
  LAST_TAR_LIST="$list"
}

safe_relative() {
  case "$1" in
    ''|/*|..|../*|*/../*) return 1 ;;
    *) return 0 ;;
  esac
}

path_within() { # <candidate> <root>
  case "$1" in "$2"|"$2"/*) return 0 ;; *) return 1 ;; esac
}

DEST_NEEDS_CREATE=0
if [ -n "$DEST_ARG" ]; then
  if [ -e "$DEST_ARG" ]; then
    [ -d "$DEST_ARG" ] || fail "destination exists and is not a directory: $DEST_ARG"
    [ -z "$(find "$DEST_ARG" -mindepth 1 -maxdepth 1 -print -quit)" ] || fail "destination must be empty: $DEST_ARG"
    DEST="$(cd "$DEST_ARG" && pwd -P)"
  else
    parent="$(dirname "$DEST_ARG")"
    [ -d "$parent" ] || fail "destination parent does not exist: $parent"
    DEST="$(cd "$parent" && pwd -P)/$(basename "$DEST_ARG")"
    DEST_NEEDS_CREATE=1
  fi
else
  DEST="/tmp/axon-restore-$CAP.XXXXXX"
  DEST_NEEDS_CREATE=2
fi

SJEL_REAL="$(cd "$SJEL_ROOT" && pwd -P)"
OVERLAY_REAL="$(cd "$SJEL_PERSONAL_ROOT" 2>/dev/null && pwd -P || true)"
path_within "$DEST" "$SJEL_REAL" && fail "destination may not be inside the Axon checkout"
[ -n "$OVERLAY_REAL" ] && path_within "$DEST" "$OVERLAY_REAL" \
  && fail "destination may not be inside the active private overlay"
# Refused BEFORE the destination exists, for the reason the container preflight this replaces
# existed (#163): the one operation whose purpose is proving a backup is recoverable must not
# announce that it cannot, having already created a directory and extracted into it.
#
# A pg_dumpall archive was verified by replaying it into a disposable PostgreSQL container.
# That capability was retired on 2026-08-27 (PRD Q45) and its image pin left with it, so this
# checkout can no longer rehearse one — and rehearsing it against SOME other Postgres would be
# a different check wearing this one's name. `tar -tzf` only lists; nothing is written here.
#
# stream_matches rather than `grep -q`: -q exits at the first hit, tar dies of SIGPIPE, and
# `set -o pipefail` turns a FOUND into a failed pipeline — so the refusal below would not fire
# for the archive it is written for. tools/pipe.test.sh is the gate on that, and it caught this
# line when the container preflight it replaced was removed.
if tar -tzf "$ARCHIVE" 2>/dev/null | stream_matches -E '(^|/)pg_dumpall\.sql$'; then
  fail "this archive carries a PostgreSQL dump, and the capability that produced it was retired on 2026-08-27 (PRD Q45). Verify it with the Axon revision that still declares the image"
fi

case "$DEST_NEEDS_CREATE" in
  1) mkdir "$DEST" ;;
  2) DEST="$(mktemp -d "$DEST")"; DEST_AUTO=1 ;;
esac

validate_tar "$ARCHIVE" "backup archive"
OUTER_LIST="$LAST_TAR_LIST"
META_MEMBER="$(grep -E '^(\./)?axon-backup\.toml$' "$OUTER_LIST" || true)"
if [ -n "$META_MEMBER" ]; then
  [ "$(printf '%s\n' "$META_MEMBER" | grep -c .)" -eq 1 ] || fail "archive contains more than one backup manifest"
  tar -xOzf "$ARCHIVE" "$META_MEMBER" > "$WORK/axon-backup.toml"
  FORMAT="$(toml_get format "$WORK/axon-backup.toml")"
  ARCHIVE_CAP="$(toml_get capability "$WORK/axon-backup.toml")"
  [ "$FORMAT" = "1" ] || fail "unsupported backup format '$FORMAT'"
  [ "$ARCHIVE_CAP" = "$CAP" ] || fail "archive belongs to capability '$ARCHIVE_CAP', not '$CAP'"
  CREATED_AT="$(toml_get created_at "$WORK/axon-backup.toml")"
  IMAGE="$(toml_get image "$WORK/axon-backup.toml")"
  TAG="$(toml_get tag "$WORK/axon-backup.toml")"
  # Optional: written since Q77 (2026-09-02), absent from every archive taken before it.
  IMAGE_DIGEST="$(toml_get image_digest "$WORK/axon-backup.toml")"
  SQLITE_REL="$(toml_get sqlite "$WORK/axon-backup.toml")"
  SQLITE_ONLINE_REL="$(toml_get sqlite_online "$WORK/axon-backup.toml")"
  # Optional: an archive taken before the field existed does not carry it, and is checked
  # for non-emptiness instead of an exact match.
  EXPECT_TABLES="$(toml_get sqlite_tables "$WORK/axon-backup.toml")"
  EXPECT_ROWS="$(toml_get sqlite_rows "$WORK/axon-backup.toml")"
  PATHS=()
  while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$WORK/axon-backup.toml")
  CPATHS=()
  while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$WORK/axon-backup.toml")

  CURRENT_IMAGE="$(toml_get image "$MANIFEST")"
  CURRENT_TAG="$(toml_get tag "$MANIFEST")"
  CURRENT_SQLITE="$(toml_get backup_sqlite "$MANIFEST")"
  CURRENT_SQLITE_ONLINE="$(toml_get backup_sqlite_online "$MANIFEST")"
  # A CHANNEL check, and only that, since Q77 (2026-09-02). Both sides were version
  # literals until then, so this refused a vaultwarden archive taken at 1.36.0-alpine from
  # being read back against a manifest declaring 1.37.0-alpine. Every declared tag is now a
  # rolling channel, so both sides are the same constant string for the life of the channel and
  # what survives is the narrower question: was this archive taken from the same image and the
  # same stream this manifest names.
  #
  # Not widened to the digest on purpose. The archive records the digest it was taken at
  # (image_digest, printed below), the running image moves daily by design, and a restore
  # refused for that reason would be refused precisely when it is needed. The version question
  # is answered by reading the two digests, which is a human's call and not a gate.
  [ "$IMAGE:$TAG" = "$CURRENT_IMAGE:$CURRENT_TAG" ] \
    || fail "archive was taken from '$IMAGE:$TAG'; this manifest declares '$CURRENT_IMAGE:$CURRENT_TAG' — use the matching Axon revision"
  [ "$SQLITE_REL:$SQLITE_ONLINE_REL" = "$CURRENT_SQLITE:$CURRENT_SQLITE_ONLINE" ] \
    || fail "archive database contract differs from the current tracked manifest; use the matching Axon revision"
  [ "$(toml_array backup_paths "$WORK/axon-backup.toml")" = "$(toml_array backup_paths "$MANIFEST")" ] \
    || fail "archive path contract differs from the current tracked manifest; use the matching Axon revision"
  [ "$(toml_array backup_container_paths "$WORK/axon-backup.toml")" = "$(toml_array backup_container_paths "$MANIFEST")" ] \
    || fail "archive container-path contract differs from the current tracked manifest; use the matching Axon revision"
else
  [ "$ALLOW_LEGACY" -eq 1 ] || fail "archive has no axon-backup.toml; pass --allow-legacy only for a known pre-format archive"
  case "$(basename "$ARCHIVE")" in
    "$CAP"-*.tar.gz) ;;
    *) fail "legacy archive name does not identify capability '$CAP'" ;;
  esac
  FORMAT="legacy"
  CREATED_AT="unknown"
  IMAGE="$(toml_get image "$MANIFEST")"
  TAG="$(toml_get tag "$MANIFEST")"
  SQLITE_REL="$(toml_get backup_sqlite "$MANIFEST")"
  SQLITE_ONLINE_REL="$(toml_get backup_sqlite_online "$MANIFEST")"
  EXPECT_TABLES=""
  EXPECT_ROWS=""
  IMAGE_DIGEST=""
  PATHS=()
  while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$MANIFEST")
  CPATHS=()
  while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$MANIFEST")
  echo "restore.sh: warning: legacy archive identity comes from its filename and the current manifest" >&2
fi
[ "${#PATHS[@]}" -gt 0 ] || [ "${#CPATHS[@]}" -gt 0 ] || [ -n "$SQLITE_ONLINE_REL" ] \
  || fail "archive declares no restorable content"

echo "→ archive: $(basename "$ARCHIVE")"
echo "  bytes: $ARCHIVE_BYTES · sha256: $ARCHIVE_SHA256"
echo "  format: $FORMAT · capability: $CAP · created: $CREATED_AT"
# The image line is what the tag above stopped answering. `image:tag` names the channel the
# archive was taken from; the digest names the build. Printed rather than compared — see the
# identity check above for why a moved digest is not an error.
if [ -n "$IMAGE" ]; then
  if [ -n "$IMAGE_DIGEST" ]; then
    echo "  image channel: $IMAGE:$TAG · running digest at backup: $IMAGE_DIGEST"
  else
    echo "  image channel: $IMAGE:$TAG · running digest at backup: unrecorded (archive predates the field, or no container answered)"
  fi
fi
echo "→ extract outer archive → $DEST"
tar -xzpf "$ARCHIVE" -C "$DEST"

if [ "${#PATHS[@]}" -gt 0 ]; then
  for path in "${PATHS[@]}"; do
    safe_relative "$path" || fail "backup manifest contains an unsafe path declaration"
    [ -e "$DEST/$path" ] || fail "required backup path is missing from the archive: $path"
  done
  echo "  declared path roots: ${#PATHS[@]} present"
fi

if [ -n "$SQLITE_REL" ]; then
  safe_relative "$SQLITE_REL" || fail "backup manifest contains an unsafe SQLite path"
  [ -f "$DEST/$SQLITE_REL" ] || fail "declared SQLite database is missing from the archive: $SQLITE_REL"
  command -v sqlite3 >/dev/null 2>&1 || fail "sqlite3 is required to verify the restored database"
  sqlite_result="$(sqlite3 "$DEST/$SQLITE_REL" 'pragma integrity_check;' 2>/dev/null || true)"
  [ "$sqlite_result" = "ok" ] || fail "restored SQLite database failed integrity_check"
  echo "  SQLite integrity_check: ok"
fi

if [ "${#CPATHS[@]}" -gt 0 ]; then
  for container_path in "${CPATHS[@]}"; do
    case "$container_path" in /*) ;; *) fail "container backup path must be absolute" ;; esac
    relative="${container_path#/}"
    safe_relative "$relative" || fail "backup manifest contains an unsafe container path"
    safe="$(printf '%s' "$relative" | tr '/' '_')"
    nested="$DEST/container-$safe.tar.gz"
    [ -s "$nested" ] || fail "required container-path archive is missing: container-$safe.tar.gz"
    validate_tar "$nested" "container-path archive for $container_path"
    nested_dest="$DEST/container/$safe"
    mkdir -p "$nested_dest"
    tar -xzpf "$nested" -C "$nested_dest"
    [ -n "$(find "$nested_dest" -mindepth 1 -print -quit)" ] || fail "container-path restore is empty"
    echo "  container path restored with archived modes: $container_path → container/$safe/"
  done
fi

# The shared store's archive holds one file taken with `sqlite3 .backup`, so verification is
# the file itself: it must open, it must be internally consistent, and it must hold what the
# producer recorded. That third check is the one that matters. An EMPTY database passes
# integrity_check perfectly, which is exactly how a backup of nothing reads as a successful
# restore — the same trap the retired pg_dumpall contract recorded its counts to avoid.
#
# Read-only, and inside $DEST rather than anywhere near the live file: `file:...?mode=ro`
# means a verification run cannot write to what it is verifying, and a WAL-less copy would
# otherwise be journalled on first open.
verify_sqlite_online() {
  local db ic tables rows count_sql
  db="$DEST/$SQLITE_ONLINE_REL"
  [ -f "$db" ] || fail "declared SQLite database is missing from the archive: $SQLITE_ONLINE_REL"
  command -v sqlite3 >/dev/null 2>&1 || fail "sqlite3 is required to verify the restored database"

  # `|| true` is load-bearing. sqlite3 exits non-zero when the file is not a database at
  # all, and under `set -e` a failing command substitution takes the script out right there —
  # with the reason captured in this variable and never printed.
  ic="$(sqlite3 "file:$db?mode=ro" 'pragma integrity_check;' 2>&1 | head -1 || true)"
  [ "$ic" = "ok" ] || fail "restored SQLite database failed integrity_check: ${ic:-unreadable}"
  echo "  SQLite integrity_check: ok"

  tables="$(sqlite3 "file:$db?mode=ro" "select count(*) from sqlite_master where type = 'table' and name not like 'sqlite_%';" 2>/dev/null || true)"
  count_sql="$(sqlite3 "file:$db?mode=ro" "select coalesce(group_concat('select count(*) as c from \"' || name || '\"', ' union all '), 'select 0 as c') from sqlite_master where type = 'table' and name not like 'sqlite_%';" 2>/dev/null || true)"
  rows="$(sqlite3 "file:$db?mode=ro" "select coalesce(sum(c), 0) from ($count_sql);" 2>/dev/null || true)"
  case "$tables$rows" in ''|*[!0-9]*) fail "restored SQLite database could not be counted" ;; esac
  echo "  restored contents: $tables table(s), $rows row(s)"

  # An archive taken before the counts were recorded carries no expectation. It still has to
  # contain something, and the downgrade is said out loud rather than applied silently.
  if [ -n "$EXPECT_TABLES" ] || [ -n "$EXPECT_ROWS" ]; then
    [ "$tables" = "$EXPECT_TABLES" ] || fail "restored $tables table(s), archive recorded $EXPECT_TABLES"
    [ "$rows" = "$EXPECT_ROWS" ] || fail "restored $rows row(s), archive recorded $EXPECT_ROWS"
    echo "  matches the archive's recorded contents exactly"
  else
    [ "$tables" -gt 0 ] || fail "restored database has no tables — the copy came back empty"
    echo "  archive predates recorded contents; checked non-empty only"
  fi
}

if [ -n "$SQLITE_ONLINE_REL" ]; then
  verify_sqlite_online
fi

echo "✓ restore verified in isolation"
echo "  retained for inspection: $DEST"
echo "  no live capability path was written"
RESTORE_COMPLETE=1

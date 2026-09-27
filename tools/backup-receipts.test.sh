#!/bin/bash
# tools/doctor's backup section, over a planted HOME, a planted overlay and a planted destination.
#
# The section answers two questions that D10 says nobody was asking: is the last backup recent
# enough for the contract the capability declared, and is the archive its receipt names still AT
# the destination with the byte count the receipt recorded. Both are cheap to get wrong in the
# direction that reports success — a receipt is a file this machine writes about itself, and it
# stays true-looking long after the archive it describes is gone. The vault's first archive
# shipped 704 MB, verified its size on the target, wrote a receipt and could never have been
# restored.
#
# So every red path is planted here and watched. A freshness check nobody has seen fail is the
# thing this file exists to prevent.
#
# Driven against a real checkout, like tools/persistence-orphans.test.sh: handing doctor a partial
# tree makes the section return early on a failed registry call, and every "must be flagged"
# assertion would then pass over an empty report.
set -uo pipefail

_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
source "$_root/tools/lib/test-support.sh"
isolate_axon_env

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

FAKE_HOME="$WORK/home"
OVERLAY="$WORK/overlay"
DEST="$WORK/destination"
mkdir -p "$FAKE_HOME/Library/LaunchAgents" "$OVERLAY/config" "$OVERLAY/backup/receipts" "$DEST/packs"

# `packs` is a real Axon capability with a real backup contract — advise 1 day, stale 2
# (capabilities/packs/service.toml). Using it rather than a synthetic manifest keeps the test on
# the same registry path doctor uses, so a change to how a contract is declared reaches here.
cat > "$OVERLAY/config/machine.toml" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["packs"]
EOF

# A `kind = "local"` destination, which is what this deployment ships to. The receipt names the
# target it SHIPPED to, not whatever the manifest points at today — an overlay that repoints
# `backup-target` must not make doctor look for old archives in the new place.
cat > "$OVERLAY/config/systems.local.toml" <<EOF
[scratch-target]
kind = "local"
path = "$DEST"
EOF

now_stamp() { date -u +%Y%m%dT%H%M%SZ; }
days_ago_stamp() {  # <n>
  date -u -v-"$1"d +%Y%m%dT%H%M%SZ 2>/dev/null || date -u -d "$1 days ago" +%Y%m%dT%H%M%SZ
}

write_receipt() {  # <stamp> <tarball> <bytes>
  cat > "$OVERLAY/backup/receipts/packs.json" <<EOF
{
  "capability": "packs",
  "completed_at": "$1",
  "target": "scratch-target",
  "tarball": "$2",
  "bytes": $3,
  "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "contents": "paths",
  "retention_applied": true
}
EOF
}

# Only the backup section matters. doctor legitimately fails other checks against a planted HOME,
# so asserting on its exit code would be asserting on those instead — the report is captured and
# the assertions read the file.
#
# To a file rather than down a pipe: `doctor | grep -q` returns 141, not 0, because `grep -q`
# exits on the first match and `set -o pipefail` then reports the SIGPIPE. Every assertion in the
# first draft of this file failed that way, including the ones whose line was in the report.
REPORT="$WORK/report.txt"
report() {
  ( cd "$_root" && HOME="$FAKE_HOME" SJEL_OVERLAY_ROOT="$OVERLAY" tools/doctor > "$REPORT" 2>&1 )
  return 0
}
says() { grep -qF "$1" "$REPORT"; }

# --- the green case, so every red one below is a change and not the only outcome -------------
STAMP="$(now_stamp)"
ARCHIVE="packs-$STAMP.tar.gz"
head -c 2361 /dev/zero > "$DEST/packs/$ARCHIVE"
write_receipt "$STAMP" "$ARCHIVE" 2361
report
says "backed up 0.0d ago" || fail "a backup taken seconds ago was not reported fresh"
says "2361 bytes, present at the destination" \
  || fail "an archive that is present with the recorded size was not confirmed"

# --- the archive is gone from the destination ------------------------------------------------
# The receipt is untouched and still says a backup landed. This is the whole point of looking.
mv "$DEST/packs/$ARCHIVE" "$WORK/taken-away.tar.gz"
report
says "is not at the destination" \
  || fail "an archive missing from the destination was still reported as a backup"
mv "$WORK/taken-away.tar.gz" "$DEST/packs/$ARCHIVE"

# --- the archive is there and short ------------------------------------------------------------
# A truncated transfer that lands under the final name looks like a backup. backup.sh refuses to
# rename one; this catches the case where something else shortened it afterwards.
head -c 40 /dev/zero > "$DEST/packs/$ARCHIVE"
report
says "the archive holds 40 bytes, the receipt recorded 2361" \
  || fail "a short archive at the destination was accepted"
head -c 2361 /dev/zero > "$DEST/packs/$ARCHIVE"

# --- the backup stopped happening --------------------------------------------------------------
OLD="$(days_ago_stamp 5)"
write_receipt "$OLD" "$ARCHIVE" 2361
report
says "past its 2d stale threshold" || fail "a five-day-old daily backup was not reported overdue"

# --- nothing has ever landed ---------------------------------------------------------------------
rm -f "$OVERLAY/backup/receipts/packs.json"
report
says "no usable receipt" \
  || fail "a contract with no receipt at all was not reported"

# --- a receipt that cannot be dated is not a fresh backup -------------------------------------
# The near miss: a writer that emits ISO-8601 with separators. Coercing it would date the backup
# and report it fresh, which is worse than refusing it.
write_receipt "2026-09-06T21:07:09Z" "$ARCHIVE" 2361
report
says "no usable receipt" \
  || fail "a receipt with an undateable timestamp was treated as a backup"

if [ "$fails" -gt 0 ]; then
  echo "backup-receipt tests: $fails failure(s)"
  exit 1
fi
echo "backup-receipt tests: fresh, missing archive, short archive, overdue, no receipt and undateable receipt all reported"

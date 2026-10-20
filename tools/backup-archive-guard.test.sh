#!/bin/bash
# The backup producer's link guard, as a test.
#
# The guard exists because tools/restore.sh refuses any archive carrying a symlink, and
# tools/backup.sh used to produce them anyway: the vault shipped a 704 MB archive that verified
# its size, wrote a receipt, reported success, and could never have been restored (2026-08-29).
#
# It was written as `if printf ... | grep -q '^[lhbcps]'`. That form cannot work under
# `set -o pipefail`: `-q` exits at the first match, which closes the pipe while printf is still
# writing, printf dies on SIGPIPE, and pipefail makes the pipeline report 141 — so a MATCH
# evaluated as FALSE and the guard shipped the archive it exists to refuse. Found 2026-09-23 by
# rehearsing a restore of the vault archive; its only trace was a `printf: write error: Broken
# pipe` on stderr that reads like noise. Every archive since 2026-08-29 was unchecked.
#
# So the assertion that matters is behavioural and it runs the guard under pipefail, which is
# what production does: a reintroduced `-q` makes the with-link case exit 0 and this test fails.
# A structural grep for the idiom would be weaker — it would not see the same bug written a
# different way.
#
# The function is EXTRACTED from the real script rather than copied, and the extraction is
# asserted non-empty. A duplicate body would keep passing after the real one changed, which is
# the failure mode this whole test is about.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
SCRATCH="$(mktemp -d /tmp/sjel-backup-guard-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

FUNCTION_SRC="$SCRATCH/verify_archive.sh"
sed -n '/^verify_archive() {/,/^}/p' "$ROOT/tools/backup.sh" > "$FUNCTION_SRC"
grep -q '^verify_archive() {' "$FUNCTION_SRC" \
  || fail "could not extract verify_archive from tools/backup.sh — the guard moved, was renamed, or was removed"

# The archive member check requires ./axon-backup.toml, so both fixtures carry it. That is not
# decoration: it is what keeps a "clean archive" case from passing for the wrong reason.
make_archive() {
  local name="$1" mode="$2" stage="$SCRATCH/stage-$1"
  mkdir -p "$stage"
  printf 'payload\n' > "$stage/real.txt"
  printf 'capability = "synthetic"\n' > "$stage/axon-backup.toml"
  [ "$mode" = "with-link" ] && ln -s /tmp "$stage/outward-link"
  ( cd "$stage" && COPYFILE_DISABLE=1 tar czf "$SCRATCH/$name" . )
}

# Runs the extracted guard exactly as backup.sh runs it: under pipefail, with stderr captured.
# OUT and STATUS are set for the caller.
run_guard() {
  OUT="$(bash -c 'set -uo pipefail; source "$1"; verify_archive "$2"' _ "$FUNCTION_SRC" "$1" 2>&1)"
  STATUS=$?
}

make_archive with-link.tar.gz with-link
run_guard "$SCRATCH/with-link.tar.gz"
[ "$STATUS" -ne 0 ] || fail "a symlink in the archive was accepted — the guard is inverted or absent"
case "$OUT" in
  *"link or special file"*) ;;
  *) fail "refused, but did not say why. Output: $OUT" ;;
esac
case "$OUT" in
  *outward-link*) ;;
  *) fail "refused without naming the offending member, so the operator cannot act. Output: $OUT" ;;
esac
# The exact symptom of the inversion. Asserting on it turns a silent success into a named failure
# if anyone reintroduces an early-closing reader.
case "$OUT" in
  *"Broken pipe"*) fail "the guard closed its own pipe — a `grep -q`/`head` early exit is back. Output: $OUT" ;;
esac

make_archive clean.tar.gz clean
run_guard "$SCRATCH/clean.tar.gz"
[ "$STATUS" -eq 0 ] || fail "a link-free archive was refused: $OUT"

echo "backup archive guard: a symlink is refused and named, a clean archive passes"

#!/bin/bash
# Tests for tools/lib/capability-index.sh — the capabilities half of `sjel search` (ISA ISC-33).
#
# Driven against a planted checkout, so each case describes one shape. The last block asks the
# real repository the question ISC-33 states: does `mail` find comms.
#
# `sjel search` itself cannot run in CI, because tools/capability.sh registry hard-fails without
# a machine.toml. The library reads the registry from stdin, so these cases feed it directly.
set -uo pipefail

LIB="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib/capability-index.sh"
REAL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
out=""
status=0

note_fail() {
  echo "FAIL: $1"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
}

# shellcheck source=lib/capability-index.sh
. "$LIB"
SJEL_ROOT="$SCRATCH/fixture"
SJEL_OVERLAY_CAPS_DIR="$SCRATCH/overlay/capabilities"

REGISTRY="$(printf 'inbox\tprocess\nclock\tprocess\nsecret-one\tprocess\nbare\tprocess\n')"

find_in() { # find_in <query> -> $out, $status
  out="$(printf '%s\n' "$REGISTRY" | search_capabilities "$1")"
  status=$?
}

says() { printf '%s' "$out" | grep -qF -- "$1"; }

# --- the fixture checkout --------------------------------------------------

mkdir -p "$SJEL_ROOT/capabilities/inbox/src/server" "$SJEL_ROOT/capabilities/clock/src" \
  "$SJEL_ROOT/capabilities/bare" "$SJEL_OVERLAY_CAPS_DIR/secret-one"

# A README that opens with blank lines, as four real ones do, and says "backup" only below
# its opening paragraph.
cat > "$SJEL_ROOT/capabilities/inbox/README.md" <<'MD'


# inbox

Reads the postbox and proposes what to keep.

## Backups

The store has a backup contract.
MD

# A route manifest split over lines, the way rustfmt writes a long call.
cat > "$SJEL_ROOT/capabilities/inbox/src/server/main.rs" <<'RS'
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "POST",
        "/letters/{id}/shred",
        "Shred one letter for good.",
    ),
];
RS

cat > "$SJEL_ROOT/capabilities/clock/README.md" <<'MD'
# clock

Tells the time.
MD
cat > "$SJEL_ROOT/capabilities/clock/src/main.rs" <<'RS'
const ROUTES: &[Route] = &[r("GET", "/now", "The time, in \"UTC\".")];
RS

cat > "$SJEL_OVERLAY_CAPS_DIR/secret-one/README.md" <<'MD'
# secret-one

A private capability that keeps its README in the overlay.
MD

# --- matching --------------------------------------------------------------

find_in postbox
[ "$status" -eq 0 ] || note_fail "a word from the README's opening paragraph should match"
says "inbox" || note_fail "the matching capability is not named"
says "Reads the postbox and proposes what to keep." || note_fail "the summary is not the README's own line"

find_in shred
says "inbox" || note_fail "a route split over lines should match"
says "POST /letters/{id}/shred  Shred one letter for good." || note_fail "the matching route is not listed"

find_in utc
says "clock" || note_fail "a route description with escaped quotes should match"

find_in TIME
says "clock" || note_fail "matching should ignore case"

find_in private
says "secret-one" || note_fail "an overlay capability's README should be read"

find_in bare
says "bare" || note_fail "a capability with no README or source should still match by name"

find_in backup
if says "inbox"; then note_fail "a word below the opening paragraph should not match"; fi

find_in nothing-says-this
[ "$status" -ne 0 ] || note_fail "no match should exit non-zero"
[ -z "$out" ] || note_fail "no match should print nothing"

# --- the real repository ---------------------------------------------------

SJEL_ROOT="$REAL_ROOT"
unset SJEL_OVERLAY_CAPS_DIR
out="$(printf 'comms\tprocess\ncalendar\tprocess\n' | search_capabilities mail)"
printf '%s' "$out" | grep -q '^  comms ' || note_fail "the real index does not find comms for 'mail' (ISC-33)"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "capability-index.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "capability-index.test.sh: all cases passed"

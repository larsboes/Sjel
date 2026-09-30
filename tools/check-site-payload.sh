#!/bin/bash
# check-site-payload.sh — the gate in front of publishing (#168).
#
# tools/check-publication-hygiene.sh scans the Git INDEX: what a commit would carry into the
# public repository. That is the right question for tracked files and the wrong one for this
# site, because the demo's fixtures are never committed — they are recorded into an untracked
# directory minutes before being uploaded, so the index has never seen them. This scans the
# assembled bytes instead, at the last moment they can still be stopped.
#
# WHICH FAILURE THIS ACTUALLY PREVENTS. In CI it prevents almost nothing, and that is fine:
# a runner has no overlay, no vault and no real database, so the fixtures are synthetic by
# construction. The dangerous run is the LOCAL one — a machine where a real overlay is a
# directory away, the real database is a file the demo's capabilities would open if
# SJEL_DB_PATH were not pointed elsewhere, and `tools/demo-site` is one command. There, "synthetic by construction" rests on three guards
# holding, and this is what catches the case where one of them did not.
#
# Two families of marker, for two different reasons:
#
#   STRUCTURAL   things that are never legitimate on a public page whoever ran the build —
#                a real email address, an IBAN, a workstation home path, a tailnet name, a
#                private address. Written as patterns, so they need no personal data in this
#                tracked file to detect personal data.
#   DERIVED      terms read out of the active overlay at run time — its directory name, its
#                git remote, its machine names, the hosts in its systems file. Never written
#                down, never cached, and empty in CI where there is no overlay to read.
#
#   tools/check-site-payload.sh <dir>
set -euo pipefail

DIR="${1:-site}"
[ -d "$DIR" ] || { echo "check-site-payload: no such directory: $DIR" >&2; exit 2; }

failed=0
hits=0

report() {  # report <what> <matches>
  echo "site payload: $1" >&2
  printf '%s\n' "$2" | sed 's/^/    /' | head -10 >&2
  failed=1
}

scan() {  # scan <description> <extended-regex> [<filter-out-regex>]
  local what="$1" pattern="$2" allow="${3:-}"
  local found
  found="$(grep -rIEn --exclude='*.map' -- "$pattern" "$DIR" 2>/dev/null || true)"
  if [ -n "$allow" ] && [ -n "$found" ]; then
    found="$(printf '%s\n' "$found" | grep -Ev -- "$allow" || true)"
  fi
  if [ -n "$found" ]; then
    hits=$((hits + 1))
    report "$what" "$found"
  fi
}

# ─── Structural markers ───────────────────────────────────────────────────────

# RFC 2606 reserves example.com/net/org and .invalid/.test/.example for documentation, which
# is exactly what a demo address is. Anything else that parses as an address is a real one
# until proven otherwise, and a public page is not the place to work out which.
scan "an email address outside the reserved documentation domains" \
  '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' \
  '@(example\.(com|net|org)|[A-Za-z0-9.-]+\.(invalid|test|example))'

# Country code, check digits, then a BBAN of 11 to 30 characters — written either as one run
# or in the spaced groups of four people actually copy out of a banking app. The repeat count
# has to cover the whole BBAN: an earlier version stopped after three groups and then failed
# its own \b against the rest of a German IBAN, matching nothing at all.
scan "something shaped like an IBAN" \
  '\b[A-Z]{2}[0-9]{2}( ?[A-Z0-9]{4}){2,7}( ?[A-Z0-9]{1,3})?\b'

# A checkout path names the person whose machine it is, and a build product that quotes one
# says where the page was made. CI's own homes are portable public examples.
scan "a workstation home path" \
  '/(Users|home)/[A-Za-z0-9._-]+/' \
  '/home/(runner|agent|node)/'

# A MagicDNS name is a private topology fact and, on its own, a routable address.
scan "a tailnet hostname" '[A-Za-z0-9-]+\.ts\.net'

# RFC1918. 127.0.0.1 is deliberately not here: loopback names no host and appears honestly in
# a recorded health URL.
#
# The boundaries are not `\b`, and that is a measurement rather than a tidy-up. The minifier
# vite 8 brought with it (oxc, under rolldown) folds an array of two-character hex strings into
# one dot-delimited literal followed by `.split(`.`)`, and three.js ships exactly such an array
# -- its 256-entry `_lut`, which builds UUIDs. Folded, that literal reads
# `00.01.02....0d.0e.0f.10.11.12.13.14..., so `10.11.12.13` is three consecutive table entries
# and parses as an RFC1918 address. `\b` matched it and did not care that the dotted run
# continues, so this gate failed the Pages build on 2026-09-30 on a payload containing no
# address at all (see the regression case in check-site-payload.test.sh).
#
# The guards below require the address to stand alone: no hex digit or dot before it, and
# nothing dot-hex after it -- while still allowing a sentence-ending `.` when a non-hex
# character follows. A real address is written `//10.0.0.5:8080`, `"192.168.1.42"` or
# `=172.16.0.1,`, and none of those is a dot-hex continuation.
#
# The narrowing this buys, stated so it is not discovered: a dotted run of five or more octets
# is never reported, because that is a version string or a table rather than a host.
scan "a private network address" \
  '(^|[^0-9a-fA-F.])(10\.[0-9]{1,3}|192\.168|172\.(1[6-9]|2[0-9]|3[01]))\.[0-9]{1,3}\.[0-9]{1,3}([^.]|\.[^0-9a-fA-F]|$)'

# The same deployment-instance names tools/check-publication-hygiene.sh rejects from the index.
# Same trailing-character guard as tools/check-publication-hygiene.sh, and for the same
# reason: a journal tag named axon-personal-cents names no deployment.
scan "a deployment-instance marker" \
  '(axon-personal|axon-family|axon-work|lifeos-mono|obsidian-mono)([^-A-Za-z0-9]|$)'

# ─── Derived markers ──────────────────────────────────────────────────────────
#
# Read from whichever overlay this shell resolves. Skipped entirely when that is the demo
# overlay (nothing in it is private) or when there is none (CI).

derived_terms() {
  local overlay="${SJEL_OVERLAY_ROOT:-}"
  [ -n "$overlay" ] && [ -d "$overlay" ] || return 0
  case "$overlay" in *"/demo/overlay") return 0 ;; esac

  basename "$overlay"
  # The overlay's remote, and its repository name on its own — a fixture can carry either.
  if [ -d "$overlay/.git" ]; then
    local remote
    remote="$(git -C "$overlay" remote get-url origin 2>/dev/null || true)"
    if [ -n "$remote" ]; then
      printf '%s\n' "$remote"
      printf '%s\n' "${remote##*/}" | sed 's/\.git$//'
    fi
  fi
  # Machine names are private facts (tools/lib/paths.sh says so) and name real hosts.
  local m
  for m in "$overlay"/config/machines/*.toml; do
    [ -e "$m" ] || continue
    basename "$m" .toml
  done
  # Every host the overlay knows how to reach.
  cat "$overlay"/config/systems*.toml 2>/dev/null |
    grep -Eo 'https?://[A-Za-z0-9._:-]+' | sed 's|https\?://||' | sort -u
}

SJEL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# A term that is ALREADY in this public repository cannot be leaked by publishing it again.
#
# Without this the check fires on its own footer: an overlay's systems file legitimately names
# github.com, github.com is on every generated page, and the build fails for a fact the world
# already has. Filtering against the tracked tree is the general form of that — it keeps the
# derived list to terms that are genuinely private, with no allowlist in here to maintain and
# no chance of an allowlist quietly excusing something that matters. A private host, a personal
# domain or a public-but-personal IP address is exactly what survives it.
already_public() {  # already_public <term>
  git -C "$SJEL_ROOT" grep -qIF -- "$1" 2>/dev/null
}

while IFS= read -r term; do
  [ -n "$term" ] || continue
  # Two characters is not a term, it is a coincidence waiting to fail a build.
  [ "${#term}" -ge 4 ] || continue
  already_public "$term" && continue
  found="$(grep -rIFn --exclude='*.map' -- "$term" "$DIR" 2>/dev/null || true)"
  if [ -n "$found" ]; then
    hits=$((hits + 1))
    # The term itself is the private thing, so it is NOT echoed — only where it was found.
    report "a term derived from the active overlay appears in the payload" \
      "$(printf '%s\n' "$found" | cut -d: -f1,2 | sort -u)"
  fi
done < <(derived_terms)

if [ "$failed" -ne 0 ]; then
  echo "" >&2
  echo "check-site-payload: $hits marker(s) matched in $DIR — refusing to publish." >&2
  echo "Nothing was uploaded. Fix the generator or the seed data, not this list." >&2
  exit 1
fi

echo "site payload passed ($(find "$DIR" -type f | wc -l | tr -d ' ') files in $DIR)"

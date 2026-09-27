#!/bin/bash
# Planted-tree regression tests for check-presentations-theme-contract.sh.
#
# The gate's whole value is that it fails when the two role lists diverge, and a gate that
# cannot fail looks exactly like one that finds nothing wrong. So every case here plants its
# own pair of theme.py files (and its own themes directory) via the SJEL_* overrides, and the
# last case runs the gate over the REAL pack so a fix that only satisfies the fixtures — or a
# path that drifted after an edit — is caught too.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-presentations-theme-contract.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

# A minimal stand-in for each real file: the gate only reads the two assignments.
plant() { # plant <case> <deck-roles-csv> <diagram-roles-csv> -> echoes the case root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/themes"
  local deck_tuples diagram_tuples role
  for role in ${2//,/ }; do deck_tuples="$deck_tuples\"$role\", "; done
  for role in ${3//,/ }; do diagram_tuples="$diagram_tuples\"$role\", "; done
  printf 'REQUIRED_ROLES = (\n    %s\n)\n' "${deck_tuples%, }" > "$root/deck.py"
  printf 'NEEDED = (%s)\n' "${diagram_tuples%, }" > "$root/diagram.py"
  # A theme defining exactly the deck set, so only the subset relation is under test.
  local entries=""
  for role in ${2//,/ }; do entries="$entries\"$role\": \"AABBCC\", "; done
  printf '{"palette": {%s}}\n' "${entries%, }" > "$root/themes/t.json"
  printf '%s' "$root"
}

run() { # run <case-root> -> output in $out, status in $status
  out=$(SJEL_DECK_THEME="$1/deck.py" SJEL_DIAGRAM_THEME="$1/diagram.py" \
        SJEL_THEMES_DIR="$1/themes" "$CHECK" 2>&1)
  status=$?
}

expect() { # expect <description> <want-status> <case-root> [substring]
  local desc="$1" want="$2" root="$3" want_text="${4:-}"
  run "$root"
  if [ "$status" -ne "$want" ]; then
    echo "FAIL: $desc — wanted exit $want, got $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
    return
  fi
  if [ -n "$want_text" ] && ! printf '%s' "$out" | grep -qF "$want_text"; then
    echo "FAIL: $desc — output does not name '$want_text':"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# ── the green path, so a gate that fails everything cannot pass this file ──────────────
root=$(plant holds "ink,white,accent,paper" "ink,white,paper")
expect "a subset relation holds" 0 "$root"

# ── the class the gate exists for: the live `paper` bug, reproduced ────────────────────
# diagramkit required paper, deckkit did not. Both shipped themes happened to define it, so
# nothing failed until a user derived a theme. This is that shape.
root=$(plant paper-divergence "ink,white,accent" "ink,white,paper")
expect "a role diagramkit needs but deckkit does not require is caught" 1 "$root" "paper"
expect "and the failure says why it matters" 1 "$root" "build a deck and then fail"

# ── the reverse direction is NOT drift, and must not be reported as such ───────────────
# deckkit requiring more than diagramkit needs is the design: a diagram needs no caution
# tint. A gate that flagged this would be red on the correct pack.
root=$(plant deck-broader "ink,white,accent,caution" "ink,white")
expect "a deck-only role is fine" 0 "$root"

# ── end to end: a shipped theme that cannot satisfy both ───────────────────────────────
root=$(plant theme-missing-role "ink,white,accent" "ink,white")
printf '{"palette": {"ink": "AABBCC", "white": "FFFFFF"}}\n' > "$root/themes/t.json"
expect "a theme missing a required role fails even when the lists agree" 1 "$root" "missing required roles"

# ── silent green ───────────────────────────────────────────────────────────────────────
# An unparseable list must FAIL. A regex matching nothing would compare two empty sets,
# report a clean subset, and pass — the failure mode every gate here exists to prevent.
root=$(plant unparseable "ink,white" "ink")
printf 'DIFFERENT_NAME = ("ink",)\n' > "$root/diagram.py"
expect "an unparseable list fails rather than comparing empty sets" 1 "$root" "no \`NEEDED = (\` assignment"

root=$(plant no-themes "ink,white" "ink")
rm -f "$root/themes/t.json"
expect "no themes found fails rather than passing over nothing" 1 "$root" "the walk is broken"

# ── setup errors do not blame the pack ─────────────────────────────────────────────────
root=$(plant missing-file "ink,white" "ink")
rm -f "$root/diagram.py"
expect "a missing source file is a setup error" 2 "$root"

# ── the real pack, end to end ──────────────────────────────────────────────────────────
out=$("$CHECK" 2>&1)
status=$?
if [ "$status" -ne 0 ]; then
  echo "FAIL: the real presentations pack should satisfy its own contract, got exit $status:"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
fi

if [ "$fails" -gt 0 ]; then
  echo "check-presentations-theme-contract.test.sh: $fails case(s) failed."
  exit 1
fi
echo "check-presentations-theme-contract.test.sh: all cases passed."

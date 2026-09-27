#!/bin/bash
# check-presentations-theme-contract.sh — the palette contract gate (CI: repo gates).
#
# Two skills in the presentations Pack derive their rendering theme from the same palette
# file. `deckkit` requires a set of roles for a deck to build; `diagramkit` requires a set
# for a diagram to render. The invariant is one-directional and it is the whole point:
#
#     every role diagramkit requires must be one deckkit's themes define
#
# Break it and a theme builds a deck happily and then fails the moment a diagram is rendered
# from that same palette. That is not hypothetical — it was live. `paper` was required by
# diagramkit and not by deckkit, both shipped themes happened to define it, and so the
# divergence only ever bit a user who derived a theme by following `theming.md`, which is
# what that file tells them to do. It sat unnoticed until a Packs review on 2026-09-17.
#
# This gate exists instead of a shared file. The two lists used to live in one
# `Packs/presentations/shared/palette-roles.json`, which made them identical BYTES — but it
# also made the SOURCE skill incomplete (the file was absent in the repo and only
# materialized by the deployer), and it was copied into all four skills of the Pack when only
# two read it. What matters is that the lists AGREE, and that is checkable. A skill must be
# able to RUN from its own directory; it may still POINT at a sibling by name for material it
# merely reads.
#
# Three properties, all proven in tools/check-presentations-theme-contract.test.sh:
#   - diagramkit's needs are a subset of deckkit's requirements
#   - every SHIPPED theme defines every required role, so the invariant holds end to end
#     rather than only on paper
#   - a list that cannot be parsed is a FAILURE, not an empty list. A regex that silently
#     matched nothing would report a clean subset relation between two empty sets, which is
#     the silent-green failure every gate in this directory exists to prevent.
#
# Paths are overridable so the test can plant a divergent pair; the real ones are the default.
#
# Exit 0 = the contract holds · 1 = it is broken · 2 = setup error (no python3, no such file).
# bash 3.2-safe, no git, no network.
set -u

_here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"   # SJEL_ROOT

DECK_THEME="${SJEL_DECK_THEME:-$SJEL_ROOT/Packs/presentations/skills/slide-deck/scripts/deckkit/theme.py}"
DIAGRAM_THEME="${SJEL_DIAGRAM_THEME:-$SJEL_ROOT/Packs/presentations/skills/diagrams/scripts/diagramkit/theme.py}"
THEMES_DIR="${SJEL_THEMES_DIR:-$SJEL_ROOT/Packs/presentations/skills/slide-deck/assets/themes}"

if ! command -v python3 >/dev/null 2>&1; then
  echo "check-presentations-theme-contract: python3 is not on PATH — setup error, not a finding." >&2
  exit 2
fi
for f in "$DECK_THEME" "$DIAGRAM_THEME"; do
  if [ ! -r "$f" ]; then
    echo "check-presentations-theme-contract: cannot read $f" >&2
    echo "  A moved theme.py is a broken gate, not a passing one." >&2
    exit 2
  fi
done

python3 - "$DECK_THEME" "$DIAGRAM_THEME" "$THEMES_DIR" <<'PY'
import glob, json, os, re, sys

deck_path, diagram_path, themes_dir = sys.argv[1], sys.argv[2], sys.argv[3]
problems = []
setup = []


def roles_from(path, name):
    """The string literals of `name = ( ... )`, by scanning the balanced parentheses.

    Not a one-line regex: the tuples are written across lines and one is wrapped, and a
    regex that quietly matched nothing would compare two empty sets and pass.
    """
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as exc:
        setup.append(f"cannot read {path}: {exc}")
        return None
    m = re.search(re.escape(name) + r"\s*=\s*\(", text)
    if not m:
        problems.append(f"{path}: no `{name} = (` assignment found")
        return None
    start = m.end() - 1
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                found = re.findall(r'"([A-Za-z_][A-Za-z0-9_]*)"', text[start:i])
                if not found:
                    problems.append(f"{path}: `{name}` parsed as empty — the gate cannot check nothing")
                    return None
                return found
    problems.append(f"{path}: `{name}` is never closed")
    return None


deck = roles_from(deck_path, "REQUIRED_ROLES")
diagram = roles_from(diagram_path, "NEEDED")

if setup:
    for line in setup:
        print(f"check-presentations-theme-contract: {line}", file=sys.stderr)
    sys.exit(2)

if deck is not None and diagram is not None:
    missing = [role for role in diagram if role not in deck]
    if missing:
        problems.append(
            "diagramkit requires role(s) deckkit's themes need not define: "
            + ", ".join(missing)
            + "\n  A theme could build a deck and then fail to render as a diagram."
            + "\n  Add them to REQUIRED_ROLES in " + os.path.basename(deck_path) + "."
        )

    themes = sorted(glob.glob(os.path.join(themes_dir, "*.json")))
    if not themes:
        problems.append(f"no themes found in {themes_dir} — the walk is broken, not the pack")
    for theme_path in themes:
        try:
            palette = (json.load(open(theme_path, encoding="utf-8")) or {}).get("palette") or {}
        except json.JSONDecodeError as exc:
            problems.append(f"{theme_path} is not valid JSON: {exc}")
            continue
        absent = [role for role in deck if role not in palette]
        undrawable = [role for role in diagram if role not in palette]
        if absent:
            problems.append(f"{os.path.basename(theme_path)} is missing required roles: {', '.join(absent)}")
        if undrawable:
            problems.append(
                f"{os.path.basename(theme_path)} cannot be rendered as a diagram — missing: {', '.join(undrawable)}"
            )

if problems:
    print("check-presentations-theme-contract: the palette contract is broken.", file=sys.stderr)
    for line in problems:
        print(f"  {line}", file=sys.stderr)
    print(f"\n  deckkit requires : {', '.join(deck or [])}", file=sys.stderr)
    print(f"  diagramkit needs : {', '.join(diagram or [])}", file=sys.stderr)
    sys.exit(1)

print(
    f"palette contract holds: {len(diagram)} diagram role(s) all defined by "
    f"deckkit's {len(deck)} required, across {len(glob.glob(os.path.join(themes_dir, '*.json')))} shipped theme(s)."
)
PY

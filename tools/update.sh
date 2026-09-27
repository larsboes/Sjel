#!/bin/bash
# tools/update.sh — the maintainer half of tools/install.sh. Fast-forward
# this checkout to origin/main and re-run tools/doctor as a post-update
# sanity check. Never force-pushes/rebases/resets: a diverged checkout
# (local commits ahead AND behind) is left alone with instructions, not
# silently discarded. Every Axon deployment, current or future, runs this
# the same way — there is no fleet/central-registry concept; each checkout
# maintains itself, per
# `CONTRIBUTING.md#documentation-stays-owned-and-current`'s bring-your-
# own-check philosophy.
#
#   tools/update.sh            # fetch + ff-only pull + doctor
#   tools/update.sh --check    # fetch + version summary + incoming preview, never pulls
#   tools/update.sh --no-pull  # fetch + report only, don't pull
#   tools/update.sh --yes      # answer the confirm up front (also: SJEL_ASSUME_YES=1)
#   tools/update.sh -h         # this help
#
# Interactive: when stdin is a TTY, the pull path shows the incoming preview
# and asks before fast-forwarding. Non-TTY (CI, cron, pipes) never prompts
# and pulls straight through, exactly as before. --yes shows the preview and
# proceeds without asking, so automation that allocates a PTY -- which looks
# like a TTY and would otherwise block on the prompt forever -- has a way
# through that does not depend on hiding the terminal.
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SJEL_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"

# paths.sh (which sources toml.sh) — it resolves this machine's overlay and exports
# SJEL_MACHINE_TOML, where the enabled capability set moved on 2026-07-26. See
# schemas/machine.toml.example.
source "$TOOLS_DIR/lib/paths.sh"
# The categorized version-to-version delta (capabilities/upstreams/toolchain/commits), shared
# with tools/release so the "what changed" view and the release notes never drift (CONTRIBUTING.md#documentation-stays-owned-and-current).
source "$TOOLS_DIR/lib/delta.sh"

NO_PULL=0; CHECK=0
# Env default so a wrapper can set it once for a whole automated run; the flag still wins.
ASSUME_YES=0
[ "${SJEL_ASSUME_YES:-0}" = "1" ] && ASSUME_YES=1
# One loop over every argument, not three tests against $1: --check --yes together used to
# depend on which flag was typed first, and an unrecognised flag was accepted in silence.
for _a in "$@"; do
  case "$_a" in
    --no-pull)  NO_PULL=1 ;;
    --check)    CHECK=1 ;;
    --yes|-y)   ASSUME_YES=1 ;;
    -h|--help)  sed -n '2,23p' "$0"; exit 0 ;;
    *) echo "update.sh: unknown argument '$_a' — see --help" >&2; exit 1 ;;
  esac
done

# Incoming preview: the categorized delta the pull would bring (capabilities/upstreams/
# toolchain/commits, via tools/lib/delta.sh) plus which ENABLED capabilities (machine.toml, via
# toml.sh) it touches — capabilities are the churny surface, so that's the "does this update
# concern me" signal. The delta targets origin/main because that is what update.sh actually
# fast-forwards to; the newest RELEASE tag is shown separately as a version-identity line (a tag
# is a label to stand next to, not what the pull merges). Shown by --check and the interactive
# confirm.
# Every delta below is computed against origin/main, so resolve it ONCE. A usage install
# --- `git clone --depth 1 --branch <tag>`, which is what tools/install.sh's usage profile
# and the one-line installer produce --- has no origin/main ref at all: its refspec is
# `+refs/tags/<tag>:refs/tags/<tag>`, so no branch was ever fetched. Without this gate the
# absence surfaces as four separate raw `fatal:` lines plus two summary lines with their
# numbers missing, which is a poor first thing for a new install to print.
have_origin_main() { git rev-parse --verify --quiet origin/main >/dev/null 2>&1; }

# The honest promotion path, verified rather than assumed. `git fetch --unshallow` ALONE
# does not help here: it deepens history along a refspec that names no branch, so
# origin/main is still absent afterwards. Widening the refspec is the part that matters.
print_promotion_hint() {
  echo "  This is a usage install pinned to a tag, so there is no origin/main to compare"
  echo "  against — version identity still works, the delta does not. Promote it to a"
  echo "  development checkout that can compute one:"
  echo
  echo "    git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'"
  echo "    git fetch --unshallow origin"
}

# The overlay is absent between cloning and running tools/install.sh, which is the normal
# state for a fresh checkout — not an error, but toml_array's grep says so on stderr. Report
# the state the way tools/doctor already does for the same absence.
enabled_capabilities() {
  [ -f "$SJEL_MACHINE_TOML" ] || return 0
  toml_array capabilities "$SJEL_MACHINE_TOML"
}

have_overlay() { [ -f "$SJEL_MACHINE_TOML" ]; }

print_incoming() {
  echo
  echo "What the update brings (HEAD..origin/main):"
  echo
  if ! have_origin_main; then
    print_promotion_hint
    return 0
  fi
  print_manifest_delta HEAD origin/main
  echo
  echo "Enabled capabilities with incoming changes:"
  if ! have_overlay; then
    echo "  (no overlay configured yet — run tools/install.sh)"
    return 0
  fi
  CAP_INCOMING=0
  while IFS= read -r cap; do
    [ -n "$cap" ] || continue
    # Three-dot on purpose: merge-base→origin/main = what origin actually
    # brings in. Two-dot (endpoint diff) would list OUR local commits'
    # files as "incoming" on any checkout that's ahead.
    if [ -n "$(git diff --name-only HEAD...origin/main -- "capabilities/$cap/")" ]; then
      echo "  $cap"
      CAP_INCOMING=1
    fi
  done <<EOF
$(enabled_capabilities)
EOF
  if [ "$CAP_INCOMING" -eq 0 ]; then
    echo "  (no enabled capability affected)"
  fi
}

echo "Axon update · $SJEL_ROOT"
echo

cd "$SJEL_ROOT"

# --check never pulls, so the dirty-tree guard (which only protects the
# pull) would just block a read-only status question — skip it there.
if [ "$CHECK" -eq 0 ] && [ -n "$(git status --porcelain)" ]; then
  echo "update.sh: working tree not clean — commit or stash before updating." >&2
  git status --short >&2
  exit 1
fi

echo "Fetching origin/main..."
# A tag-pinned clone has no branch in its refspec, so this fetch cannot create origin/main
# and must not be treated as a failure. `|| true` under `set -e`, and the gate below decides.
git fetch --quiet origin main 2>/dev/null || true

if have_origin_main; then
  read -r AHEAD BEHIND <<<"$(git rev-list --left-right --count HEAD...origin/main)"
  echo "  $AHEAD ahead, $BEHIND behind origin/main"
else
  # Deliberately NOT "0 ahead, 0 behind": unknown is not the same as in sync, and printing
  # the latter would tell a pinned usage install it is current when nothing was compared.
  echo "  (delta unavailable — no origin/main in this checkout)"
fi

if [ "$CHECK" -eq 1 ]; then
  echo
  # Version identity survives a shallow tag clone — `git describe` reads the tag that is
  # present — so these lines stay useful even when nothing can be compared.
  echo "  installed: $(describe_release) ($(git log -1 --format=%cs))"
  if have_origin_main; then
    echo "  latest:    $(describe_release origin/main) ($(git log -1 --format=%cs origin/main)) — origin/main"
  fi
  # Release-aware identity: once tags exist, say where this checkout sits relative to the newest
  # release, not only relative to the moving main branch. Silent when no release has been cut yet.
  LATEST_TAG="$(latest_release_ref)"
  [ -n "$LATEST_TAG" ] && echo "  release:   $LATEST_TAG ($(git log -1 --format=%cs "$LATEST_TAG")) — newest release tag"
  print_incoming
  echo
  echo "Check only (--check) — not pulling."
  exit 0
fi

# Past this point every path needs a ref to fast-forward TO, so stop with the promotion
# instructions rather than failing four commands deep.
if ! have_origin_main; then
  echo
  print_promotion_hint
  exit 1
fi

if [ "$BEHIND" -eq 0 ]; then
  echo
  echo "Already up to date."
  exit 0
fi

if [ "$AHEAD" -gt 0 ]; then
  echo
  echo "update.sh: diverged (local commits ahead AND behind) — not auto-merging." >&2
  echo "Resolve by hand: git log HEAD..origin/main / git merge origin/main" >&2
  exit 1
fi

if [ "$NO_PULL" -eq 1 ]; then
  echo
  echo "$BEHIND commit(s) available (--no-pull given, not pulling)."
  exit 0
fi

# Interactive confirm — TTY only. Non-TTY (CI, cron, `echo | update.sh`)
# skips both preview and prompt and pulls straight through, byte-compatible
# with the pre-interactive behavior (same guard idiom as install.sh's
# capability prompt: `read` on a closed stdin would EOF non-zero under
# `set -e`).
if [ "$ASSUME_YES" -eq 1 ]; then
  print_incoming
  echo
  echo "Confirmed up front (--yes / SJEL_ASSUME_YES) — fast-forwarding."
elif [ -t 0 ]; then
  print_incoming
  echo
  read -r -p "Fast-forward now? [y/N]: " CONFIRM
  case "$CONFIRM" in
    y|Y) ;;
    *)
      echo "not pulling."
      exit 0
      ;;
  esac
fi

echo
echo "Fast-forwarding..."
OLD_HEAD="$(git rev-parse HEAD)"   # before the pull, for the changed-capabilities report
git merge --ff-only origin/main

# Report which ENABLED capabilities moved in this update — capabilities are the
# churny surface, so a compact "what changed" summary is more useful than the
# raw log. Only runs on the pull path (BEHIND==0 and --no-pull both exit above),
# so --no-pull naturally skips it. Read via toml.sh; heredoc (not a pipe) keeps
# the loop in this shell so CHANGED survives.
echo
echo "Enabled capabilities changed in this update:"
if ! have_overlay; then
  echo "  (no overlay configured yet — run tools/install.sh)"
else
  CHANGED=0
  while IFS= read -r cap; do
    [ -n "$cap" ] || continue
    changed_files="$(git diff --name-only "$OLD_HEAD"..HEAD -- "capabilities/$cap/")"
    if [ -n "$changed_files" ]; then
      echo "  $cap:"
      echo "$changed_files" | head | sed 's/^/    /'
      CHANGED=1
    fi
  done <<EOF
$(enabled_capabilities)
EOF
  if [ "$CHANGED" -eq 0 ]; then
    echo "  (no enabled capability changed)"
  fi
fi

echo
echo "Re-running tools/doctor to confirm the update didn't break anything:"
echo
exec "$TOOLS_DIR/doctor"

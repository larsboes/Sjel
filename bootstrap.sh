#!/bin/bash
# bootstrap.sh — the one-line installer's entry point, and the only Axon script that runs
# before there is a checkout. Decided in #2; everything it does is deliberately small.
#
#   curl -fsSL https://raw.githubusercontent.com/larsboes/Sjel/<tag>/bootstrap.sh | bash
#
# Fetch it BY TAG, never from main: a pipe-to-bash of a moving branch executes whatever that
# branch says today, and the whole point of a release line is that an install is reproducible.
#
# What it does, and nothing else: check the host has git, resolve the newest release tag,
# clone at it in the profile you pick, and hand off to tools/install.sh. Every real decision
# — which overlay, which capabilities, which harness — stays in the repository, where it is
# reviewable and tested. This file is the part that cannot be, so it stays trivial.
#
#   SJEL_REF=v1.2.3      clone that tag instead of the newest release
#   SJEL_DIR=~/src/Axon  clone here instead of ./Axon
#   SJEL_PROFILE=usage   answer the profile question up front (usage | development)
#
# bash 3.2-safe.
#
# Every line below is a definition. The single call is the last line of the file, so a
# download truncated mid-transfer defines some functions and then reaches EOF without ever
# running one — the failure mode that makes piping a URL into a shell worth doing carefully.
set -euo pipefail

SJEL_REPO="${SJEL_REPO:-larsboes/Sjel}"
# Split from SJEL_REPO so the clone URL has one home. Overridable so bootstrap.test.sh can
# point both profiles at a scratch remote and exercise the real cloning rather than a mock.
SJEL_REMOTE="${SJEL_REMOTE:-https://github.com/$SJEL_REPO.git}"

say()  { printf '%s\n' "$*"; }
die()  { printf 'bootstrap: %s\n' "$*" >&2; exit 1; }

require_git() {
  command -v git >/dev/null 2>&1 || die "git is required and was not found.
  Axon resolves its own version from the checkout (git describe) and updates by fast-forward,
  so git is a hard requirement rather than a convenience — see #2."
}

# The newest release tag, asked of the API rather than embedded here. Embedding it would make
# this file a second home for the version and something to remember at every release.
resolve_ref() {
  if [ -n "${SJEL_REF:-}" ]; then printf '%s' "$SJEL_REF"; return 0; fi
  command -v curl >/dev/null 2>&1 || die "curl is required to resolve the newest release (or set SJEL_REF)."
  curl -fsSL "https://api.github.com/repos/$SJEL_REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -1
}

# usage = a pinned tag you run; development = the full history you change. The difference is
# only the clone; both land in the same tools/install.sh.
ask_profile() {
  if [ -n "${SJEL_PROFILE:-}" ]; then printf '%s' "$SJEL_PROFILE"; return 0; fi
  # Piped into bash, stdin is the script itself — so the prompt reads the terminal directly.
  # Without a terminal there is nothing to ask, and usage is the safe default to assume.
  if [ ! -r /dev/tty ]; then printf 'usage'; return 0; fi
  say "" >&2
  say "  usage        pinned to the release, shallow clone — run Axon" >&2
  say "  development  full history — change Axon" >&2
  say "" >&2
  printf '  Which? [usage/development]: ' >&2
  local answer=""
  read -r answer < /dev/tty || answer=""
  case "$answer" in
    d|dev|development) printf 'development' ;;
    *)                 printf 'usage' ;;
  esac
}

clone_axon() { # clone_axon <profile> <ref> <dir>
  local profile="$1" ref="$2" dir="$3"
  [ -e "$dir" ] && die "$dir already exists — remove it or set SJEL_DIR to somewhere else."
  if [ "$profile" = "development" ]; then
    say "Cloning $SJEL_REPO (development: full history) into $dir"
    git clone --quiet "$SJEL_REMOTE" "$dir"
    git -C "$dir" checkout --quiet "$ref"
  else
    say "Cloning $SJEL_REPO at $ref (usage: shallow) into $dir"
    git clone --quiet --depth 1 --branch "$ref" "$SJEL_REMOTE" "$dir"
  fi
}

main() {
  require_git

  local ref profile dir
  ref="$(resolve_ref)"
  [ -n "$ref" ] || die "could not resolve the newest release of $SJEL_REPO — set SJEL_REF to a tag."
  dir="${SJEL_DIR:-$PWD/Axon}"
  profile="$(ask_profile)"
  case "$profile" in
    usage|development) ;;
    *) die "unknown profile '$profile' — use usage or development." ;;
  esac

  clone_axon "$profile" "$ref" "$dir"

  say ""
  say "Cloned $(git -C "$dir" describe --tags 2>/dev/null || printf '%s' "$ref") into $dir"
  say "Handing off to tools/install.sh — every decision from here lives in the repository."
  say ""
  exec "$dir/tools/install.sh"
}

main "$@"

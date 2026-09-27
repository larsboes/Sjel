#!/bin/bash
# Both install profiles of tools/update.sh --check, exercised against clones this test
# makes for real. A fixture would not have caught what this covers: the usage profile's
# breakage came entirely from `git clone --depth 1 --branch <tag>` producing a refspec of
# `+refs/tags/<tag>:refs/tags/<tag>` and therefore NO origin/main ref — a property of real
# cloning, not of any directory you can construct by hand.
#
# The remote here is a scratch repository built from the tracked update path, not the Axon
# checkout: a test that pulled from the developer's own working tree would pass or fail
# depending on what they had uncommitted.
set -uo pipefail

_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

WORK="$(mktemp -d)"
trap 'chmod -R u+w "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

# --- build a real remote ---------------------------------------------------------------
REMOTE="$WORK/remote"
mkdir -p "$REMOTE/tools/lib"
for f in tools/update.sh tools/lib/paths.sh tools/lib/env-compat.sh tools/lib/toml.sh tools/lib/delta.sh tools/lib/version.sh; do
  [ -f "$_root/$f" ] || { echo "missing input: $f"; exit 1; }
  cp "$_root/$f" "$REMOTE/$f"
done
cp "$_root/axon.toml" "$REMOTE/axon.toml" 2>/dev/null || true
chmod +x "$REMOTE/tools/update.sh"
mkdir -p "$REMOTE/capabilities/demo"
echo "demo" > "$REMOTE/capabilities/demo/README.md"

git -C "$REMOTE" init --quiet -b main
git -C "$REMOTE" config user.email axon-test@example.invalid
git -C "$REMOTE" config user.name "Axon Test"
git -C "$REMOTE" add -A
git -C "$REMOTE" commit --quiet -m "v0.0.1"
git -C "$REMOTE" tag v0.0.1

# --- clone both profiles, exactly as tools/install.sh documents them ---------------------
git clone --quiet "file://$REMOTE" "$WORK/dev" 2>/dev/null
git clone --quiet --depth 1 --branch v0.0.1 "file://$REMOTE" "$WORK/usage" 2>/dev/null

# No overlay anywhere: the state a fresh clone is in before tools/install.sh runs.
export SJEL_OVERLAY_ROOT="$WORK/absent-overlay"

run_check() { # run_check <checkout> -> stdout+stderr, interleaved on purpose
  ( cd "$1" && ./tools/update.sh --check 2>&1 )
}

DEV_OUT="$(run_check "$WORK/dev")"
USAGE_OUT="$(run_check "$WORK/usage")"

# --- the usage profile is the one that was broken ---------------------------------------
case "$USAGE_OUT" in
  *fatal:*) fail "usage profile leaks a raw git 'fatal:' to the user" ;;
esac
case "$USAGE_OUT" in
  *grep:*) fail "usage profile leaks a raw grep error to the user" ;;
esac
# The malformed summary lines that followed each swallowed fatal.
case "$USAGE_OUT" in
  *"  ahead, "*) fail "usage profile prints an ahead/behind line with the numbers missing" ;;
esac
case "$USAGE_OUT" in
  *"latest:     ()"*) fail "usage profile prints a latest line with no version" ;;
esac

# The advice has to be advice that works. `git fetch --unshallow` alone does NOT restore
# origin/main on a tag-pinned clone, so naming it without the refspec widening would be
# the same wrong hint in a politer voice.
case "$USAGE_OUT" in
  *"git fetch --unshallow"*) ;;
  *) fail "usage profile does not name git fetch --unshallow" ;;
esac
case "$USAGE_OUT" in
  *"remote.origin.fetch"*) ;;
  *) fail "usage profile names --unshallow without the refspec widening that makes it work" ;;
esac

# A missing overlay is a state, not an error, on both profiles.
case "$DEV_OUT" in
  *grep:*) fail "development profile leaks a raw grep error for the absent overlay" ;;
esac
case "$DEV_OUT" in
  *"no overlay configured yet"*) ;;
  *) fail "development profile does not report the absent overlay as a state" ;;
esac

# --- version identity must survive the shallow clone ------------------------------------
# This worked before the fix and is the thing most likely to regress while fixing the rest.
for profile in dev usage; do
  got="$(git -C "$WORK/$profile" describe --tags 2>/dev/null)"
  [ "$got" = "v0.0.1" ] || fail "$profile: git describe said '$got', expected v0.0.1"
done
case "$USAGE_OUT" in
  *"installed: v0.0.1"*) ;;
  *) fail "usage profile does not report its installed release" ;;
esac
case "$DEV_OUT" in
  *"installed: v0.0.1"*) ;;
  *) fail "development profile does not report its installed release" ;;
esac

# --- the development profile still computes a real delta --------------------------------
case "$DEV_OUT" in
  *"0 ahead, 0 behind origin/main"*) ;;
  *) fail "development profile lost its ahead/behind summary" ;;
esac
case "$DEV_OUT" in
  *"remote.origin.fetch"*) fail "development profile shows the promotion hint it does not need" ;;
esac

# --- the pull path stops rather than failing four commands deep -------------------------
PULL_OUT="$( cd "$WORK/usage" && ./tools/update.sh --yes 2>&1 )"
PULL_RC=$?
[ "$PULL_RC" -eq 1 ] || fail "usage profile pull exited $PULL_RC, expected 1"
case "$PULL_OUT" in
  *"remote.origin.fetch"*) ;;
  *) fail "usage profile pull does not print the promotion instructions" ;;
esac
case "$PULL_OUT" in
  *fatal:*) fail "usage profile pull leaks a raw git 'fatal:'" ;;
esac

# --- and the promotion actually works ---------------------------------------------------
# The whole point of printing instructions is that following them fixes the checkout.
( cd "$WORK/usage" \
  && git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' \
  && git fetch --unshallow --quiet origin ) 2>/dev/null
if ! git -C "$WORK/usage" rev-parse --verify --quiet origin/main >/dev/null; then
  fail "following the printed promotion instructions did not produce origin/main"
fi
[ "$(git -C "$WORK/usage" rev-parse --is-shallow-repository)" = "false" ] \
  || fail "the promoted checkout is still shallow"
[ "$(git -C "$WORK/usage" describe --tags)" = "v0.0.1" ] \
  || fail "the promoted checkout lost its release identity"
PROMOTED_OUT="$(run_check "$WORK/usage")"
case "$PROMOTED_OUT" in
  *"0 ahead, 0 behind origin/main"*) ;;
  *) fail "the promoted checkout still cannot compute a delta" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "update.sh profiles: $fails check(s) failed"
  exit 1
fi
echo "update.sh profiles: all checks passed"

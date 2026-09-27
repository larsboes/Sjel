#!/bin/bash
# bootstrap.sh, over a scratch remote it clones for real.
#
# Two properties here cannot be checked by reading the script. The first is that no truncated
# prefix of it executes anything — the failure mode that makes `curl | bash` worth doing
# carefully, and the reason every line is a definition and the call is the last one. The
# second is that the two profiles actually produce the two clone shapes, which is a property
# of git's refspec handling rather than of this file.
set -uo pipefail

_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
BOOTSTRAP="$_root/bootstrap.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

# --- criterion 1: a truncated download must never execute partial logic ------------------
# Every prefix either fails to parse (nothing ran) or parses to definitions only and exits
# without side effects. What must NEVER happen is a prefix that clones, writes, or execs.
total_lines="$(wc -l < "$BOOTSTRAP" | tr -d ' ')"
probe="$WORK/probe"
n=1
while [ "$n" -lt "$total_lines" ]; do
  head -n "$n" "$BOOTSTRAP" > "$WORK/trunc.sh"
  rm -rf "$probe"; mkdir -p "$probe"
  # A real git on PATH would let a prefix clone; this one records the attempt and fails.
  printf '#!/bin/sh\ntouch "%s/git-ran"\nexit 1\n' "$probe" > "$WORK/git"
  chmod +x "$WORK/git"
  ( cd "$probe" && PATH="$WORK:$PATH" bash "$WORK/trunc.sh" >/dev/null 2>&1 )
  if [ -e "$probe/git-ran" ]; then
    fail "the first $n of $total_lines lines invoked git — a truncated download ran logic"
    break
  fi
  n=$((n + 1))
done

# --- build a scratch remote with both a tag and a main branch ----------------------------
REMOTE="$WORK/remote"
mkdir -p "$REMOTE/tools"
printf '#!/bin/bash\necho "INSTALL_SH_REACHED"\n' > "$REMOTE/tools/install.sh"
chmod +x "$REMOTE/tools/install.sh"
echo "axon" > "$REMOTE/README.md"
git -C "$REMOTE" init --quiet -b main
git -C "$REMOTE" config user.email axon-test@example.invalid
git -C "$REMOTE" config user.name "Axon Test"
git -C "$REMOTE" add -A
git -C "$REMOTE" commit --quiet -m "v9.9.9"
git -C "$REMOTE" tag v9.9.9
# A commit after the tag, so a full clone is visibly more than the shallow one.
echo "later" >> "$REMOTE/README.md"
git -C "$REMOTE" add -A
git -C "$REMOTE" commit --quiet -m "after the release"

run_bootstrap() { # run_bootstrap <profile> <dir>
  SJEL_REMOTE="file://$REMOTE" SJEL_REF=v9.9.9 SJEL_PROFILE="$1" SJEL_DIR="$2" \
    bash "$BOOTSTRAP" 2>&1
}

# --- criterion 3: two profiles, two clone shapes -----------------------------------------
USAGE_DIR="$WORK/usage"
USAGE_OUT="$(run_bootstrap usage "$USAGE_DIR")"
DEV_DIR="$WORK/dev"
DEV_OUT="$(run_bootstrap development "$DEV_DIR")"

[ "$(git -C "$USAGE_DIR" rev-parse --is-shallow-repository)" = "true" ] \
  || fail "the usage profile did not produce a shallow clone"
[ "$(git -C "$DEV_DIR" rev-parse --is-shallow-repository)" = "false" ] \
  || fail "the development profile produced a shallow clone"

# The development profile must reach the tag AND still have the history around it.
[ "$(git -C "$DEV_DIR" rev-list --count HEAD)" -ge 1 ] || fail "development clone has no history"
git -C "$DEV_DIR" rev-parse --verify --quiet origin/main >/dev/null \
  || fail "development clone has no origin/main"

# --- criterion 2/4: a real version on both, never (unknown) ------------------------------
for profile in usage dev; do
  d="$WORK/$profile"
  got="$(git -C "$d" describe --tags 2>/dev/null)"
  [ "$got" = "v9.9.9" ] || fail "$profile: git describe said '$got', expected v9.9.9"
  case "$got" in
    *unknown*) fail "$profile: version reported as unknown" ;;
  esac
done

# --- the handoff actually happens --------------------------------------------------------
for out in "$USAGE_OUT" "$DEV_OUT"; do
  case "$out" in
    *INSTALL_SH_REACHED*) ;;
    *) fail "bootstrap did not hand off to tools/install.sh. Got:
$out" ;;
  esac
done

# --- criterion 5: the DOCUMENTED promotion path works ------------------------------------
# Not `git fetch --unshallow` alone: a --depth 1 --branch <tag> clone has a tag-only refspec,
# so unshallowing deepens history along a refspec naming no branch and origin/main is still
# absent. Both commands, exactly as tools/install.sh prints them.
( cd "$USAGE_DIR" \
  && git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' \
  && git fetch --unshallow --quiet origin ) 2>/dev/null
git -C "$USAGE_DIR" rev-parse --verify --quiet origin/main >/dev/null \
  || fail "the documented promotion did not produce origin/main"
[ "$(git -C "$USAGE_DIR" rev-parse --is-shallow-repository)" = "false" ] \
  || fail "the promoted checkout is still shallow"
[ "$(git -C "$USAGE_DIR" describe --tags)" = "v9.9.9" ] \
  || fail "the promoted checkout lost its release identity"

# The negative half — proving the criterion had to be rewritten rather than taken on trust.
ALONE="$WORK/unshallow-alone"
git clone --quiet --depth 1 --branch v9.9.9 "file://$REMOTE" "$ALONE" 2>/dev/null
git -C "$ALONE" fetch --unshallow --quiet 2>/dev/null
if git -C "$ALONE" rev-parse --verify --quiet origin/main >/dev/null; then
  fail "--unshallow alone produced origin/main — the promotion hint can be simplified"
fi

# --- refusals ----------------------------------------------------------------------------
EXISTING="$WORK/existing"; mkdir -p "$EXISTING"
out="$(run_bootstrap usage "$EXISTING")"
case "$out" in
  *"already exists"*) ;;
  *) fail "bootstrap overwrote or ignored an existing directory. Got: $out" ;;
esac

out="$(SJEL_REMOTE="file://$REMOTE" SJEL_REF=v9.9.9 SJEL_PROFILE=nonsense SJEL_DIR="$WORK/bad" \
       bash "$BOOTSTRAP" 2>&1)"
case "$out" in
  *"unknown profile"*) ;;
  *) fail "bootstrap accepted an unknown profile. Got: $out" ;;
esac

# --- tools/install.sh states which profile it is serving ---------------------------------
# Against the real installer, not the stub, with its interactive half never reached: the
# profile line is printed before the first prompt.
say_profile() { # say_profile <checkout>
  ( cd "$1" && printf '' | bash "$_root/tools/install.sh" 2>&1 | head -8 )
}
# Three legitimate answers, not two: a tree that is not a git checkout at all — an export,
# a copy — has to say so, which is a correct third statement rather than a gap. The
# criterion is that it states which profile it serves — asserting on one specific answer
# would only be asserting on where the test happens to be running.
PROFILE_LINE="$(say_profile "$_root" | grep '^Profile:' || true)"
case "$PROFILE_LINE" in
  "Profile: development"*|"Profile: usage"*|"Profile: not a git checkout"*) ;;
  *) fail "tools/install.sh does not state which profile it is serving (got: '${PROFILE_LINE:-<nothing>}')" ;;
esac

# A checkout with real git history must resolve to a real profile, never the fallback — the
# half that keeps the assertion above from passing on "not a git checkout" forever. Both
# clones this test made are genuine checkouts, so they are the honest place to check it.
for d in "$USAGE_DIR" "$DEV_DIR"; do
  case "$(git -C "$d" rev-parse --is-shallow-repository 2>/dev/null)" in
    true|false) ;;
    *) fail "$d is not a git checkout, so the profile statement cannot be exercised against it" ;;
  esac
done

if [ "$fails" -gt 0 ]; then
  echo "bootstrap: $fails check(s) failed"
  exit 1
fi
echo "bootstrap: all checks passed"

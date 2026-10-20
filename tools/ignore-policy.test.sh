#!/bin/bash
# tools/ignore-policy — the renderer, the three failure modes, and what git actually does
# with the result (Sjel#194).
#
# The behaviour cases run `git check-ignore` against a real temp repository rather than
# asserting on the text of the generated file. That distinction is the point of the issue: a
# .gitignore that reads correctly and resolves incorrectly is exactly the bug, and only git
# can tell the two apart. Every path here is synthetic.
set -uo pipefail

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
POLICY="$_here/ignore-policy"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
check() { # check <label> <expected> <actual>
  if [ "$2" = "$3" ]; then printf '  ✓ %s\n' "$1"
  else printf '  ✗ %s\n      expected: %s\n      got:      %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
}

# overlay <name> <allowlist lines...> — a temp git repo with an allowlist, rendered.
overlay() {
  local name="$1"; shift
  local dir="$SCRATCH/axon-$name"
  mkdir -p "$dir"
  git -C "$dir" init -q 2>/dev/null
  printf '%s\n' "$@" > "$dir/.ignore-allowlist"
  "$POLICY" render "$dir" >/dev/null || return 1
  printf '%s' "$dir"
}

# ignored <repo> <path> — "yes" if git would ignore it. The file is created first, because
# check-ignore answers about a path either way and creating it is what a real mistake looks
# like. --no-index so an already-tracked file cannot mask the answer.
ignored() {
  local repo="$1" path="$2"
  mkdir -p "$repo/$(dirname "$path")" 2>/dev/null
  : > "$repo/$path"
  if git -C "$repo" check-ignore --no-index -q "$path" 2>/dev/null; then echo yes; else echo no; fi
}

echo "render"

DIR="$(overlay alpha '!/config/' '/config/*' '!/config/.gitkeep')"
check "renders without an existing .gitignore" "yes" "$([ -f "$DIR/.gitignore" ] && echo yes || echo no)"
check "the overlay name reaches the header" "1" "$(grep -c 'axon-alpha' "$DIR/.gitignore")"

# Two different allowlists must produce two different files from the same Sjel inputs — that
# is what makes the allowlist a separate input rather than decoration.
DIR2="$(overlay beta '!/network/' '/network/*')"
check "a different allowlist renders a different policy" "no" \
  "$(cmp -s "$DIR/.gitignore" "$DIR2/.gitignore" && echo yes || echo no)"
# Anchored on the marker line, not on the phrase: the preamble explains the floor in prose and
# a loose match lands there instead, comparing two headers that legitimately differ by name.
floor_of() { sed -n '/^# ---- Sjel immutable hard blocks/,$p' "$1"; }
check "both still carry the same immutable floor" "yes" \
  "$( [ "$(floor_of "$DIR/.gitignore")" = "$(floor_of "$DIR2/.gitignore")" ] && echo yes || echo no)"

echo
echo "what git does with it"

DIR="$(overlay alpha '!/config/' '/config/*' '!/config/.gitkeep' '!/resources/' '!/resources/backups/**')"
check "an allowed file is tracked"          "no"  "$(ignored "$DIR" "config/.gitkeep")"
check "an unlisted top-level file is ignored" "yes" "$(ignored "$DIR" "notes.md")"
check "an unlisted file inside an allowed dir is ignored" "yes" "$(ignored "$DIR" "config/scratch.txt")"

# The floor, exercised through git rather than through grep. resources/backups/** is allowed
# above, so these three prove the blocks below actually outrank a live allow rule.
check "a .env inside an allowed tree is blocked"      "yes" "$(ignored "$DIR" "resources/backups/app.env")"
check "a secret-shaped name is blocked"               "yes" "$(ignored "$DIR" "resources/backups/my-secret-notes.md")"
check "a credential-shaped name is blocked"           "yes" "$(ignored "$DIR" "resources/backups/credential-list.txt")"
check "a .DS_Store anywhere is blocked"               "yes" "$(ignored "$DIR" "resources/backups/.DS_Store")"

echo
echo "check — the three failure modes"

DIR="$(overlay alpha '!/config/' '/config/*' '!/config/.gitkeep')"
"$POLICY" check "$DIR" >/dev/null 2>&1
check "a freshly rendered policy is clean" "0" "$?"

# 1. drift — someone edited the generated file instead of the allowlist.
DRIFT="$(overlay drift '!/config/')"
printf '!/snuck-in.txt\n' >> "$DRIFT/.gitignore"
"$POLICY" check "$DRIFT" >/dev/null 2>&1
check "an edit to the generated file fails the check" "1" "$?"

# 2. a weakened floor — a hard block removed by hand.
WEAK="$(overlay weak '!/config/')"
grep -v '^\*\.env$' "$WEAK/.gitignore" > "$WEAK/.g" && mv "$WEAK/.g" "$WEAK/.gitignore"
out="$("$POLICY" check "$WEAK" 2>&1)"; rc=$?
check "a removed hard block fails the check" "1" "$rc"
case "$out" in *"missing hard block: *.env"*) echo "  ✓ and names the block that went missing" ;;
  *) echo "  ✗ the failure does not name the missing block"; fails=$((fails + 1)) ;; esac

# 3. weakened ordering — an allow rule appended below the floor. This one is the reason the
#    check exists: the file still contains every hard block, so a presence test passes it.
#
#    The allowlist has to open the parent chain for the demonstration to be honest. Git cannot
#    re-include a file whose parent directory is excluded, so appending `!/resources/x.env` to
#    an overlay that never un-ignored `/resources/` proves nothing — the file stays ignored for
#    a reason that has nothing to do with the floor, and the case would pass while testing
#    something else entirely.
ORDER="$(overlay order '!/config/' '!/resources/' '!/resources/backups/**')"
check "the floor blocks it while the floor is last" "yes" "$(ignored "$ORDER" "resources/backups/leaked.env")"
printf '!/resources/backups/leaked.env\n' >> "$ORDER/.gitignore"
out="$("$POLICY" check "$ORDER" 2>&1)"; rc=$?
check "a pattern below the hard blocks fails the check" "1" "$rc"
case "$out" in *"outrank the floor"*) echo "  ✓ and says the floor was outranked, not merely that content changed" ;;
  *) echo "  ✗ ordering violation not reported as such"; fails=$((fails + 1)) ;; esac
# The bite, proven through git: one appended line takes a real .env back out of the floor.
check "and git agrees the appended line un-protected a .env" "no" "$(ignored "$ORDER" "resources/backups/leaked.env")"

echo
if [ "$fails" -eq 0 ]; then echo "overlay ignore policy: all checks passed"
else echo "overlay ignore policy: $fails check(s) failed"; fi
exit "$fails"

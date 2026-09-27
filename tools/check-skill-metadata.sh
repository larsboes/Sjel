#!/bin/bash
# check-skill-metadata.sh — the skill-metadata gate (CI: repo gates).
#
# Every SKILL.md this repository authors is checked against the one validator that
# encodes the rules: name, description, directory match, reserved words, XML tags,
# first/second person, the trigger clauses a model routes on, and the Level-2 body
# budget. The validator is `skill-creator`'s own (Packs/writing), which is why the
# gate is a wrapper and not a second implementation — a second one would be the very
# duplication this repository keeps deleting.
#
# The class of failure this closes, dated: the validator existed from 2026-07-28 in
# two copies and was called by NOTHING. Six skills had drifted out of compliance and
# nobody could have known. One was crystallize, which used second person in its
# description (a hard error, so this gate would have failed the build) *and* had lost
# both trigger clauses, so the skill a model is supposed to reach for when someone says
# "turn these notes into a spec" advertised no phrase anyone would say — that half is a
# warning, see below. A rule nothing checks is a rule that has already been broken;
# nobody notices, because the only symptom is a skill that never fires.
#
# Scope: Axon-authored skills only, `Packs/<pack>/skills/<name>/SKILL.md`.
# Deliberately NOT checked, each for a reason rather than by oversight:
#   Packs/*/pi-packages/**  vendored third-party pi packages. Their files are
#                           byte-identical to upstream on purpose (Packs/harness/
#                           LICENSE), so rewriting a description here would break a
#                           verifiable invariant to satisfy a local convention.
#                           The two accordion skills fail the XML-tag rule for a
#                           real reason: their descriptions reproduce the fold
#                           marker's own `<code>` placeholder.
#   */evals/**              skill-creator's fixtures. `evals/files/bad-skill/` is a
#                           deliberately invalid skill, so a failure there is the
#                           test passing.
# Exclusions are printed, because a silently skipped path reads as a clean run.
#
# Errors fail. DISCOVERY WARNINGs are printed and do not, because they are judgement
# calls about trigger wording and a gate that fails on judgement gets disabled. Run
# with --strict to fail on them too.
#
# Exit 0 clean · 1 a skill is out of compliance · 2 setup error (no python3, no
# validator, or nothing to check). bash 3.2-safe, no git and no network: same contract as
# the sibling gates.
#
# SJEL_PACKS_ROOT overrides the tree that is walked, for the planted-tree regression
# tests in tools/check-skill-metadata.test.sh. The validator is still resolved from the
# real SJEL_ROOT, because that is the real dependency and faking it would only test the
# fake.
set -u

_here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"   # SJEL_ROOT

PACKS_ROOT="${SJEL_PACKS_ROOT:-$SJEL_ROOT/Packs}"

VALIDATOR="$SJEL_ROOT/Packs/writing/skills/skill-creator/scripts/validate_metadata.py"
strict=0
case "${1:-}" in
  "")        ;;
  --strict)  strict=1 ;;
  -h|--help) sed -n '2,35p' "$0"; exit 0 ;;
  *)         echo "check-skill-metadata: unknown argument '$1' (try --help)" >&2; exit 2 ;;
esac

# A missing interpreter or validator is a setup error, never a finding. It must not
# read as clean (that hides the gate) and must not read as a violation (that blames a
# skill for a broken checkout).
if ! command -v python3 >/dev/null 2>&1; then
  echo "check-skill-metadata: python3 is not on PATH — setup error, not a finding." >&2
  exit 2
fi
if [ ! -r "$VALIDATOR" ]; then
  echo "check-skill-metadata: validator missing at $VALIDATOR" >&2
  echo "  It ships inside the writing Pack; a checkout without it cannot run this gate." >&2
  exit 2
fi

checked=0
failed=0
warned=0
skipped=0

while IFS= read -r f; do
  [ -n "$f" ] || continue
  rel="${f#"$SJEL_ROOT"/}"

  case "$rel" in
    */pi-packages/*|*/evals/*|*/node_modules/*)
      skipped=$((skipped + 1))
      echo "skip (not ours to edit): $rel"
      continue
      ;;
  esac

  checked=$((checked + 1))
  out="$(python3 "$VALIDATOR" --file "$f" --dir "$(dirname "$f")" 2>&1)"
  rc=$?

  if [ $rc -ne 0 ]; then
    failed=$((failed + 1))
    echo "FAIL: $rel" >&2
    printf '%s\n' "$out" | grep -v '^SUCCESS' | grep -v '^Body: ' | sed 's/^/    /' >&2
  elif printf '%s\n' "$out" | grep -q 'WARNING'; then
    warned=$((warned + 1))
    echo "warn: $rel"
    printf '%s\n' "$out" | grep 'WARNING' | sed 's/^/    /'
  fi
done <<EOF
$(find "$PACKS_ROOT" -path '*/skills/*/SKILL.md' -not -path '*/node_modules/*' | sort)
EOF

echo
echo "check-skill-metadata: $checked checked, $failed failed, $warned warned, $skipped skipped."

if [ "$failed" -gt 0 ]; then
  exit 1
fi
if [ "$strict" -eq 1 ] && [ "$warned" -gt 0 ]; then
  echo "  --strict: $warned skill(s) carry a discovery warning." >&2
  exit 1
fi
if [ "$checked" -eq 0 ]; then
  # Zero skills checked means the find or the exclusions ate the whole tree — the
  # silent-green failure this gate exists to prevent.
  echo "check-skill-metadata: no SKILL.md found under $PACKS_ROOT — the walk is broken." >&2
  exit 2
fi
exit 0

#!/bin/bash
# Planted-tree regression tests for check-skill-metadata.sh.
#
# Each case is a throwaway Packs tree the gate is pointed at with SJEL_PACKS_ROOT, so
# every red path is PROVEN rather than assumed: a gate that passes because it checked
# nothing looks exactly like a gate that passes because everything is fine. That
# ambiguity is the whole reason this gate exists (the validator went uncalled for six
# weeks), so the test has to be able to tell the two apart.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-skill-metadata.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

tree() { # tree <case-name> -> echoes the fresh packs root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root"
  printf '%s' "$root"
}

skill() { # skill <packs-root> <pack> <name> <description>
  local dir="$1/$2/skills/$3"
  mkdir -p "$dir"
  printf -- '---\nname: %s\ndescription: %s\n---\n\nBody.\n' "$3" "$4" > "$dir/SKILL.md"
}

run() { # run <packs-root> [extra args...] -> output in $out, exit status in $status
  local root="$1"; shift
  out=$(SJEL_PACKS_ROOT="$root" "$CHECK" "$@" 2>&1)
  status=$?
}

expect() { # expect <description> <want-status> <packs-root> [extra args...]
  local desc="$1" want="$2" root="$3"; shift 3
  run "$root" "$@"
  if [ "$status" -ne "$want" ]; then
    echo "FAIL: $desc — wanted exit $want, got $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

GOOD="Declines the thing cleanly. Use when the user asks for the thing, or invokes /thing. Do not use for the other thing (use other-skill)."

# ── the green path, so a gate that fails everything cannot pass this file ──────────
root=$(tree green)
skill "$root" "demo" "demo-skill" "$GOOD"
expect "a compliant skill passes" 0 "$root"

# ── the class the gate exists for: a description the validator rejects ──────────────
# Second person is a hard STYLE ERROR, so this must fail rather than warn.
root=$(tree second-person)
skill "$root" "demo" "demo-skill" "Extracts the thing when you ask for it. Use when asked."
expect "a second-person description fails" 1 "$root"

# A description with no trigger clause is the other half of the crystallize failure:
# the skill a model should reach for advertises no phrase anyone would say. On its own
# this is a DISCOVERY WARNING, so the gate reports it and passes; --strict escalates it.
# Crystallize failed CI because it ALSO used second person — a hard error. Both halves
# are tested here, separately, so neither is mistaken for the other.
root=$(tree no-trigger)
skill "$root" "demo" "demo-skill" "Extracts the thing from a pile of notes and makes it precise."
expect "a description with no trigger clause passes with a warning" 0 "$root"
expect "--strict fails a description with no trigger clause" 1 "$root" --strict

# ── warnings report without failing, and --strict escalates them ───────────────────
# Missing negative trigger is a DISCOVERY WARNING, not an error: a judgement call about
# wording. A gate that fails on judgement gets switched off, so it defaults to warning.
root=$(tree warning-only)
skill "$root" "demo" "demo-skill" "Extracts the thing from a pile of notes. Use when asked to make notes precise."
expect "a discovery warning does not fail the gate" 0 "$root"
expect "--strict escalates a discovery warning" 1 "$root" --strict

# ── the exclusions are exclusions, and they are real ───────────────────────────────
# A vendored pi package is byte-identical to upstream on purpose; its description
# reproducing the fold marker's own <code> placeholder must not fail a local gate.
root=$(tree vendored)
skill "$root" "harness" "pi-packages" "$GOOD"            # decoy: wrong shape, ignored anyway
mkdir -p "$root/harness/pi-packages/accordion/extension/skills/accordion-x"
printf -- '---\nname: accordion-x\ndescription: "Read this if you see {#<code> FOLDED} markers."\n---\n\nBody.\n' \
  > "$root/harness/pi-packages/accordion/extension/skills/accordion-x/SKILL.md"
skill "$root" "demo" "demo-skill" "$GOOD"
expect "a vendored pi-package skill is skipped, not failed" 0 "$root"

root=$(tree evals-fixture)
mkdir -p "$root/writing/skills/skill-creator/evals/files/bad-skill"
printf -- '---\nname: bad\ndescription: you broke it\n---\n\nBody.\n' \
  > "$root/writing/skills/skill-creator/evals/files/bad-skill/SKILL.md"
skill "$root" "demo" "demo-skill" "$GOOD"
expect "a deliberately-bad eval fixture is skipped" 0 "$root"

# ── silent green ───────────────────────────────────────────────────────────────────
# Nothing checked must never read as everything fine. 34 skills once went unchecked
# because a rule had no runner; an empty walk is the same failure in miniature.
root=$(tree empty)
expect "checking nothing is a setup error, not a pass" 2 "$root"

# ── setup errors do not blame a skill ──────────────────────────────────────────────
# A missing interpreter must be a named setup error. Only `dirname` is needed before
# the interpreter check, so a PATH holding just that proves the branch in isolation.
root=$(tree no-python)
skill "$root" "demo" "demo-skill" "$GOOD"
tmpbin="$SCRATCH/bin"
rm -rf "$tmpbin"; mkdir -p "$tmpbin"
ln -s "$(command -v dirname)" "$tmpbin/dirname"
out=$(PATH="$tmpbin" SJEL_PACKS_ROOT="$root" "$CHECK" 2>&1)
status=$?
if [ "$status" -ne 2 ] || ! printf '%s' "$out" | grep -qF "setup error"; then
  echo "FAIL: a missing python3 should be exit 2 and name a setup error, got exit $status:"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
fi

# ── the real tree, end to end ──────────────────────────────────────────────────────
# The planted cases above prove the gate can fail; this proves it still passes over the
# repository it actually guards, so a fix that only satisfies the fixtures is caught.
out=$("$CHECK" 2>&1)
status=$?
if [ "$status" -ne 0 ]; then
  echo "FAIL: the real Packs tree should pass the gate, got exit $status:"
  printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
fi

if [ "$fails" -gt 0 ]; then
  echo "check-skill-metadata.test.sh: $fails case(s) failed."
  exit 1
fi
echo "check-skill-metadata.test.sh: all cases passed."

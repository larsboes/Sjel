#!/bin/bash
# tools/protection-zones — verdict coverage, fail-closed behaviour, and the derived Claude
# deny rules (Axon#147).
#
# Every path in here is synthetic and lives in a temp dir. The real policy names material
# that must not reach a cloud-backed model; a fixture that copied it would be the disclosure
# the tool exists to prevent, and would leak through CI output on the first red run.
set -uo pipefail

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
ZONES="$_here/protection-zones"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
check() {
  if [ "$2" = "$3" ]; then printf '  ✓ %s\n' "$1"
  else printf '  ✗ %s\n      expected: %s\n      got:      %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
}

policy() { # policy <name> <body...> -> path
  local p="$SCRATCH/$1.toml"; shift
  printf '%s\n' "$@" > "$p"
  printf '%s' "$p"
}

echo "verdicts"

out="$("$ZONES" verdicts 2>&1)"
# The roster is read from agent-integrations.sh rather than restated, so this asserts
# coverage of whatever that file declares — a newly adopted harness with no verdict is
# supposed to fail here rather than be silently absent from the report.
roster="$(sed -n 's/^HARNESSES="\(.*\)"$/\1/p' "$_here/agent-integrations.sh" | head -1)"
missing=""
for h in $roster; do
  printf '%s' "$out" | grep -qE "^ +[✓◐✗] +$h " || missing="$missing $h"
done
check "every harness in the roster gets a verdict" "" "$missing"
check "no harness is silently absent" "4" "$(printf '%s' "$out" | grep -cE '^ +[✓◐✗] ')"

# The load-bearing claim of the whole issue: an unenforceable class must SAY so.
check "codex is reported unsupported, not omitted"    "1" "$(printf '%s' "$out" | grep -c 'codex  *unsupported')"
check "opencode is reported unsupported, not omitted" "1" "$(printf '%s' "$out" | grep -c 'opencode  *unsupported')"
check "pi is separately-profiled, not claimed as host protection" "1" "$(printf '%s' "$out" | grep -c 'pi  *separately-profiled')"
case "$out" in
  *"CONTAINER paths"*) echo "  ✓ and says why: it protects container paths, not host paths" ;;
  *) echo "  ✗ pi's verdict does not state its actual boundary"; fails=$((fails + 1)) ;;
esac

echo
echo "check — fail closed"

SJEL_PROTECTION_ZONES="$SCRATCH/does-not-exist.toml" "$ZONES" check >/dev/null 2>&1
check "a missing policy fails, never defaults to allow" "1" "$?"
out="$(SJEL_PROTECTION_ZONES="$SCRATCH/does-not-exist.toml" "$ZONES" check 2>&1)"
case "$out" in
  *"absence is not permission"*) echo "  ✓ and says absence is not permission" ;;
  *) echo "  ✗ the refusal is not actionable"; fails=$((fails + 1)) ;;
esac

P="$(policy notapolicy 'title = "something else"')"
SJEL_PROTECTION_ZONES="$P" "$ZONES" check >/dev/null 2>&1
check "a file without [zones] is rejected" "1" "$?"

# An empty policy is a VALID claim — "nothing on this machine is protected" — and must pass.
# A tool that refuses it teaches people to write a fake entry to get past it.
P="$(policy empty '[zones]' 'local_only = []' 'no_model = []')"
SJEL_PROTECTION_ZONES="$P" "$ZONES" check >/dev/null 2>&1
check "an empty policy is valid, not an error" "0" "$?"

P="$(policy good '[zones]' 'local_only = ["~/synthetic-work"]' 'no_model = ["/tmp/synthetic-keys"]')"
SJEL_PROTECTION_ZONES="$P" "$ZONES" check >/dev/null 2>&1
check "a well-formed policy passes" "0" "$?"

# One path, one class. Two classes for one path is a contradiction each harness would
# resolve differently and silently.
P="$(policy overlap '[zones]' 'local_only = ["/tmp/synthetic-both"]' 'no_model = ["/tmp/synthetic-both"]')"
out="$(SJEL_PROTECTION_ZONES="$P" "$ZONES" check 2>&1)"; rc=$?
check "a path in two classes is rejected" "1" "$rc"
case "$out" in
  *"names withheld"*) echo "  ✓ and withholds the paths while reporting the count" ;;
  *) echo "  ✗ the overlap report may be leaking paths"; fails=$((fails + 1)) ;;
esac
check "and the offending path is not printed" "0" "$(printf '%s' "$out" | grep -c 'synthetic-both')"

P="$(policy relative '[zones]' 'local_only = ["work/stuff"]' 'no_model = []')"
SJEL_PROTECTION_ZONES="$P" "$ZONES" check >/dev/null 2>&1
check "a relative path is rejected" "1" "$?"

echo
echo "claude-fragment — derived, not hand-written"

P="$(policy derive '[zones]' 'local_only = ["/tmp/synthetic-work"]' 'no_model = ["/tmp/synthetic-keys"]')"
out="$(SJEL_PROTECTION_ZONES="$P" "$ZONES" claude-fragment 2>&1)"; rc=$?
check "renders from a valid policy" "0" "$rc"
check "both classes become deny rules" "2" "$(printf '%s' "$out" | grep -c 'Read(')"
check "local_only is denied too (Claude cannot see a local model)" "1" "$(printf '%s' "$out" | grep -c 'synthetic-work')"
# Valid JSON is not decoration here: this text is destined for a security floor, and a
# fragment that half-parses is worse than one that fails.
if command -v bun >/dev/null 2>&1; then
  printf '%s' "$out" | bun -e 'const t=await Bun.stdin.text(); JSON.parse(t)' >/dev/null 2>&1
  check "the fragment is parseable JSON" "0" "$?"
else
  echo "  ⊘ bun not on PATH — JSON parse check skipped"
fi

P="$(policy badderive '[zones]' 'local_only = ["relative/path"]')"
SJEL_PROTECTION_ZONES="$P" "$ZONES" claude-fragment >/dev/null 2>&1
check "an invalid policy renders no fragment at all" "2" "$?"

echo
if [ "$fails" -eq 0 ]; then echo "protection zones: all checks passed"
else echo "protection zones: $fails check(s) failed"; fi
exit "$fails"

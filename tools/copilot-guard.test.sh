#!/bin/bash
# The Copilot guard, as a test.
#
# Until 2026-10-24 the blocks on `rm`, `git commit` and `git push` lived in tools/copilot as three
# --deny-tool flags. A launcher flag is a claim about the sessions somebody started through the
# launcher; typing `copilot` reached the same agent with none of the blocks, and nothing in the
# repository would have said so. Copilot persists no tool-deny setting, but it does load
# ~/.copilot/hooks/*.json for every session, and a command preToolUse hook is fail-closed — a
# crash or non-zero exit denies the tool call.
#
# So the assertions that matter are behavioural, not structural: a payload of each blocked shape
# must produce a deny decision, a payload that merely mentions the word must not, and the two
# files must be reported drifted when either is edited or removed. A test that grepped the script
# for "git push" would keep passing while the awk that decides stopped matching.
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
ROOT="$(cd "$_dir/.." && pwd)"
GUARD="$ROOT/tools/templates/copilot/hooks/sjel-guard.sh"
SCRATCH="$(mktemp -d /tmp/sjel-copilot-guard.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

source "$ROOT/tools/lib/test-support.sh"
isolate_sjel_env
unset COPILOT_HOME

[ -x "$GUARD" ] || { echo "FAIL: $GUARD is missing or not executable — the hook Copilot cannot run" >&2; exit 1; }

# run_guard <payload> — the decision object, or nothing when the guard has no opinion.
run_guard() { printf '%s' "$1" | "$GUARD"; }

# --- 1. blocked shapes -----------------------------------------------------
# Each payload is the shape Copilot sends: toolName plus toolArgs.command. The wrappers case
# matters because `sudo rm` and `FOO=1 /bin/rm` are the same command to a shell and looked
# different to the launcher flag's `shell(rm:*)` pattern.
while IFS='|' read -r label payload; do
  [ -n "$label" ] || continue
  out="$(run_guard "$payload")"
  case "$out" in
    *'"permissionDecision":"deny"'*) ;;
    *) fail "not denied: $label" ;;
  esac
done <<'CASES'
plain rm|{"toolName":"bash","toolArgs":{"command":"rm -rf /tmp/x"}}
rm after cd|{"toolName":"bash","toolArgs":{"command":"cd /tmp && rm -rf x"}}
rm after semicolon|{"toolName":"bash","toolArgs":{"command":"echo hi; rm -rf x"}}
rm in a subshell|{"toolName":"bash","toolArgs":{"command":"(cd x; rm -rf y)"}}
rm under sudo|{"toolName":"bash","toolArgs":{"command":"sudo rm -rf /tmp/x"}}
rm under sudo -u|{"toolName":"bash","toolArgs":{"command":"sudo -u root rm -rf /tmp/x"}}
rm by absolute path|{"toolName":"bash","toolArgs":{"command":"/bin/rm -rf /tmp/x"}}
rm with an env assignment|{"toolName":"bash","toolArgs":{"command":"FOO=1 /usr/bin/rm -rf x"}}
git commit|{"toolName":"bash","toolArgs":{"command":"git commit -m x"}}
git commit after a quoted echo|{"toolName":"bash","toolArgs":{"command":"echo \"hi\" && git commit -am x"}}
git push|{"toolName":"bash","toolArgs":{"command":"git push origin main"}}
git push after cd|{"toolName":"bash","toolArgs":{"command":"cd /tmp && git push origin main"}}
git push with -C|{"toolName":"bash","toolArgs":{"command":"git -C /repo push"}}
git push, Claude-format payload|{"tool_name":"Bash","tool_input":{"command":"git push"}}
CASES

# --- 2. shapes that must stay allowed -------------------------------------
# A guard that denies too much gets removed, and a removed guard blocks nothing. `npm run
# rm-cache` and `ls | grep rm` are the two that a substring match would break.
while IFS='|' read -r label payload; do
  [ -n "$label" ] || continue
  out="$(run_guard "$payload")"
  if [ -n "$out" ]; then fail "wrongly denied: $label -> $out"; fi
done <<'CASES'
git status|{"toolName":"bash","toolArgs":{"command":"git status"}}
git log|{"toolName":"bash","toolArgs":{"command":"git log --oneline"}}
git commit is only a prefix|{"toolName":"bash","toolArgs":{"command":"git committed-history --help"}}
rm inside a longer word|{"toolName":"bash","toolArgs":{"command":"npm run rm-cache"}}
rm as a grep argument|{"toolName":"bash","toolArgs":{"command":"ls | grep rm"}}
plain ls with a timeout sibling|{"toolName":"bash","toolArgs":{"command":"ls -la","timeout":30000}}
a non-shell tool naming rm|{"toolName":"view","toolArgs":{"path":"rm"}}
an edit tool|{"toolName":"edit","toolArgs":{"path":"a.md","oldString":"rm -rf /"}}
empty payload||{}
CASES

# --- 3. apply and check round-trip ----------------------------------------
# The guard is only a floor if `check` sees it. This half proves the two files are written, that
# the registration points at THIS Copilot home (COPILOT_HOME honoured), and that both an edit and
# a deletion are reported — the two ways a session running as the owner could lift the floor.
COPILOT_HOME="$SCRATCH/copilot"
export COPILOT_HOME
mkdir -p "$COPILOT_HOME"

if ! sjel_cli_prebuilt; then
  fail "could not build sjel-cli"
else
  cli() { SJEL_ROOT="$ROOT" "$SJEL_CLI_BIN" copilot "$@"; }
  SJEL_ROOT="$ROOT" "$SJEL_CLI_BIN" copilot apply >/dev/null 2>&1 || fail "copilot apply failed"
  [ -x "$COPILOT_HOME/hooks/sjel-guard.sh" ] || fail "apply did not write an executable hooks/sjel-guard.sh"
  grep -q "$COPILOT_HOME/hooks/sjel-guard.sh" "$COPILOT_HOME/hooks/sjel-guard.json" \
    || fail "hooks/sjel-guard.json does not name this COPILOT_HOME's script"

  cli check >/dev/null 2>&1 || fail "check reported drift straight after apply"

  printf '\n# edited by a session\n' >> "$COPILOT_HOME/hooks/sjel-guard.sh"
  if cli check >"$SCRATCH/check-edit.txt" 2>&1; then fail "an edited guard was not reported as drift"; fi
  grep -q 'sjel-guard.sh' "$SCRATCH/check-edit.txt" || fail "edited guard drift did not name the script"

  cli apply >/dev/null 2>&1 || fail "apply did not restore the edited guard"
  cli check >/dev/null 2>&1 || fail "check still reports drift after restoring the guard"

  rm -f "$COPILOT_HOME/hooks/sjel-guard.json"
  if cli check >"$SCRATCH/check-missing.txt" 2>&1; then fail "a removed guard registration was not reported"; fi
  grep -q 'sjel-guard.json' "$SCRATCH/check-missing.txt" || fail "missing registration drift did not name the file"
fi

if [ "$fails" -ne 0 ]; then
  echo "copilot-guard: $fails failure(s)" >&2
  exit 1
fi
echo "copilot-guard: every blocked shape denied, every allowed shape allowed, drift seen both ways"

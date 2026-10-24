#!/bin/bash
# Regression tests for tools/skills. The policy IS the tool, so each case proves an outcome
# rather than a message: a refusal must not reach npx at all, and a project install must land
# at the repository root with the two agents pi and Claude Code actually read.
#
# Nothing here touches the developer's real skills: npx is replaced by a fake on PATH that
# records its argv, and the global list audit reads is a fixture.
set -uo pipefail

SKILLS="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/skills"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

LOG="$SCRATCH/npx.log"
GLOBAL_JSON="$SCRATCH/global.json"
mkdir -p "$SCRATCH/bin"

# In a stage, the fake also writes the skill the gate would read.
cat > "$SCRATCH/bin/npx" <<'FAKE'
#!/bin/bash
printf '%s  [cwd %s]\n' "$*" "$PWD" >> "$SKILLS_TEST_LOG"
case " $* " in
  *" list -g --json "*) cat "$SKILLS_TEST_GLOBAL_JSON" ;;
  *" add "*)
    case "$PWD" in
      */intake/staged/*) mkdir -p .claude/skills/staged-skill && echo '# skill' > .claude/skills/staged-skill/SKILL.md ;;
    esac ;;
  *) : ;;
esac
FAKE
# The gate stand-in records its arguments and answers SKILLS_TEST_GATE_RC.
cat > "$SCRATCH/bin/intake" <<'FAKE'
#!/bin/bash
printf 'intake %s\n' "$*" >> "$SKILLS_TEST_LOG"
exit "${SKILLS_TEST_GATE_RC:-0}"
FAKE
chmod +x "$SCRATCH/bin/npx" "$SCRATCH/bin/intake"
export PATH="$SCRATCH/bin:$PATH"
export SKILLS_TEST_LOG="$LOG"
export SKILLS_TEST_GLOBAL_JSON="$GLOBAL_JSON"
export SJEL_INTAKE="$SCRATCH/bin/intake"
export XDG_STATE_HOME="$SCRATCH/state"

fails=0

# A project tree with a git root, so "run from a subdirectory" is a real case.
project() { # project <name> -> echoes the root, resolved, so /var and /private/var compare equal
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/sub/dir"
  git -C "$root" init -q
  (cd "$root" && pwd -P)
}

run() { # run <cwd> <args...>
  local cwd="$1"
  shift
  out=$(cd "$cwd" && "$SKILLS" "$@" 2>&1)
  status=$?
}

expect() { # expect <description> <substring> <present:1|absent:0>
  if [ "$3" = "1" ]; then
    if ! printf '%s' "$out" | grep -qF -- "$2"; then
      echo "FAIL: $1 — expected output to contain '$2'"
      printf '%s\n' "$out" | sed 's/^/    /'
      fails=$((fails + 1))
    fi
  elif printf '%s' "$out" | grep -qF -- "$2"; then
    echo "FAIL: $1 — expected output NOT to contain '$2'"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# The argv npx received for the real install, which is where install scope is visible. The
# staging call and the gate are filtered out; expect_stage reads those.
installs() { grep -v -e 'intake/staged' -e '^intake ' "$LOG"; }
expect_log() { # expect_log <description> <substring> <present:1|absent:0>
  if [ "$3" = "1" ]; then
    if ! installs | grep -qF -- "$2"; then
      echo "FAIL: $1 — expected npx to receive '$2'"
      sed 's/^/    /' "$LOG"
      fails=$((fails + 1))
    fi
  elif installs | grep -qF -- "$2"; then
    echo "FAIL: $1 — expected npx NOT to receive '$2'"
    sed 's/^/    /' "$LOG"
    fails=$((fails + 1))
  fi
}

# --- add: project scope, repository root, the two agents that read project skills ---------
tree=$(project add-root)
: > "$LOG"
run "$tree/sub/dir" add remotion-dev/skills
[ "$status" -eq 0 ] || { echo "FAIL: add should exit 0, got $status"; fails=$((fails + 1)); }
expect "add resolves the repository root" "project-scoped install into $tree" 1
expect "add names pi's project directory" "$tree/.agents/skills" 1
expect "add names Claude Code's project directory" "$tree/.claude/skills" 1
expect "add passes the package through" "add remotion-dev/skills" 0
expect_log "add passes the package through" "add remotion-dev/skills" 1
expect_log "add installs for pi" "-a pi" 1
expect_log "add installs for Claude Code" "-a claude-code" 1
expect_log "add runs non-interactively" "-y" 1
if grep -q -- " -g " "$LOG"; then
  echo "FAIL: add reached npx with -g"
  cat "$LOG" | sed 's/^/    /'
  fails=$((fails + 1))
fi

# --- add --skill / --agent: explicit selection replaces the default agent pair -------------
tree=$(project add-narrow)
: > "$LOG"
run "$tree" add remotion-dev/skills --skill remotion-maps --agent codex
expect_log "add forwards --skill" "-s remotion-maps" 1
expect_log "add forwards --agent" "-a codex" 1
expect_log "explicit --agent replaces the default" "-a claude-code" 0

# --- add: the stage is fetched for Claude Code only and gated against the installed copy -----
tree=$(project add-gated)
: > "$LOG"
run "$tree" add remotion-dev/skills
if ! grep -q "add remotion-dev/skills -a claude-code -y  \[cwd $SCRATCH/state/sjel/intake/staged/" "$LOG"; then
  echo "FAIL: add should fetch into the intake stage for Claude Code only"
  sed 's/^/    /' "$LOG"
  fails=$((fails + 1))
fi
if ! grep -q "^intake gate .*/staged-skill --source skills:remotion-dev/skills/staged-skill --against $tree/.claude/skills/staged-skill" "$LOG"; then
  echo "FAIL: add should gate the staged skill against the installed copy"
  sed 's/^/    /' "$LOG"
  fails=$((fails + 1))
fi

# --- add: a held package installs nothing --------------------------------------------------
tree=$(project add-held)
: > "$LOG"
SKILLS_TEST_GATE_RC=2 run "$tree" add remotion-dev/skills
[ "$status" -eq 2 ] || { echo "FAIL: a held add should exit 2, got $status"; fails=$((fails + 1)); }
expect "a held add says so" "nothing installed: remotion-dev/skills is held" 1
if [ -n "$(installs)" ]; then
  echo "FAIL: a held add still installed"
  sed 's/^/    /' "$LOG"
  fails=$((fails + 1))
fi

# --- add --global: refused, and npx is never reached ---------------------------------------
tree=$(project add-global)
: > "$LOG"
run "$tree" add remotion-dev/skills --global
[ "$status" -eq 1 ] || { echo "FAIL: add --global should exit 1, got $status"; fails=$((fails + 1)); }
expect "refusal states the rule" "refusing --global" 1
expect "refusal names the Pack route" "tools/packs-pi deploy" 1
if [ -s "$LOG" ]; then
  echo "FAIL: a refused add still invoked npx"
  cat "$LOG" | sed 's/^/    /'
  fails=$((fails + 1))
fi

# --- audit: a global skill the CLI installed, which no Pack owns ---------------------------
cat > "$GLOBAL_JSON" <<'JSON'
[
  {"name": "remotion-maps", "scope": "global", "source": "remotion-dev/skills", "agents": ["Pi", "Claude Code"]},
  {"name": "obsidian", "scope": "global", "source": null, "agents": ["Claude Code"]},
  {"name": "interceptor", "scope": "global", "source": null, "agents": ["Pi"]}
]
JSON
run "$SCRATCH" audit
[ "$status" -eq 1 ] || { echo "FAIL: audit with a violation should exit 1, got $status"; fails=$((fails + 1)); }
expect "audit names the offending skill" "remotion-maps" 1
expect "audit reports where it came from" "source=remotion-dev/skills" 1
expect "audit prints the fix" "tools/skills remove -g -s remotion-maps -y" 1
expect "audit does not flag a Pack-deployed skill" "obsidian" 0
expect "audit does not flag an adopted external install" "interceptor" 0

# --- audit: clean --------------------------------------------------------------------------
cat > "$GLOBAL_JSON" <<'JSON'
[
  {"name": "obsidian", "scope": "global", "source": null, "agents": ["Claude Code"]},
  {"name": "svelte-code-writer", "scope": "global", "source": null, "agents": ["Claude Code"]}
]
JSON
run "$SCRATCH" audit
[ "$status" -eq 0 ] || { echo "FAIL: clean audit should exit 0, got $status"; fails=$((fails + 1)); }
expect "clean audit says so" "no skill is installed globally outside a Sjel Pack" 1

# --- unknown verb --------------------------------------------------------------------------
run "$SCRATCH" frobnicate
[ "$status" -eq 1 ] || { echo "FAIL: unknown verb should exit 1, got $status"; fails=$((fails + 1)); }
expect "unknown verb says how to get help" "tools/skills --help" 1

if [ "$fails" -eq 0 ]; then
  echo "✓ tools/skills: all cases pass"
else
  echo "✗ tools/skills: $fails case(s) failed"
  exit 1
fi

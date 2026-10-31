#!/bin/bash
# agent-integrations.sh — install an adopted upstream's OWN agent-harness integration,
# at that upstream's latest release.
#
# Some upstreams ship an agent integration alongside the tool: a skill, a hook, a plugin.
# This script drives their own installers instead of Sjel keeping a copy of what they emit.
#
# ## Adding an upstream
#
# Add a row to INTEGRATIONS below, a family of <id>_* functions, and a case arm in
# integration_detect / integration_install / integration_write_marker /
# integration_harnesses / integration_description. Never write a version here or into
# upstreams.toml: an upstream's integration is consumed at its latest release
# (CONTRIBUTING.md#patch-first).
#
# Usage:
#   tools/agent-integrations.sh list                              what is available, and where
#   tools/agent-integrations.sh status                            what is installed on this machine
#   tools/agent-integrations.sh status --json                     machine-readable status for tooling
#   tools/agent-integrations.sh status --machine                  key/value-ish status, one line per harness
#   tools/agent-integrations.sh install <upstream> <harness>...   e.g. install interceptor claude
#   tools/agent-integrations.sh install --all-configured          every configured harness x every upstream
#   tools/agent-integrations.sh update [<upstream>...]            each upstream's native update tooling
#   integration detection states: missing / runnable / configured / stale / integrated
#
# Exit 0 = done / nothing to do, 1 = usage or a missing prerequisite.

set -euo pipefail

# Not named _lib: paths.sh ends with `unset _lib`, which would take this one with it and
# leave the next source dereferencing an unset var under `set -u`.
_ai_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_ai_lib/paths.sh"
# toml.sh is no longer sourced here. It existed to read `pin` out of upstreams.toml, and that
# field is deleted (Q77, 2026-09-02) -- this script reads no manifest at all now.

# Every upstream this tool can install, status and update.
INTEGRATIONS="interceptor"

# Harnesses Sjel knows how to detect, and where each keeps its global config.
harness_command() {
  case "$1" in
    claude)   echo "claude" ;;
    opencode) echo "opencode" ;;
    codex)    echo "codex" ;;
    pi)       echo "pi" ;;
    copilot)  echo "copilot" ;;
    *)        echo "" ;;
  esac
}

harness_config_dir() {
  case "$1" in
    claude)   echo "$HOME/.claude" ;;
    opencode) echo "${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}" ;;
    codex)    echo "$HOME/.codex" ;;
    pi)       echo "$HOME/.pi" ;;
    copilot)  echo "$HOME/.copilot" ;;
    *)        echo "" ;;
  esac
}

# Where each harness keeps its SKILLS, distinct from its config dir: for pi that is
# ~/.pi/agent/skills, not ~/.pi/skills.
harness_skill_root() {
  case "$1" in
    pi)       echo "$HOME/.pi/agent/skills" ;;
    claude)   echo "$HOME/.claude/skills" ;;
    opencode) echo "${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}/skills" ;;
    codex)    echo "$HOME/.codex/skills" ;;
    copilot)  echo "$HOME/.copilot/skills" ;;
    *)        echo "" ;;
  esac
}
HARNESSES="claude opencode codex pi copilot"
json_escape() {
  local text="$1"
  text="${text//\\/\\\\}"
  text="${text//\"/\\\"}"
  text="${text//$'\n'/\\n}"
  text="${text//$'\r'/\\r}"
  printf '%s' "$text"
}

cmd_status() { # cmd_status [json|machine]
  local output="${1:-human}"
  local h row state dir_present command runnable command_present configured integration_ready stale dir skill plugin command_version install_command
  local first="1"

  if [ "$output" = "json" ]; then
    printf '{"integrations": ['
  fi
  local u first_upstream="1"
  for u in $INTEGRATIONS; do
    if [ "$output" = "json" ]; then
      [ "$first_upstream" = "0" ] && printf ','
      first_upstream="0"
      printf '{ "upstream": "%s", "harnesses": [' "$u"
      first="1"   # per-upstream row comma flag — each array starts bare
    elif [ "$output" = "machine" ]; then
      :
    else
      echo "$(integration_description "$u") (${u}; installed at latest — never a recorded version, Q77):"
    fi
    local h
    for h in $(integration_harnesses "$u"); do
      row="$(integration_detect "$u" "$h")"
    IFS='|' read -r harness state dir_present command runnable command_present configured integration_ready stale dir skill plugin command_version install_command <<<"$row"
    if [ "$output" = "json" ]; then
      if [ "$first" = "0" ]; then
        printf ','
      fi
      first="0"
      printf '{'
      printf '"name":"%s",' "$(json_escape "$h")"
      printf '"state":"%s",' "$(json_escape "$state")"
      printf '"runnable":%s,' "$( [ "$runnable" = "yes" ] && printf true || printf false )"
      printf '"configured":%s,' "$( [ "$configured" = "yes" ] && printf true || printf false )"
      printf '"integrated":%s,' "$( [ "$state" = "integrated" ] && printf true || printf false )"
      printf '"stale":%s,' "$( [ "$stale" = "yes" ] && printf true || printf false )"
      printf '"command":"%s",' "$( [ -n "$command" ] && json_escape "$command" || echo "" )"
      printf '"config_dir":"%s",' "$(json_escape "$dir")"
      printf '"skill":"%s",' "$(json_escape "$skill")"
      printf '"plugin":"%s",' "$(json_escape "$plugin")"
      printf '"command_version":"%s",' "$(json_escape "$command_version")"
      printf '"command_present":%s' "$( [ "$command_present" = "yes" ] && printf true || printf false )"
      printf ',"install_command":"%s"' "$(json_escape "$install_command")"
      printf '}'
    elif [ "$output" = "machine" ]; then
      printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
        "$h" "$state" "$dir_present" "$command" "$runnable" "$command_present" \
        "$configured" "$integration_ready" "$stale" "$dir" \
        "$(json_escape "$skill")" "$(json_escape "$plugin")" "$(json_escape "$command_version")" \
        "$(json_escape "$install_command")"
    else
      printf '  %-9s state=%-11s skill=%-4s plugin=%-4s %s\n' \
        "$h" "$state" "$skill" "$plugin" "$dir"
    fi
    done
    if [ "$output" = "json" ]; then
      printf ' ] }'
    fi
  done
  if [ "$output" = "json" ]; then
    printf '] }'
  fi
}

# ── interceptor ────────────────────────────────────────────────────────────
# A product (not a uv/npm tool): CLI at /usr/local/bin/interceptor, product root at
# /Library/Application Support/Interceptor. It OWNS five skills, which its own `skills`
# verb symlinks into Claude Code / Codex / ~/.agents (upstreams.toml [interceptor]: never
# vendor them into Packs/). Version comes from the CLI at status time — the row in
# upstreams.toml is asked of the runtime, never held (Q77, C18). Native update: `update`.

interceptor_product_dir() {
  [ -d "/Library/Application Support/Interceptor/skills" ] && echo "/Library/Application Support/Interceptor"
}

interceptor_skill_names() {
  local root
  root="$(interceptor_product_dir)"
  [ -z "$root" ] && return 0
  ls "$root/skills" 2>/dev/null
}

# interceptor adopts into Claude Code and Codex (~/.agents) through its own `skills` verb. The
# two rows below it are OURS to place, because that verb accepts only claude/codex/agents: the
# product still owns the content (a symlink to its installed skill root, never a copy), we only
# create the link it refuses to. Recorded 2026-10-24, when opencode carried no
# interceptor skills, and copilot carried neither integration at all.
interceptor_harness_dir() {  # <harness>
  case "$1" in
    claude)   echo "$HOME/.claude" ;;
    codex)    echo "$HOME/.agents" ;;
    opencode) echo "$HOME/.config/opencode" ;;
    copilot)  echo "$HOME/.copilot" ;;
    *)        echo "" ;;
  esac
}

# The harnesses the product's own verb adopts into. Every other row in the table above is
# linked here instead, by us.
interceptor_self_adopting() {  # <harness> -> yes|no
  case "$1" in
    claude | codex) echo "yes" ;;
    *)              echo "no" ;;
  esac
}

interceptor_detect() {  # <harness> -> the shared 14-field row
  local harness="$1" dir root total n name link runnable state configured skill
  local marker marker_present command_version install_command
  dir="$(interceptor_harness_dir "$harness")"
  [ -n "$dir" ] || { printf '%s|missing|no|interceptor|no|no|no|no|no||no|n/a|unknown|interceptor skills\n' "$harness"; return 0; }
  root="$(interceptor_product_dir)"
  runnable="no"; command_version="unknown"
  if command -v interceptor >/dev/null 2>&1; then
    runnable="yes"
    command_version="$(interceptor --version 2>/dev/null | head -n 1 | tr -d '\r\n')"
  fi
  skill="no"; n=0; total="$(interceptor_skill_names | wc -l | tr -d ' ')"
  [ -n "$root" ] && [ "$total" -gt 0 ] && {
    for name in $(interceptor_skill_names); do
      local link="$dir/skills/$name"
      [ -L "$link" ] && [ "$(readlink "$link")" = "$root/skills/$name" ] && n=$((n + 1))
    done
    [ "$n" -eq "$total" ] && skill="yes"
  }
  configured="no"; [ "$skill" = "yes" ] && configured="yes"
  marker="$dir/.interceptor-skills-axoned"
  marker_present="no"
  [ -s "$marker" ] && marker_present="yes"
  state="missing"
  if [ "$runnable" = "yes" ] && [ "$configured" = "yes" ] && [ "$marker_present" = "yes" ]; then
    state="integrated"
  elif [ "$configured" = "yes" ]; then
    state="stale"
  elif [ "$runnable" = "yes" ]; then
    state="runnable"
  fi
  install_command="interceptor skills"
  printf '%s|%s|yes|interceptor|%s|%s|%s|%s|%s|%s|%s|n/a|%s|%s\n' \
    "$harness" "$state" "$runnable" "$runnable" "$configured" "$skill" "$([ "$state" = "stale" ] && echo yes || echo no)" \
    "$dir" "$skill" "$command_version" "$install_command"
}

interceptor_install() {  # <harness> — the product re-adopts its own skills, or we link them
  command -v interceptor >/dev/null 2>&1 || { echo "✗ interceptor CLI not found — install the product first" >&2; return 1; }
  if [ "$(interceptor_self_adopting "$1")" = "yes" ]; then
    interceptor skills
    return 0
  fi
  local dir root name total=0 linked=0
  dir="$(interceptor_harness_dir "$1")"
  root="$(interceptor_product_dir)"
  [ -n "$dir" ] && [ -n "$root" ] || { echo "✗ interceptor product root or $1 skill root not found" >&2; return 1; }
  mkdir -p "$dir/skills" || return 1
  for name in $(interceptor_skill_names); do
    total=$((total + 1))
    ln -sfn "$root/skills/$name" "$dir/skills/$name" || { echo "  x failed to link $1 skill $name" >&2; return 1; }
    linked=$((linked + 1))
  done
  echo "  linked $linked/$total skill(s) -> $dir/skills (the product's own root; its \`skills\` verb does not accept $1)"
}

interceptor_write_marker() {  # <harness>
  local dir
  dir="$(interceptor_harness_dir "$1")"
  [ -n "$dir" ] && [ -d "$dir" ] && date -u +%Y-%m-%d > "$dir/.interceptor-skills-axoned"
}

# ── the table (C15) ────────────────────────────────────────────────────────
# One row per upstream: which harnesses it ships for, what a status line looks like,
# how it installs and how it is updated. Adding an upstream = one row here, its own
# <id>_* functions, and case arms below — never edits inside another upstream's logic.

integration_harnesses() {
  case "$1" in
    interceptor) echo "claude codex opencode copilot" ;;
  esac
}

integration_description() {
  case "$1" in
    interceptor) echo "browser/macOS control product; owns five skills via its own installer" ;;
  esac
}

integration_detect() {  # <upstream> <harness> -> the shared 14-field row
  case "$1" in
    interceptor) interceptor_detect "$2" ;;
  esac
}

integration_install() {  # <upstream> <harness>
  case "$1" in
    interceptor) interceptor_install "$2" ;;
  esac
}

integration_write_marker() {  # <upstream> <harness>
  case "$1" in
    interceptor) interceptor_write_marker "$2" ;;
  esac
}

# ── dispatch ────────────────────────────────────────────────────────────────

configured_harnesses() {
  local h dir
  for h in $HARNESSES; do
    dir="$(harness_config_dir "$h")"
    [ -n "$dir" ] && [ -d "$dir" ] && echo "$h"
  done
}

cmd_list() {
  echo "Agent-harness integrations shipped by adopted upstreams:"
  echo
  local u
  for u in $INTEGRATIONS; do
    printf '  %-12s %-58s harnesses: %s\n' "$u" "$(integration_description "$u")" "$(integration_harnesses "$u")"
  done
  echo
  echo "Installed at upstream's latest, never a recorded version (Q77)."
  echo "Update: tools/agent-integrations.sh update [<upstream>...] — each upstream's own updater."
  echo "Configured on this machine: $(configured_harnesses | tr '\n' ' ')"
  echo
  echo "Optional upstream packages (explicit opt-in; excluded from --all-configured):"
  echo "  accordion    Pi context-window management — https://github.com/a-Fig/Accordion"
  echo "               tools/agent-integrations.sh install accordion pi"
  echo "               delegates to: pi install npm:@a-fig/accordion; updates: pi update"
}

cmd_install() {
  local upstream="" targets=""
  if [ "${1:-}" = "accordion" ]; then
    if [ "$#" -ne 2 ] || [ "$2" != "pi" ]; then
      echo "usage: agent-integrations.sh install accordion pi" >&2
      return 1
    fi
    command -v pi >/dev/null 2>&1 || { echo "pi is required to install Accordion" >&2; return 1; }
    pi install npm:@a-fig/accordion
    return
  fi
  if [ "${1:-}" = "--all-configured" ] || [ "${1:-}" = "--all-detected" ] || [ "${1:-}" = "--all" ]; then
    targets="$(configured_harnesses)"
    [ -n "$targets" ] || { echo "No known harness config dirs found — nothing to do."; return 0; }
    local u
    for u in $INTEGRATIONS; do
      _install_one_upstream "$u" "$targets"
    done
    echo
    echo "Restart the harness to pick up new plugins (most load them once at startup)."
    return 0
  fi
  case " $INTEGRATIONS " in
    *" ${1:-} "*) upstream="$1"; shift ;;
  esac
  targets="$*"
  [ -n "$upstream" ] && [ -n "$targets" ] || { echo "usage: agent-integrations.sh install <upstream> <harness>... | --all-configured" >&2; exit 1; }
  _install_one_upstream "$upstream" "$targets"
  echo
  echo "Restart the harness to pick up new plugins (most load them once at startup)."
}

_install_one_upstream() {  # <upstream> <harness>...
  local upstream="$1"; shift
  local targets="$*" h
  for h in $targets; do
    if [ -z "$(harness_config_dir "$h")" ]; then
      echo "✗ unknown harness '$h' (known: $HARNESSES)" >&2
      exit 1
    fi
    echo "$upstream -> $h"
    integration_install "$upstream" "$h"
    integration_write_marker "$upstream" "$h"
  done
}

# C17: update checking lives HERE — a networked verb that calls each upstream's own
# updater — and deliberately not in doctor, which stayed offline by PRD Q41's ruling.
cmd_update() {  # update [<upstream>...]
  local list="${1:-$INTEGRATIONS}" u
  for u in $list; do
    case " $INTEGRATIONS " in
      *" $u "*) ;;
      *) echo "✗ unknown upstream '$u' (known: $INTEGRATIONS)" >&2; exit 1 ;;
    esac
    echo "── $u"
    case "$u" in
      interceptor)
        # `interceptor update`, NOT `interceptor upgrade`. Measured 2026-10-01 on 1.0.19:
        # `upgrade` is "Promote browser-only install to full computer-use mode (macOS)" and exits
        # 1 without `--full`, so `interceptor skills` — the second half of the pair — had never
        # once run, and the harnesses' skills marker kept whatever date it was given by hand.
        # `update` is the verb that asks the product's own appcast
        # (https://updates.hackervalley.media/appcast.xml); `skills` re-adopts the five skills
        # the product owns. upstreams.toml [interceptor] said "its own update/upgrade verbs keep
        # it current" — the half that was true is `update`, and this is that half.
        if command -v interceptor >/dev/null 2>&1; then
          # The marker records the DATE the skills were last re-adopted, and `sjel update` reads
          # that date to say whether the integration has drifted. Without the write below the
          # branch refreshed the skills and left the date at whatever it had been, so a
          # successful update still read as three weeks stale.
          if interceptor update && interceptor skills; then
            local h
            for h in $(configured_harnesses); do
              interceptor_write_marker "$h"
            done
          else
            echo "  ✗ interceptor update or skills failed — the marker is left alone so the\n    integration keeps reading as stale rather than being dated by a failed run" >&2
          fi
        else
          echo "  interceptor CLI not found — install the product first (its own installer)" >&2
        fi
        ;;
    esac
  done
}

case "${1:-list}" in
  list|--list)     cmd_list ;;
  status|--status) shift; cmd_status "${1#--}" ;;
  install)         shift; cmd_install "$@" ;;
  update)          shift; cmd_update "$@" ;;
  -h|--help|help)  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) echo "agent-integrations.sh: unknown command '$1' (try: list|status|install|update|-h)" >&2; exit 1 ;;
esac

#!/bin/bash
# tools/install.sh — first-time Axon setup. Detects OS, resolves where the
# private overlay (axon-overlay) lives, scaffolds or clones it, writes this
# machine's machine.toml, then hands off to the harness-specific Pack
# deployers for optional activation.
#
# Idempotent: safe to re-run on an already-set-up machine — every step
# detects existing state and leaves it alone instead of overwriting. It never migrates an
# existing install: the overlay location is per-machine and changes only when someone answers
# the prompt differently, and nothing here special-cases an older default path.
#
#   tools/install.sh          # run the guided setup
#   tools/install.sh -h       # this help
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SJEL_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"

case "${1:-}" in
  -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
esac

source "$TOOLS_DIR/lib/toml.sh"

# Write only the one safety setting Axon owns in the laptop-wide Bun config. Do not put the
# scanner here: Bun requires a configured scanner to be a dependency of every project, so a
# global scanner block would break unrelated checkouts. This function preserves every other
# line and key in an existing file; it only creates or replaces [install].minimumReleaseAge.
write_bun_global_hold() {
  local path="$HOME/.bunfig.toml"
  local tmp="${path}.tmp.$$"
  if [ -f "$path" ]; then
    awk '
      BEGIN { in_install = 0; found_install = 0; found_age = 0 }
      /^[[:space:]]*\[install\][[:space:]]*$/ {
        if (in_install && !found_age) print "minimumReleaseAge = 86400"
        in_install = 1; found_install = 1
        print
        next
      }
      /^[[:space:]]*\[[^]]+\][[:space:]]*$/ {
        if (in_install && !found_age) print "minimumReleaseAge = 86400"
        in_install = 0
        print
        next
      }
      in_install && /^[[:space:]]*minimumReleaseAge[[:space:]]*=/ {
        print "minimumReleaseAge = 86400"
        found_age = 1
        next
      }
      { print }
      END {
        if (in_install && !found_age) print "minimumReleaseAge = 86400"
        if (!found_install) {
          print ""
          print "[install]"
          print "minimumReleaseAge = 86400"
        }
      }
    ' "$path" > "$tmp"
    mv "$tmp" "$path"
  else
    printf '%s\n' '[install]' 'minimumReleaseAge = 86400' > "$path"
  fi
  echo "Bun install policy: ~/.bunfig.toml now holds new npm releases for 24h."
}

# `sjel` is the public human/agent interface, and `axon` is its earlier name (a symlink in the
# checkout), so installation puts both launchers in the conventional user bin directory. Never
# replace an unrelated command: an existing non-symlink requires an explicit operator decision
# instead of silently changing their PATH behavior.
SJEL_BIN_DIR="$HOME/.local/bin"
for cli in sjel axon; do
  SJEL_BIN="$SJEL_BIN_DIR/$cli"
  if [ -L "$SJEL_BIN" ] && [ "$(readlink "$SJEL_BIN")" = "$SJEL_ROOT/$cli" ]; then
    echo "Sjel CLI: $SJEL_BIN already points at this checkout."
  elif [ -e "$SJEL_BIN" ] || [ -L "$SJEL_BIN" ]; then
    echo "Sjel CLI: $SJEL_BIN already exists and is not owned by this checkout, leaving it alone."
    echo "  Run this checkout directly: $SJEL_ROOT/$cli help"
  else
    mkdir -p "$SJEL_BIN_DIR"
    ln -s "$SJEL_ROOT/$cli" "$SJEL_BIN"
    echo "Sjel CLI: linked $SJEL_BIN -> $SJEL_ROOT/$cli"
  fi
done
case ":$PATH:" in
  *":$SJEL_BIN_DIR:"*) ;;
  *) echo "Sjel CLI: add $SJEL_BIN_DIR to PATH to run 'sjel' from any directory." ;;
esac

# Where THIS machine's overlay location gets recorded. Gitignored and per-machine, so a
# second node never has to edit the tracked axon.toml — that file keeps only a neutral
# shipped default (consumed by tools/lib/paths.sh as the fallback, not here). See
# schemas/machine.toml.example.
SJEL_LOCAL_TOML="$SJEL_ROOT/axon.local.toml"
SKELETON="$TOOLS_DIR/templates/overlay-skeleton"

echo "Axon installer"
echo "==============="

# Which profile this checkout IS, read off the checkout rather than passed in — bootstrap.sh
# is one way to arrive here and `git clone` by hand is the other, so a flag would be right
# only half the time. A usage install behaves differently in ways the operator will meet
# later (tools/update.sh cannot compute a delta against a ref a tag-pinned clone does not
# have, #58), so say it once, here, instead of letting it be discovered as a malfunction.
if [ "$(git -C "$SJEL_ROOT" rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
  echo "Profile: usage — shallow clone pinned to $(git -C "$SJEL_ROOT" describe --tags 2>/dev/null || echo 'a tag')."
  echo "  To change Axon rather than run it, promote this checkout:"
  echo "    git -C \"$SJEL_ROOT\" config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'"
  echo "    git -C \"$SJEL_ROOT\" fetch --unshallow origin"
elif git -C "$SJEL_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
  echo "Profile: development — full history."
else
  echo "Profile: not a git checkout — tools/update.sh and the version line will not work."
fi
echo

# 1) OS detection — platform.sh can't run yet (needs machine.toml, which
# doesn't exist until this script writes it), so detect directly.
case "$(uname -s)" in
  Darwin) DETECTED_OS="macos" ;;
  Linux)  DETECTED_OS="linux" ;;
  *)      DETECTED_OS="" ;;
esac

if [ -n "$DETECTED_OS" ]; then
  echo
  echo "Detected OS: $DETECTED_OS"
else
  echo
  echo "Could not auto-detect OS from 'uname -s' ($(uname -s))."
  read -r -p "Enter OS [macos/linux/windows]: " DETECTED_OS
fi

# 2) Overlay location — the single write point is axon.local.toml's `overlay` field;
# tools/lib/paths.sh resolves everything downstream from it, so this is the only place
# a location decision has to be made. An already-configured machine's own value wins as
# the prompt default; the tracked axon.toml is only consulted for the shipped fallback.
CURRENT_OVERLAY=""
if [ -f "$SJEL_LOCAL_TOML" ]; then
  CURRENT_OVERLAY="$(toml_get overlay "$SJEL_LOCAL_TOML")"
fi

# The suggested path is deliberately generic. Deployment ownership and topology belong inside
# the private overlay, not in a public list of operator-specific repository names.
OVERLAY_NAME="deployment"
echo
if [ -n "$CURRENT_OVERLAY" ]; then
  # Already configured: this deployment's own value is the default.
  read -r -p "Overlay location [$CURRENT_OVERLAY]: " OVERLAY_INPUT
  OVERLAY_RAW="${OVERLAY_INPUT:-$CURRENT_OVERLAY}"
else
  SUGGESTED_OVERLAY="~/.axon-overlay"
  read -r -p "Overlay location [$SUGGESTED_OVERLAY]: " OVERLAY_INPUT
  OVERLAY_RAW="${OVERLAY_INPUT:-$SUGGESTED_OVERLAY}"
fi
OVERLAY_PATH="${OVERLAY_RAW/#\~/$HOME}"

if [ "$OVERLAY_RAW" != "$CURRENT_OVERLAY" ]; then
  if [ -f "$SJEL_LOCAL_TOML" ]; then
    toml_set overlay "$OVERLAY_RAW" "$SJEL_LOCAL_TOML"
  else
    cat > "$SJEL_LOCAL_TOML" <<EOF
# This machine's overlay location — gitignored, one per machine. Written by
# tools/install.sh; tools/lib/paths.sh reads it and falls back to axon.toml's
# shipped default. Everything else that differs per machine lives in that
# overlay's own config/machine.toml.
overlay = "$OVERLAY_RAW"
EOF
  fi
  echo "axon.local.toml: overlay = \"$OVERLAY_RAW\""
fi

# 3) Scaffold or clone the overlay
echo
if [ -d "$OVERLAY_PATH" ]; then
  echo "Overlay already exists at $OVERLAY_PATH — leaving contents alone."
else
  echo "Nothing at $OVERLAY_PATH yet."
  read -r -p "(c)lone an existing private git remote, or (s)caffold a fresh skeleton? [c/s] " MODE
  case "$MODE" in
    c|C)
      read -r -p "Git remote URL: " REMOTE_URL
      [ -n "$REMOTE_URL" ] || { echo "install.sh: no URL given" >&2; exit 1; }
      git clone "$REMOTE_URL" "$OVERLAY_PATH"
      ;;
    *)
      mkdir -p "$OVERLAY_PATH"
      cp -R "$SKELETON/." "$OVERLAY_PATH/"
      # Stamp the generic deployment label into the scaffolded README and allowlist (they ship
      # the __OVERLAY_NAME__ placeholder — see tools/templates/overlay-skeleton). sed -i.bak +
      # rm is the portable in-place form: GNU and BSD sed disagree on bare -i (CONTRIBUTING.md#portable-shell).
      for f in "$OVERLAY_PATH/README.md" "$OVERLAY_PATH/.ignore-allowlist"; do
        [ -f "$f" ] && sed -i.bak "s/__OVERLAY_NAME__/$OVERLAY_NAME/g" "$f" && rm -f "$f.bak"
      done
      # .gitignore is generated, not copied (Axon#194). The skeleton ships only the allowlist —
      # what this overlay has reviewed — and Axon renders the deny-all above it and the
      # immutable hard blocks below it. Two overlays hand-maintaining the same floor is exactly
      # how that floor drifts, and `tools/ignore-policy check` is what notices.
      "$TOOLS_DIR/ignore-policy" render "$OVERLAY_PATH" \
        || { echo "install.sh: could not render the overlay ignore policy" >&2; exit 1; }
      git -C "$OVERLAY_PATH" init -q
      echo "Scaffolded fresh overlay at $OVERLAY_PATH (local git repo, no remote)."
      echo "Add a private remote yourself when ready:"
      echo "  git -C \"$OVERLAY_PATH\" remote add origin <url>"
      ;;
  esac
fi

# 4) machine.toml — this machine's platform identity, read by
# tools/lib/platform.sh. Never overwritten if already present.
MACHINE_TOML="$OVERLAY_PATH/config/machine.toml"
mkdir -p "$(dirname "$MACHINE_TOML")"

echo
if [ -f "$MACHINE_TOML" ]; then
  echo "machine.toml already present at $MACHINE_TOML — leaving it alone."
else
  DEFAULT_RUNTIME=""
  case "$DETECTED_OS" in
    macos) DEFAULT_RUNTIME="docker" ;;
    linux) DEFAULT_RUNTIME="docker" ;;
  esac
  if [ -n "$DEFAULT_RUNTIME" ]; then
    read -r -p "Container runtime [docker/podman] [$DEFAULT_RUNTIME]: " RUNTIME_INPUT
    CONTAINER_RUNTIME="${RUNTIME_INPUT:-$DEFAULT_RUNTIME}"
  else
    read -r -p "Container runtime [docker/podman]: " CONTAINER_RUNTIME
  fi
  cat > "$MACHINE_TOML" <<EOF
# This machine's identity — every fact that is true of THIS deployment instance and
# not of Axon itself. One overlay is one instance, which is why this file can afford
# to describe exactly one machine. See
# schemas/machine.toml.example.

# Platform. tools/lib/platform.sh reads these so capability scripts branch on them
# instead of assuming macOS (or any other OS).
os = "$DETECTED_OS"
container_runtime = "$CONTAINER_RUNTIME"

# Capabilities enabled here — managed by tools/capability.sh (enable/disable resolve
# service.toml \`requires =\` transitively). Hand-editing is legal; tools/doctor
# re-checks that the set stays dependency-closed. Single-line array per
# tools/lib/toml.sh's contract.
capabilities = []

# Where tools on this machine actually persist data, in each tool's own default place.
# data_class = "public" | "personal" | "vault"; sync = "git" | "restic" | "rsync" | "none"
# direction = "capture" (tool -> overlay backup) | "inject" (overlay -> tool) | "both"
# Add one [[state_mount]] block per tool; tools/doctor checks each path exists and has
# a matching systems.toml identity. Example:
#
# [[state_mount]]
# tool = "some-tool"
# path = "~/some/dir"
# data_class = "personal"
# sync = "none"
# direction = "both"
# monitor = true
EOF
  echo "Wrote $MACHINE_TOML"
fi

# 4.5) Host toolchain report — is every tool Axon's own scripts assume actually installed?
# Report-only: a fresh box legitimately installs tools as it goes, so a missing required
# tool prints its install hint but never aborts the setup (the `|| true` is load-bearing
# under `set -e`). Runs here because the container runtime is now known (freshly chosen
# above, or already in machine.toml, which toolchain-check falls back to). Pure bash, so it
# works during this bash-only bootstrap before bun is guaranteed on PATH — unlike doctor,
# which runs the same tools/toolchain-check afterwards for the ongoing check.
echo
echo "Host toolchain (tools/toolchain-check):"
RUNTIME_ARG=""
[ -n "${CONTAINER_RUNTIME:-}" ] && RUNTIME_ARG="--runtime $CONTAINER_RUNTIME"
# shellcheck disable=SC2086  # RUNTIME_ARG is a deliberate two-token flag or empty
"$TOOLS_DIR/toolchain-check" --os "$DETECTED_OS" $RUNTIME_ARG || true

# 5) Hand off to Pack selection
echo
echo "Available Packs:"
"$TOOLS_DIR/packs.sh" list
echo "Activate for Claude Code: tools/packs.sh link <name>"
echo "Deploy for Codex:        tools/packs-codex deploy <name>"
echo "Deploy for OpenCode:     tools/packs-opencode deploy <name>"
echo "Deploy for Pi:           tools/packs-pi deploy <name>"

# 6) Deploy Axon's baseline Claude Code harness settings (auto permission mode, etc.)
# into ~/.claude/settings.json — a general default Axon delivers on every machine, not
# just inside this repo. Merge-only, existing keys always win. Post-bootstrap: needs bun
# (not guaranteed during the bash-only bootstrap above), so skip with a pointer rather
# than failing setup if it isn't on PATH yet. Runs before the capability prompt so it
# still happens on a non-TTY install (which exits early below).
echo
if command -v bun >/dev/null 2>&1; then
  "$TOOLS_DIR/claude-code-config" || echo "  (baseline settings step skipped — re-run: tools/claude-code-config)"
  # The managed security policy (macOS: /Library/Application Support/ClaudeCode, Linux:
  # /etc/claude-code) is opt-in and needs root, so it is a
  # deliberate step rather than part of the guided install — just point at it.
  echo "  Optional hardening: tools/claude-code-config --managed  (deploys the managed security policy; needs sudo)"
else
  echo "Claude Code baseline settings: skipped ('bun' not on PATH yet)."
  echo "  Apply them later with: tools/claude-code-config"
fi

# 6b) Agent-harness integrations shipped by adopted upstreams (graphify's skill + hook
# today). Offered here because this is the "AI assistants" region of the install — right
# after Packs and the Claude Code baseline — and because the alternative is what actually
# happened before: you run an upstream's own installer by hand, from whatever directory
# you were standing in, and it writes a project-local integration somewhere it can never
# fire from. Driving it through this script, from a scratch cwd, is the fix.
#
# Detection-driven and non-interactive-safe: with no harness config dir present it prints
# a line and moves on. Never installs without being asked — `read` is skipped on non-TTY
# the same way the capability prompt below is.
echo
echo "Agent-harness integrations (each upstream's own installer, at its latest release):"
AI_STATE="$("$TOOLS_DIR/agent-integrations.sh" status --machine)"
echo "$AI_STATE" | sed 's/^/  /'

AI_INSTALL_TARGETS=()
AI_INSTALL_COMMANDS=()
AI_HAS_REAL_GRAPH="no"
while IFS='|' read -r AI_HARNESS AI_STATE_NAME _ _ _ _ _ _ _ AI_GRAPH_STATE _ _ _ _ AI_INSTALL_COMMAND; do
  [ -n "$AI_HARNESS" ] || continue
  if [ "$AI_GRAPH_STATE" = "present" ]; then
    AI_HAS_REAL_GRAPH="yes"
  fi
  if [ "$AI_GRAPH_STATE" != "present" ] || { [ "$AI_STATE_NAME" != "runnable" ] && [ "$AI_STATE_NAME" != "configured" ] && [ "$AI_STATE_NAME" != "stale" ]; }; then
    continue
  fi
  AI_INSTALL_TARGETS+=( "$AI_HARNESS" )
  [ -n "$AI_INSTALL_COMMAND" ] || AI_INSTALL_COMMAND="tools/agent-integrations.sh install ${AI_HARNESS}"
  AI_INSTALL_COMMANDS+=( "$AI_INSTALL_COMMAND" )
done <<< "$AI_STATE"

if [ -t 0 ]; then
  if [ "${AI_HAS_REAL_GRAPH}" = "yes" ] && [ "${#AI_INSTALL_TARGETS[@]}" -gt 0 ]; then
    AI_INSTALL_TARGETS_JOINED="${AI_INSTALL_TARGETS[*]}"
    read -r -p "Install suggested integrations for: [${AI_INSTALL_TARGETS_JOINED}]? [y/N]: " AI_INPUT
    case "${AI_INPUT:-n}" in
      y|Y|yes)
        echo "  Exact commands:"
        for AI_INSTALL_COMMAND in "${AI_INSTALL_COMMANDS[@]}"; do
          echo "  - $AI_INSTALL_COMMAND"
        done
        "$TOOLS_DIR/agent-integrations.sh" install "${AI_INSTALL_TARGETS[@]}" || \
          echo "  (integration step failed — re-run: tools/agent-integrations.sh install --all-configured)"
        ;;
      *) echo "  Skipped. Later: tools/agent-integrations.sh install --all-configured" ;;
    esac
  elif [ "${AI_HAS_REAL_GRAPH}" = "yes" ]; then
    echo "  No detected or stale assistant harnesses need action."
  else
    echo "  Skipped. No real graph detected at graphify-out/graph.json."
    echo "  Generate a graph first with tools/graphify.sh, then run: tools/agent-integrations.sh install --all-configured"
  fi
else
  echo "  Non-interactive — skipped. Later: tools/agent-integrations.sh install --all-configured"
fi

# 7) Laptop-wide Bun/npm resolution policy — opt-in because this affects projects outside Axon.
# The two Axon UI trees carry the same 24-hour hold in tracked bunfig.toml files. This optional
# copy covers other projects on THIS machine and is deliberately hold-only: the Socket scanner
# cannot be global because Bun requires it to be installed in each project's dependencies.
# Never ask on a non-TTY; CI and piped installs must not hang on a machine-policy prompt.
if [ -t 0 ]; then
  echo
  read -r -p "Hold newly published npm/Bun packages for 24h on this machine? [y/N]: " BUN_HOLD_INPUT
  case "$BUN_HOLD_INPUT" in
    y|Y|yes|Yes|YES) write_bun_global_hold ;;
    *) echo "Skipped — run tools/install.sh again to add the laptop-wide hold." ;;
  esac
else
  echo
  echo "No TTY — skipped the optional laptop-wide Bun/npm hold."
fi

# 8) Capability selection — opt-in and skippable. Delegates entirely to
# tools/capability.sh (the single writer of machine.toml's capabilities line);
# install.sh only drives the prompt. enable is idempotent, so re-running the
# installer and re-picking an already-enabled capability is a harmless no-op.
# Non-TTY (piped, CI, ssh without -t): `read` would EOF non-zero and `set -e`
# would kill the install mid-flight — skip the interactive step entirely.
if [ ! -t 0 ]; then
  echo
  echo "No TTY — skipping capability selection. Enable later: tools/capability.sh enable <name>"
  echo
  echo "Setup done."
  exit 0
fi
echo
echo "Capabilities on this machine:"
"$TOOLS_DIR/capability.sh" list
echo
read -r -p "Enable capabilities now? [names (space-separated) / skip]: " CAP_INPUT
case "$CAP_INPUT" in
  ""|skip|Skip|SKIP)
    echo "Skipped — enable later with: tools/capability.sh enable <name>"
    ;;
  *)
    # capability.sh exits non-zero on an unknown name; guard it so one typo
    # doesn't abort the whole installer under `set -e` — report and move on.
    for cap in $CAP_INPUT; do
      echo
      "$TOOLS_DIR/capability.sh" enable "$cap" \
        || echo "install.sh: enable '$cap' failed — continuing." >&2
    done
    ;;
esac

# 9) Boot persistence for the autostart set. Nothing reconciled this before (#9): a capability
# could declare autostart = true, be enabled here, start fine, and be gone after the next reboot,
# with no warning at any point. Asked rather than assumed — installing persistence loads a launchd
# or systemd unit, which is a machine-level change an installer should not make silently. Declining
# is fine and leaves the state doctor already reports.
echo
PERSIST_OWED=""
while IFS=$'\t' read -r p_name p_state _; do
  [ -n "$p_name" ] || continue
  case "$p_state" in
    missing|stale) PERSIST_OWED="$PERSIST_OWED $p_name" ;;
  esac
done <<EOF
$("$TOOLS_DIR/service-runner.sh" persistence 2>/dev/null || true)
EOF

if [ -n "$PERSIST_OWED" ]; then
  echo "These enabled capabilities declare autostart but have no matching boot persistence:"
  for cap in $PERSIST_OWED; do echo "  $cap"; done
  echo "Without it they do not come back after a reboot."
  read -r -p "Install boot persistence for them now? [y/N]: " PERSIST_INPUT
  case "$PERSIST_INPUT" in
    y|Y|yes|Yes|YES)
      for cap in $PERSIST_OWED; do
        "$TOOLS_DIR/service-runner.sh" install-persistence "$cap" \
          || echo "install.sh: install-persistence '$cap' failed — continuing." >&2
      done
      ;;
    *)
      echo "Skipped — install later with: tools/service-runner.sh install-persistence <name>"
      echo "tools/doctor reports this for as long as it is outstanding."
      ;;
  esac
else
  echo "Boot persistence: nothing outstanding for the enabled autostart set."
fi

echo
echo "Setup done."

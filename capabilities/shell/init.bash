# Portable bash config — the bash sibling of init.zsh, for Linux/WSL nodes whose
# login shell is bash. No secrets, no machine-specific paths beyond existence-guarded
# tool detection. Sourced by ~/.bashrc after it exports SJEL_ROOT; see README.md.
#
# Kept deliberately parallel to init.zsh: same tool detection, same Bitwarden/mkcert/
# gh wiring, same capability-CLI PATH sweep. The two can't share a file (zsh vs bash
# syntax), but they must not drift — a change to one usually wants the same change here.
# bash 3.2-safe (CONTRIBUTING.md#portable-shell): no mapfile/readarray, no associative arrays.

# SJEL_ROOT is the one bootstrap fact ~/.bashrc must set before sourcing this (the bash
# mirror of ~/.zshrc's first line — CONTRIBUTING.md#dynamic-paths-and-current-facts's single sanctioned rc exception). Without it
# nothing below can resolve, so bail softly rather than guessing a hardcoded path.
if [ -z "${SJEL_ROOT:-}" ]; then
  echo "init.bash: SJEL_ROOT is unset — add it to ~/.bashrc (see capabilities/shell/README.md)" >&2
  return 0 2>/dev/null || exit 0
fi

# --- tool completions / PATH (existence-guarded, safe if not installed) ---
[ -s "$HOME/.bun/_bun" ] && source "$HOME/.bun/_bun"
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"

[ -d "$HOME/.lmstudio/bin" ] && export PATH="$PATH:$HOME/.lmstudio/bin"

[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# --- capability CLIs on PATH ---
# Same convention as init.zsh: a capability that ships a command names it after itself
# (capabilities/<name>/<name>), and that file is the entry point. This loop puts every
# directory holding such a file on PATH, so a new capability CLI is callable from the next
# shell with nothing to register by hand. Appended, never prepended — a capability CLI
# must not shadow a system binary. (bash has no :t/%/ expansion, so basename + ${%/} do it.)
for _cap in "$SJEL_ROOT"/capabilities/*/; do
  _name="$(basename "$_cap")"
  [ -x "$_cap$_name" ] && PATH="$PATH:${_cap%/}"
done
unset _cap _name
export PATH

# --- OpenCode (privacy defaults; not Claude Code, so shell env not settings.json) ---
export OPENCODE_DISABLE_SHARE=true
export OPENCODE_DISABLE_MODELS_FETCH=true
# Claude Code env (agent teams, MCP CLI, telemetry-off, auto-compact) is NOT exported here
# — it lives in tools/templates/claude-code/settings.base.json (CONTRIBUTING.md#one-manifest-per-concern), same as init.zsh.
# Deployment-specific backend config (Vertex, model IDs) is overlay shell config — see README.

# --- Bitwarden SSH agent ---
# Route ssh through the Bitwarden desktop app's agent — keys live in the vault, not on
# disk. Socket resolution is shared with tools/backup.sh via the lib.
[ -f "$SJEL_ROOT/tools/lib/bw-agent.sh" ] && source "$SJEL_ROOT/tools/lib/bw-agent.sh"

# --- Bitwarden CLI session ---
# Pick up a session cached by tools/bw-unlock so a new shell starts from the existing
# unlock. Deliberately does NOT validate it (see init.zsh for the why).
if [ -r "${XDG_CACHE_HOME:-$HOME/.cache}/axon/bw-session" ]; then
  export BW_SESSION="$(<"${XDG_CACHE_HOME:-$HOME/.cache}/axon/bw-session")"
fi

# Re-unlock and adopt the key in THIS shell (a subprocess can't export into its parent).
bwu() {
  local _s
  _s="$("$SJEL_ROOT/tools/bw-unlock")" || return 1
  export BW_SESSION="$_s"
}

# --- local TLS trust (mkcert) ---
# Let Node-based CLIs trust the local mkcert CA so they can reach self-hosted HTTPS
# (Vaultwarden). Node ships its own CA store. No-op where mkcert isn't installed.
if command -v mkcert >/dev/null 2>&1; then
  _mkcert_ca="$(mkcert -CAROOT)/rootCA.pem"
  [ -f "$_mkcert_ca" ] && export NODE_EXTRA_CA_CERTS="$_mkcert_ca"
  unset _mkcert_ca
fi

# --- github token (delegates to gh's own credential storage, not a stored secret) ---
# No launchctl mirror here — that's macOS-only; on Linux/WSL the export is enough.
if command -v gh >/dev/null 2>&1; then
  export GITHUB_TOKEN="$(gh auth token 2>/dev/null)"
fi

# --- WSL integration (only defined under WSL; no-op on bare Linux) ---
if [ -n "${WSL_DISTRO_NAME:-}" ]; then
  # Open Linux URLs in the Windows default browser.
  command -v wslview >/dev/null 2>&1 && export BROWSER=wslview

  # clipimg [filename] — save the Windows clipboard image into WSL, echo its path.
  clipimg() {
    local name="${1:-clipboard-$(date +%s).png}" out winpath result
    out="/tmp/$name"
    winpath="$(wslpath -w "$out")"
    result=$(powershell.exe -NoProfile -command "
      Add-Type -AssemblyName System.Windows.Forms
      \$img = [System.Windows.Forms.Clipboard]::GetImage()
      if (\$img) { \$img.Save('$winpath'); Write-Output 'ok' }
      else { Write-Output 'empty' }
    " 2>&1 | tr -d '\r')
    [ "$result" = "ok" ] && echo "$out" || { echo "No image in clipboard" >&2; return 1; }
  }
fi

# --- interactive-only: prompt, aliases, completion ---
case $- in *i*)
  # Aliases (portable).
  alias ll='ls -alF'
  alias la='ls -A'
  alias l='ls -CF'
  alias ..='cd ..'
  alias gs='git status'
  alias dev='cd "${SJEL_ROOT%/*}"'   # parent of Axon, derived — no hardcoded path

  # History.
  HISTSIZE=10000
  HISTCONTROL=ignoreboth:erasedups
  shopt -s histappend 2>/dev/null || true

  # Prompt: Starship if installed, else a plain fallback.
  if command -v starship >/dev/null 2>&1; then
    eval "$(starship init bash)"
  else
    PS1='\[\e[0;37m\]\w\[\e[0m\] \[\e[0;34m\]❯\[\e[0m\] '
  fi

  # System bash completion.
  [ -f /usr/share/bash-completion/bash_completion ] && source /usr/share/bash-completion/bash_completion
  ;;
esac

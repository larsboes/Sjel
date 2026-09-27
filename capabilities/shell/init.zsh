# Portable zsh config — no secrets, no machine-specific paths beyond
# existence-guarded tool detection. Sourced by ~/.zshrc; see README.md.

# --- terminal / theme ---
[[ "$TERM" == "xterm-ghostty" ]] && export TERM=xterm-256color

autoload -U colors && colors
setopt prompt_subst
autoload -Uz vcs_info
zstyle ':vcs_info:*' enable git
zstyle ':vcs_info:git:*' formats ' %F{magenta}%b%f'
zstyle ':vcs_info:*' check-for-changes true
function precmd() {
    vcs_info
}
PROMPT='%F{white}%~%f${vcs_info_msg_0_} %F{blue}❯%f '
RPROMPT='%F{8}%D{%H:%M:%S}%f'
export CLICOLOR=1
export LSCOLORS=gxfxexdxcxegedabagacad

[[ -f /opt/homebrew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh ]] && \
  source /opt/homebrew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh
ZSH_HIGHLIGHT_STYLES[path]=
ZSH_HIGHLIGHT_STYLES[path_pathseparator]=fg=black,bold
ZSH_HIGHLIGHT_STYLES[path_prefix]=

# --- tool completions / PATH (existence-guarded, safe if not installed) ---
[[ -s "$HOME/.bun/_bun" ]] && source "$HOME/.bun/_bun"
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"

[[ "$TERM_PROGRAM" == "kiro" ]] && . "$(kiro --locate-shell-integration-path zsh)"

if [[ -d "$HOME/.docker/completions" ]]; then
  fpath=("$HOME/.docker/completions" $fpath)
  autoload -Uz compinit
  compinit
fi

[[ -d "$HOME/.lmstudio/bin" ]] && export PATH="$PATH:$HOME/.lmstudio/bin"

# --- capability CLIs on PATH ---
# Convention: a capability that ships a command names it after itself
# (capabilities/<name>/<name>), and that one file is the entry point. This loop puts
# every directory holding such a file on PATH, so a new capability with a CLI is
# callable from the next shell with no line added here and nothing to keep in sync.
#
# Named after the capability rather than "any executable under capabilities/" for a
# reason: printing/printctl.py is an implementation the 3d-printing Pack drives through
# `uv run`, not a command to call directly, and the name test is what tells those apart.
# Appended, never prepended — a capability CLI must not shadow a system binary.
#
# BOTH roots, because a capability is not less real for living in the overlay. This
# swept only $SJEL_ROOT until 2026-08-30, so an overlay capability shipping a CLI could
# never be called by name: `ytalbum` and `interior` were invisible to the shell by
# construction, and the only way to run one was to type its absolute path. The overlay
# is where a capability goes when it is inseparable from what it is pointed at
# (README.md#placement-guide), which is a statement about privacy, not about whether it
# has a command.
#
# Public first, so a name present in both resolves to the public one. That ordering
# should never matter — `tools/doctor` refuses one capability name declared in two roots
# — and it is written down here as the tie-break rather than left to glob order.
for _root in "$SJEL_ROOT" "$SJEL_PERSONAL_ROOT"; do
  [[ -n "$_root" && -d "$_root/capabilities" ]] || continue
  # (N) is null_glob for this glob only: a root whose capabilities/ is empty must
  # contribute nothing, not print "no matches found" on every shell start.
  for _cap in "$_root"/capabilities/*/(N); do
    # The glob yields a trailing slash. :t reads through it to the capability name, and
    # %/ trims it off for the PATH entry — pure parameter expansion, so the whole sweep
    # costs no forks at shell startup. (Not :h, which on a trailing-slash path strips the
    # last component too and would put capabilities/ itself on PATH.)
    [[ -x "$_cap${_cap:t}" ]] && PATH="$PATH:${_cap%/}"
  done
done
unset _cap _root
export PATH

# --- Claude Code ---
# Env vars (agent teams, MCP CLI, telemetry-off, auto-compact) are NOT exported here.
# They live in tools/templates/claude-code/settings.base.json's `env` block — the single
# home claude-code-config.ts deploys to ~/.claude/settings.json on every machine (README.md#one-manifest-per-concern:
# one concern, one manifest). settings.json is the right scope: these vars are Claude-Code-
# only, so a shell-wide export bought nothing. Deployment-specific backend config (Vertex,
# model IDs) is overlay shell config, never shipped public — see README's overlay note.
#
# No claude() wrapper here on purpose. It used to append a system prompt from
# PAI_SYSTEM_PROMPT.md, which LifeOS 7.x renamed to LIFEOS_SYSTEM_PROMPT.md — the
# wrapper's own [[ -f ]] guard then failed silently, so it had been a no-op since
# the 7.x upgrade. LifeOS' current doctrine is the split it was fighting: plain
# `claude` stays vanilla, and the constitutional layer is opted into with the
# `lifeos` alias LifeOS' own installer wires (LifeOS INSTALL.md:96-102). Adding
# the flag back here would load ~20k tokens into every subagent and cmux spawn.
alias clauded='claude --dangerously-skip-permissions'

# --- OpenCode (privacy defaults; not Claude Code, so shell env not settings.json) ---
export OPENCODE_DISABLE_SHARE=true
export OPENCODE_DISABLE_MODELS_FETCH=true

pi() {
  [[ -x "$HOME/.pi/bin/sync-claude-md.sh" ]] && "$HOME/.pi/bin/sync-claude-md.sh" 2>/dev/null
  command pi "$@"
}

# --- misc aliases ---
alias python=python3
alias chrome-debug='"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --remote-debugging-port=9222 --user-data-dir=/tmp/chrome-cdp-profile &'

# --- Bitwarden SSH agent ---
# Route ssh through the Bitwarden desktop app's agent — keys live in the vault,
# not on disk. Socket resolution is shared with tools/backup.sh via the lib.
[[ -n "$SJEL_ROOT" && -f "$SJEL_ROOT/tools/lib/bw-agent.sh" ]] && source "$SJEL_ROOT/tools/lib/bw-agent.sh"

# --- Bitwarden CLI session ---
# Pick up a session cached by tools/bw-unlock so a new shell starts from the existing
# unlock instead of a locked vault. Deliberately does NOT validate it: `bw status` is a
# Node startup on every shell, and a stale key costs one failed command and a `bwu`,
# while checking costs a second of latency on every terminal that never touches bw.
[[ -r "${XDG_CACHE_HOME:-$HOME/.cache}/axon/bw-session" ]] \
  && export BW_SESSION="$(<"${XDG_CACHE_HOME:-$HOME/.cache}/axon/bw-session")"

# Re-unlock and adopt the key in THIS shell. A subprocess cannot export into its parent,
# so this stays a function rather than living inside bw-unlock.
bwu() {
  local _s
  _s="$("$SJEL_ROOT/tools/bw-unlock")" || return 1
  export BW_SESSION="$_s"
}

# --- local TLS trust (mkcert) ---
# Let Node-based CLIs (bitwarden-cli, etc.) trust the local mkcert CA so they
# can talk to self-hosted HTTPS like Vaultwarden — Node ships its own CA store
# and ignores the macOS system trust. Guarded: no-op where mkcert isn't installed.
if command -v mkcert >/dev/null 2>&1; then
  _mkcert_ca="$(mkcert -CAROOT)/rootCA.pem"
  [[ -f "$_mkcert_ca" ]] && export NODE_EXTRA_CA_CERTS="$_mkcert_ca"
  unset _mkcert_ca
fi

# --- github ---
# Deliberately no GITHUB_TOKEN here. `gh` authenticates from its own keyring and git
# from the osxkeychain helper (which never reads GITHUB_TOKEN), so nothing needs an
# ambient copy — while a non-empty one makes `gh auth login` refuse to run at all.
# This used to export it and `launchctl setenv` it for Pulse's GitHub check, which
# reads process.env.GITHUB_TOKEN but is a no-op unless LIFEOS_PULSE_REPOS is set.
# If that check ever wants auth, it should shell out to `gh auth token` itself rather
# than have every launchd-started process on the machine carry the value in plaintext.

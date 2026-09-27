# shell

Portable shell config — theme, completions, aliases, functions. Nothing here is
personal or secret; it's safe in the public repo and works on any machine. Two
sibling files, one per shell family:

- **zsh** (macOS default): `~/.zshrc` sources `init.zsh`, then
  `axon-overlay/config/shell/*.zsh` for whatever's actually machine-specific.
- **bash** (Linux/WSL nodes): `~/.bashrc` exports `SJEL_ROOT` and sources
  `init.bash`, then `axon-overlay/config/shell/*.bash` for the machine-specific
  bits. The `SJEL_ROOT` line is the bash mirror of `~/.zshrc`'s first line —
  CONTRIBUTING.md#dynamic-paths-and-current-facts's one sanctioned rc-file bootstrap exception. `install.sh` does not
  inject either line; wiring the login rc file stays a manual, per-machine step
  (same as zsh).

**Verdict:** build (this is Axon-maintained config, not adopted from anywhere).

## Layout

| File | Holds |
|---|---|
| `init.zsh` | Everything for zsh — theme, syntax highlighting, tool completions (bun/docker/kiro), capability CLIs on PATH, aliases, the `pi` wrapper, `chrome-debug`, Bitwarden SSH-agent + session pickup, mkcert local TLS trust, GitHub token export, OpenCode privacy env |
| `init.bash` | The bash sibling for Linux/WSL — the same tool detection, capability-CLI PATH sweep, Bitwarden/mkcert/gh wiring and OpenCode env, plus WSL-only helpers (`clipimg`, `wslview` browser). No macOS-only bits (homebrew, `launchctl`, `chrome-debug`) |

Kept deliberately parallel — the two can't share a file (zsh vs bash syntax),
so a change to one usually wants the same change in the other. Each stays a
single file on purpose (CONTRIBUTING.md#documentation-stays-owned-and-current, docs minimalism) — split only once
it's actually too big to navigate.

## Why this shape: Claude Code and AI backend env

Claude-Code-only env vars (experimental agent teams, MCP CLI, telemetry-off,
auto-compact window) are **not** exported from these shell files. They live in
`tools/templates/claude-code/settings.base.json`'s `env` block — the single home
`tools/claude-code-config` deploys into `~/.claude/settings.json` on every
machine (CONTRIBUTING.md#one-manifest-per-concern: one concern, one manifest). `settings.json` is the correct
scope: those vars only affect Claude Code, so a shell-wide export bought nothing
and duplicated the fact in two places.

Deployment-specific *backend* config — `CLAUDE_CODE_USE_VERTEX`,
`CLOUD_ML_REGION`, `ANTHROPIC_VERTEX_BASE_URL`, and the backend-format model IDs
(`ANTHROPIC_DEFAULT_*_MODEL`) — is **not** shipped in this public repo. It is a
fact about one deployment (a machine wired to Vertex vs. Anthropic direct), so it
belongs in overlay shell config: `axon-overlay/config/shell/*.zsh` (or `*.bash`
on a bash node), sourced right after these files. A personal machine on the
Anthropic API simply doesn't set them and gets the harness defaults. OpenCode's
privacy env (`OPENCODE_DISABLE_SHARE`, `OPENCODE_DISABLE_MODELS_FETCH`) *is*
backend-agnostic, so it stays here as a public default.

## Gotchas

- `zsh-syntax-highlighting` is sourced early in `init.zsh`, matching the original
  `.zshrc` order. Its own docs recommend sourcing it last; that's a pre-existing
  quirk carried over during the split, not something new — worth fixing
  separately if highlighting ever misbehaves.
- Tool completion/PATH lines (bun, docker, LM Studio, kiro) are all
  existence-guarded so both files are safe to source on a machine that doesn't
  have those tools installed — they just no-op.
- `init.bash`'s WSL helpers (`clipimg`, `BROWSER=wslview`) are defined only when
  `$WSL_DISTRO_NAME` is set, so the file is also safe on bare (non-WSL) Linux.

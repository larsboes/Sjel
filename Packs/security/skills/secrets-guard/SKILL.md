---
name: secrets-guard
description: Safe handling of secrets and credentials in a Claude Code session — how to use API keys, tokens, and .env / .secrets values for a task without ever pulling them into the model context, working WITH Axon's managed-settings deny policy instead of around it. Use when a task needs a credential (an authenticated API call, a deploy, a login), before reading a .env / .secrets / credentials / .pem / .key file, or when a command was blocked by the security policy and the safe equivalent is needed. Do not use for pi sessions (the secrets-guard extension enforces this there) or for creating/storing a new secret (that is tools/setup-secret.sh, run by the human).
allowed-tools: Bash
---

# secrets-guard (Claude Code)

Credentials must do their job **without their values ever entering the conversation**. Once a
secret is in context it can leak into session files, logs, or provider requests. This skill is the
workflow; enforcement is separate and already active:

- **Managed policy** — `tools/templates/claude-code/managed-settings.json` (deployed to
  `/etc/claude-code/managed-settings.json`) denies reads of `**/*.env`, `**/*.pem`, `~/.ssh`,
  `~/.aws`, `git-credentials`, `printenv`, `*_TOKEN*`, etc., and denies ~20 secret env vars to the
  sandbox. You cannot relax it from a repo. If a command here is blocked, that is the floor working.
- **pi** — the same protection is enforced as a tool-call extension (`Packs/security/extensions/
  secrets-guard.ts`); this skill is the Claude Code counterpart for the parts a policy can't teach.

## The one rule

Never `cat` / `read` / `grep` / `echo` a secret. Feed it to the command that needs it via the
environment, so only the child process sees the value.

## Patterns

**Use a credential in a command** — source the env file, then run; sourcing loads variables
without printing them:

```bash
source .env && curl -sS -H "Authorization: Bearer $SOME_API_KEY" https://api.example.com/...
```

The value is in the subshell's env, never in your output. Do **not** `echo "$SOME_API_KEY"` to
"check" it — that defeats the point and is blocked.

**Discover what a config needs** without reading values — read the *committed template*, not the
real file (templates are safe; real `.env` is denied):

```bash
cat .env.example        # or config.example.toml, etc. — placeholders only
```

**A secret from Bitwarden/Vaultwarden** — unlock, then let the command consume it; never `bw get`
into the transcript:

```bash
bw unlock            # sets a session; the token is not a secret value
# then source a script that itself calls `bw get` and exports into env — you never see the value
```

## Creating or rotating a secret — stop and hand off

Never generate, paste, or write a secret value through an agent turn (CONTRIBUTING.md#secrets). Ask the
human to run, in their own terminal:

```bash
tools/setup-secret.sh <capability> <slug> <ENV_VAR>
```

It writes the value to Vaultwarden + the capability's git-ignored `.env` + a pointer doc, and never
prints it.

## Data class

`c2` (Others) and `c3` (Secret) material never reaches a cloud model. That one is enforced in code
(CONTRIBUTING.md#data-classes): the derivative builder refuses to produce a preview, the tier check
refuses the dispatch, dispatch re-reads the row's current class, and the table's CHECK constraint
refuses the row.

`c3` stays out of **local** prompts too, and that is gated now:
`content_item::local_prompt_allowed` answers `false` for `c3` and for any class it does not
recognize, and both of Axon's prompt-builders over stored text ask it first (`digest.rs`,
`media.rs`). Axon's own paths will stop you. Yours will not — if a task would put `c3` content in
front of any model, local included, don't; route it around the model or hand off.

## When blocked

A denied read/command is the managed policy doing its job — switch to the env-sourcing pattern
above rather than trying to work around the block. If a genuinely-needed path is over-blocked, that
is a policy change (`tools/claude-code-config.ts --managed`), made deliberately by the human, not a
reason to bypass it.

# Security Pack

Guard secrets and credentials from agent visibility across harnesses.

Codex's opt-in native filesystem permissions and deletion-approval baseline lives in
[`tools/templates/codex`](../../tools/templates/codex/README.md). Its documented limits
differ from Pi's tool-call interception and `vault_exec`; it is not a Pi extension port.

## What's here

| Artifact | Harness | What it does |
|----------|---------|-------------|
| `extensions/secrets-guard.ts` | **pi** | Intercepts `read`, `edit`, `ls`, `grep`, `find`, `bash` tool calls to block reading/searching .env, .secrets, credentials, .pem/.key, .netrc, git-credentials, kubeconfig, ssh keys, and Bitwarden items. Registers `vault_exec` and `vault_keys` custom tools for safe credential usage. |
| `extensions/command-policy/` | **pi** | Classifies shell commands and direct writes and adds machine-local autonomy for routine work. Consequential exceptions retain approval cards. The runtime profile still controls local builds (P1). The shared baseline lives in [`docs/plans/agent.md` §3](../../docs/plans/agent.md); Pi-specific grants and limits are below. Not a sandbox. |
| `extensions/inference-keys/` | **pi** | Supplies provider API keys from the macOS login keychain, falling back to the Bitwarden/Vaultwarden vault, instead of from environment variables or `auth.json`. The key is handed to pi by a command and never enters the conversation. The pi entry stays in TypeScript; its credential gateway, unlock flow, and keychain migration are implemented in the shared Rust CLI. See its [README](extensions/inference-keys/README.md). |
| `skills/secrets-guard/` | **Claude Code** | Skill teaching the safe credential workflow (env-sourcing, never read/echo secrets, hand off secret creation to `tools/setup-secret.sh`) — the counterpart to Sjel's deny policy in the user-level Claude Code settings (`tools/templates/claude-code/settings.base.json`, applied by `sjel claude`). Deploy with `tools/packs.sh link security`. |
| `skills/threat-model/` | **Claude Code** | Maps which assets hold which data class, walks one compromise at a time (exposure, reach, detection, response) and keeps a risk register. All output stays in `<overlay>/data/security/threat-model/`. |

## Pi session permissions

Pi approves routine direct file edits and recognized shell file operations inside the active Git
checkout or the session scratch directory. The checkout is fixed at session start, including
when Pi starts in a subdirectory. Outside a checkout, only scratch is automatic. Writes to
other checkouts and vault directories outside the active checkout require approval.

The overlay is protected even when it contains a checkout. Credentials, Git internals, agent
instructions, hooks, harness configuration and other protected paths never inherit a project grant.
The optional `protected_roots` array in `<overlay>/config/command-policy.json` can only add
restrictions. Invalid restrictions pause project and scratch grants. Legacy
`write_roots`, `markdown_write_roots` and `disposable_roots` no longer grant access.

### Routine work

- Local dependency changes qualify for npm, pnpm, Yarn, Bun, uv, Cargo and Go when the project
  manifest and destination resolve within scope. Global installs, unverified destination
  overrides and downloaded executors require approval. Package scripts can still execute code.
- Standalone local branch creation qualifies at the checkout root with its own `.git` directory.
  Standalone `git worktree add` qualifies for a new, unprotected destination in scope.
  `git worktree list` is routine. Git overrides, publishing and destructive operations retain
  the baseline decision.
- Recursive `rm` qualifies only for literal paths in the private session scratch directory.
  Shared build trees and source directories never qualify. Empty scratch is removed at shutdown;
  nonempty scratch remains, but a new session creates a new directory.
- A positive PID qualifies for automatic `kill` only while it is a live descendant of Pi.
  Process groups, unrelated processes and container mutations still require approval.

### Controls and limits

`/permissions` shows the checkout and scratch paths. `/permissions off` clears exact-call
grants and pauses automatic project and scratch grants until reload or session change.
There is no command to add another directory to the session.

Existing files must be read before an automatic direct edit. A changed file must be reread.
This metadata check is not atomic and does not cover shell writes. The Git checkpoint reports
changes to protected paths without starting another model turn.

Approval cards wait for an explicit decision. **Allow once** applies only to the current call.
Ask rules also accept an exact-call grant bound to the working directory and session. Block rules
never gain a session grant. Cancellation leaves no pending action. Without a UI, Ask and Block
refuse immediately.

The classifier is not a sandbox. Unknown programs, arbitrary scripts and package hooks can
have effects it cannot inspect. The separate secrets guard, runtime build policy and launcher
containment remain in force. An approval does not authorize unrelated work.

Verification: `bun test tools/command-policy*.test.ts tools/pack-extensions.test.ts` covers
the shared baseline, project scope, lifecycle and installed Pi types.

## The problem

Coding agents read files and run commands the same way you do. When you tell an agent
"check the API config" it runs `cat .env` — and the LLM sees your API keys, tokens, and
passwords. They become part of the conversation context, and from there they can leak
into session files, logs, or provider requests.

This pack prevents that by intercepting at the tool-call boundary — **before** the value
reaches the agent's context.

## How it works

### Blocked operations

| Tool | Pattern | Example |
|------|---------|---------|
| `read` | `*.env`, `*.secrets`, `*credentials*`, `*.pem`, `*.key`, `.netrc`, `git-credentials`, `kubeconfig`, `~/.ssh/id_*` | `read .env.local` → blocked |
| `ls`/`grep`/`find` | a secret path, or a pattern/glob targeting secret files | `find . -name '*.env'` → blocked |
| `bash` | `cat`/`grep`/`head`/`tail` on secret files | `cat .env` → blocked |
| `bash` | `bw get`/`list`/`sync`/`export` | `bw get item "API"` → blocked |
| `bash` | `echo $TOKEN` or `printenv` | `echo $GOOGLE_CLIENT_SECRET` → blocked |
| `edit`/`write` | `*.env`, `*.secrets`, also through a symlinked path | `edit .env` → blocked |

### Allowed operations

| Operation | Example | Why safe |
|-----------|---------|----------|
| `bash` with `source` | `source .env && curl ...` | Sourced into env, stdout doesn't contain values |
| `bash` with `bw unlock` | `bw unlock` | Unlocks vault, doesn't read items |
| `export VAR=...` | `export API_KEY=....` | Setting env vars is not reading them |

### Providers trusted with raw secrets

By default no model sees a raw secret. The overlay may name providers that can, in
`config/secrets-guard.toml`:

```toml
raw_secret_providers = ["provider-name"]
```

A listed provider gets raw output only in an ephemeral (`--no-session`) session, where tool
results are not written to a session file. Which provider is trusted is a machine decision,
so the list never lives in Sjel.

### Custom tools

| Tool | What it does |
|------|-------------|
| `vault_exec` | Runs a shell command with an env file sourced. The extension reads the file directly — the LLM never constructs the command with secret values and never sees them in the output. Output is sanitized for leaked secrets. |
| `vault_keys` | Lists variable names (not values) from an env file. Use to discover what secrets are available. |

## Installation (pi)

```bash
# Symlink or copy into pi's extension directory
mkdir -p ~/.pi/agent/extensions
ln -s "$(pwd)/Packs/security/extensions/secrets-guard.ts" ~/.pi/agent/extensions/
```

Or reference from `settings.json`:

```json
{
  "extensions": ["/path/to/Sjel/Packs/security/extensions/secrets-guard.ts"]
}
```

## Usage in an agent session

### BAD — exposes secrets
```
read .env
cat .env
grep API_KEY .env
bw get item "Google OAuth"
```

These are all **blocked** by the guard.

### GOOD — secrets never reach context

```
# Source env then run command (source doesn't print values)
source .env && curl -H "Authorization: Bearer $GOOGLE_CLIENT_SECRET" ...

# Or use vault_exec (tool, extension reads env file directly)
vault_exec({cmd: "curl -s https://api.example.com", env_file: ".env"})

# Check what variables are available (shows keys only)
vault_keys({env_file: ".env"})
```

## Future

- **opencode adapter** — if harness supports tool-call interception, same pattern
- **Vaultwarden-aware vault_read** — read specific secrets from Vaultwarden via
  the API without exposing values to the LLM

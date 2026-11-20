# Security Pack

Guard secrets and credentials from agent visibility across harnesses.

Codex's opt-in native filesystem permissions and deletion-approval baseline lives in
[`tools/templates/codex`](../../tools/templates/codex/README.md). Its documented limits
differ from Pi's tool-call interception and `vault_exec`; it is not a Pi extension port.

## What's here

| Artifact | Harness | What it does |
|----------|---------|-------------|
| `extensions/secrets-guard.ts` | **pi** | Intercepts `read`, `edit`, `ls`, `grep`, `find`, `bash` tool calls to block reading/searching .env, .secrets, credentials, .pem/.key, .netrc, git-credentials, kubeconfig, ssh keys, and Bitwarden items. Registers `vault_exec` and `vault_keys` custom tools for safe credential usage. |
| `extensions/command-policy/` | **pi** | Classifies shell commands and direct writes, adds machine-local autonomy for routine work, and supplies live permission context. Consequential exceptions retain approval cards. The runtime profile still controls local builds (P1). The shared baseline lives in [`docs/plans/agent.md` §3](../../docs/plans/agent.md); Pi-specific grants and limits are below. Not a sandbox. |
| `extensions/inference-keys/` | **pi** | Supplies provider API keys from the macOS login keychain, falling back to the Bitwarden/Vaultwarden vault, instead of from environment variables or `auth.json`. The key is handed to pi by a command and never enters the conversation. The pi entry stays in TypeScript; its credential gateway, unlock flow, and keychain migration are implemented in the shared Rust CLI. See its [README](extensions/inference-keys/README.md). |
| `skills/secrets-guard/` | **Claude Code** | Skill teaching the safe credential workflow (env-sourcing, never read/echo secrets, hand off secret creation to `tools/setup-secret.sh`) — the counterpart to Sjel's deny policy in the user-level Claude Code settings (`tools/templates/claude-code/settings.base.json`, applied by `sjel claude`). Deploy with `tools/packs.sh link security`. |
| `skills/threat-model/` | **Claude Code** | Maps which assets hold which data class, walks one compromise at a time (exposure, reach, detection, response) and keeps a risk register. All output stays in `<overlay>/data/security/threat-model/`. |

## Pi session permissions

Machine-local grants live in `<overlay>/config/command-policy.json`, resolved from
`SJEL_OVERLAY_ROOT` with `SJEL_PERSONAL_ROOT` as fallback. Project settings cannot grant access.
The extension loads and pins canonical roots at session start and reload:

```json
{
  "markdown_write_roots": [],
  "write_roots": ["/absolute/path/to/development", "/absolute/path/to/knowledge"],
  "disposable_roots": [],
  "protected_roots": ["/absolute/path/to/private-overlay"]
}
```

Each root must be an existing absolute directory other than home or filesystem root.
Missing config grants nothing. Invalid or unreadable config disables all persistent grants
and emits a warning; unknown keys are invalid. Symlink escapes, dangling links and replaced
root paths do not gain a grant. The optional arrays default to empty; `markdown_write_roots`
is required for compatibility with Markdown-only configurations.

### Routine work

`write_roots` admits recognized file operations, direct edits, ordinary moves, redirects and
compound commands across the named directories. Routine application configuration qualifies;
credentials, Git internals, agent instructions, harness configuration, extensions, hooks,
prompts and `protected_roots` do not. Protected effects cannot inherit a legacy file-scope grant.
Writes outside approved roots, the initial working directory and temporary directories remain gated.

Project-local dependency restores, additions and upgrades run without approval when the
working directory has the corresponding `package.json`, `pyproject.toml`, `Cargo.toml` or
`go.mod`. Supported managers are npm, pnpm, Yarn, Bun, uv, Cargo and Go. Explicit
`npm --prefix`, `uv --project` and `cargo --manifest-path` selectors also qualify when a single
literal destination and its manifest resolve inside an approved, unprotected root. Split and
`--option=value` forms work before or after the operation. Global installs, configuration
overrides, other destination flags, dynamic or duplicate selectors, unverified flags, linking
and downloaded executors (`npx`, `bunx`, `uvx`) retain approval. A single static `cd <directory> && ...` is supported;
other directory-changing control flow requires an explicit working directory. Package scripts
and configuration can execute arbitrary code: this grant accepts dependency execution risk,
not containment of its effects.

Recursive `rm` qualifies only for literal paths under `disposable_roots` or the private scratch
directory created for the current extension lifetime. Configured disposable roots must lie
strictly below a write root and cannot overlap protected roots or name live build trees.
The machine owner must identify genuinely disposable output; a directory name is not proof.
Shared build trees, source deletion, globs, dynamic targets and `find -delete` stay gated.
Empty scratch is removed on shutdown; nonempty scratch remains available. Reload creates new scratch.

`kill` of positive PIDs qualifies only while each PID is a live descendant of this Pi process.
Process groups, unrelated or reparented processes, `pkill`, `killall` and container mutations
remain gated. Privilege escalation, destructive history changes, secrets, publishing and
recognized network writes retain their existing guards. The separate secrets guard and
runtime-profile build policy remain active.

### Context and checkpoints

Every model request receives fresh extension-generated context from the active grants,
including after compaction. It states the allowed roots, dependency execution risk, disposable
output and remaining boundaries. Context is advisory and never grants permission. Execution
permission does not authorize new tasks or waive repository and vault rules.

Direct writes to existing files require a successful current read. A changed inode, size or
timestamp requires a reread and reconciliation, without an approval card. Successful direct
writes update the observation. These metadata checks are best-effort, not atomic file locks;
shell writes and changes between preflight and execution are not covered.
A Git checkpoint records pre-existing dirty paths and warns after a run changes protected
paths. An unavailable checkpoint is reported rather than treated as a clean tree. Ignored
files and external effects are outside that checkpoint.

The classifier is not a sandbox. Arbitrary scripts, package hooks and unrecognized programs
can have effects the filter cannot inspect. `tools/pi` retains the existing provider-specific
containment; its trusted profile denies selected private paths, not all writes outside these
roots or all network egress. Neither context nor postflight checks can undo a disclosure.

### Controls and exceptional approvals

`/permissions` shows active roots. `/permissions off` clears
session grants and suppresses all persistent grants until reload or session change.
`/permissions auto <directory>` retains the narrower legacy grant: direct writes, simple
`cp -n` (optionally `-p`) to an explicit file, `mkdir` (optionally `-p`) and flag-free `touch`.
It grants no dependency, publishing, process-control or recursive-cleanup permission.

Only consequential exceptions need approval cards. Cards wait for explicit approval, denial
or cancellation, without an idle timeout, deferred queue or retry tokens. **Allow once** applies
only to the current call. Ask rules also support an exact-call grant bound to the working
directory and session; Block rules never gain a session grant. Cancellation executes nothing
and stores no action. Without a UI, Ask and Block refuse immediately. The agent continues
independent approved work and never treats silence as consent.
Session grants and observations clear on reload or session change. Persistent roots load again
from machine config. Original tool calls remain in the session transcript.

Verification: `bun test tools/command-policy*.test.ts tools/pack-extensions.test.ts` covers
the shared baseline, relaxed policy, lifecycle and installed Pi types.

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

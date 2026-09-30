# Security Pack

Guard secrets and credentials from agent visibility across harnesses.

## What's here

| Artifact | Harness | What it does |
|----------|---------|-------------|
| `extensions/secrets-guard.ts` | **pi** | Intercepts `read`, `edit`, `ls`, `grep`, `find`, `bash` tool calls to block reading/searching .env, .secrets, credentials, .pem/.key, .netrc, git-credentials, kubeconfig, ssh keys, and Bitwarden items. Registers `vault_exec` and `vault_keys` custom tools for safe credential usage. |
| `skills/secrets-guard/` | **Claude Code** | Skill teaching the safe credential workflow (env-sourcing, never read/echo secrets, hand off secret creation to `tools/setup-secret.sh`) — the counterpart to the managed-settings deny policy. Deploy with `tools/packs.sh link security`. |

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
| `edit` | `*.env`, `*.secrets` | `edit .env` → blocked |

### Allowed operations

| Operation | Example | Why safe |
|-----------|---------|----------|
| `bash` with `source` | `source .env && curl ...` | Sourced into env, stdout doesn't contain values |
| `bash` with `bw unlock` | `bw unlock` | Unlocks vault, doesn't read items |
| `write` to `.env` | `write .env` | Writing secrets is legitimate setup |
| `export VAR=...` | `export API_KEY=....` | Setting env vars is not reading them |

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

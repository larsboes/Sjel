# inference-keys

Gives pi's API-key providers their credentials from the macOS login keychain,
and from the Bitwarden/Vaultwarden vault for any key that has not been migrated
out of it yet — instead of from environment variables or `auth.json`.

Nothing is exported into the environment, no key is written into this repo, and
no key is ever printed by the extension. When pi needs to talk to a provider it
runs `tools/inference-keys key <slug>`, a Rust CLI that reads the key from the login
keychain, or falls back to the vault, and hands it back on stdout.

Lives in this Pack at `Packs/security/extensions/inference-keys/`, and is registered in
`~/.pi/agent/settings.json` by `tools/packs-pi deploy security`. Nothing is copied into
`~/.pi`: pi loads `index.ts` where it sits, which is why the helper is resolved from this
file's own URL rather than from an install path, and why `keychain-migrate.sh` defaults to
the directory it is in.

```
index.ts                         pi integration: catalog, registration, discovery, commands
tools/sjel-cli/src/inference_keys.rs  gateway, unlock, status, and keychain migration
tools/inference-keys                  Rust CLI launcher
keychain-migrate.sh               stable migration entry point
```

## How a provider becomes available

The catalog in `index.ts` lists 31 providers, but this file does not decide which
ones are enabled. The vault does.

On startup the extension registers whatever the **cached inventory on disk**
says exists, then asks the Rust CLI in the background and registers anything new
that lands. Startup never waits on the vault: one read spawns `bw` five times
over roughly nine seconds (a `sync` to a self-hosted server included), which
would otherwise be a dead terminal on every launch. Two consequences worth
knowing:

* Adding a key in Bitwarden is the entire installation step for a new provider.
  No edit here, no restart of anything but pi — the provider appears a few
  seconds into the session, with a notify saying so.
* A provider whose key is missing never appears in `/model`, so the picker never
  offers something that cannot work. The inventory is refreshed once per session
  when it is older than `MANIFEST_FRESH_MS`, and `/bw-status` shows when it was
  last read.

Providers fall into two groups, and the split is not cosmetic:

**pi already ships them** (`builtin: true`) — Groq, NVIDIA NIM, Google AI Studio,
DeepSeek, OpenRouter, Kimi For Coding, Cerebras, Mistral, xAI, Z.ai, MiniMax,
Together, Fireworks. The extension registers auth and nothing else, so pi's own
curated model metadata survives untouched.

**pi has no such provider** — Ollama Cloud, LLM7, ModelScope, SambaNova, Nebius,
Nscale, SiliconFlow, Chutes, OVHcloud, Glhf, Alibaba Model Studio, Kilo Code,
Aion Labs, Agnes, AI21, GitHub Models, Cohere. These entries carry their own
`baseUrl` and a starter model list, and most also carry a `discover` endpoint:
the provider's own `/models` listing is fetched, and its id set replaces the
starter list so retired models disappear and new ones show up. The starter list
still supplies the facts the endpoint does not publish (context window, reasoning
support), matched by id and, for Ollama's build-date tags, by prefix.

Extension-supplied models *replace* a provider's models. That is why only the
second group has a model list at all.

## Setup

Vault items are Secure Notes in the `Axon` folder, following the convention
`tools/materialize-inference-key` already uses:

| provider | vault item |
|---|---|
| most | `inference-<slug>-api-key` |
| DeepSeek | `Deepseek API Key` |
| OpenRouter | `Openrouter API Key` |
| Kimi For Coding | `Kimi K2 API Token` |

Items that predate the convention are declared with `vaultItem` in the catalog;
anything new should follow `inference-<slug>-api-key`. The note body is the key
and nothing else. A note holding prose, or a login item holding a site password,
is not a token and will be reported as unusable rather than sent as one.

### Where a key is read from

The login keychain first, the vault as the fallback. `tools/inference-keys key <slug>` looks up
the service `inference-<slug>-api-key` with `security find-generic-password -w`,
and runs `bw` only when that misses. The keychain name is derived from the slug
alone, so the three legacy vault names in the table above are not carried into
the keychain, where they would be wrong rather than merely old.

The order is about cost, not preference. This gateway is pi's `apiKey` command,
and pi runs it once per registered provider, inside startup, serially. Measured
2026-09-17 on this machine: `bw get notes` 2.9 s against `security
find-generic-password -w` 10 ms. Eight providers of that difference was the
entire wait before pi's first prompt — 20-36 s before, 1.0 s after.

To migrate, with the vault unlocked:

```
"$SJEL_ROOT"/Packs/security/extensions/inference-keys/keychain-migrate.sh
```

The launcher derives the checkout root from its own path. It reads each key through
the Rust gateway, writes it to Keychain using `security -i` with hex data on stdin
(the secret is not an argv value), reads it back for an in-memory equality check,
then drops that slug's plaintext cache. Re-runnable; a slug already in Keychain is
skipped. No separate expect helper or Node-based credential reader is needed.

Nothing has to move at once. A slug with no keychain entry keeps working through
the vault, so providers can be migrated one at a time and the vault remains the
source for everything that has not moved.

### Ollama Cloud

1. Create a key at <https://ollama.com/settings/keys>.
2. In Bitwarden, add a Secure Note named `inference-ollama-cloud-api-key` in the
   `Axon` folder, and paste the key as the only content of the note body.
3. In pi, run `/bw-refresh`. The provider appears with its live model list.

Its `/v1/models` endpoint is public, so the model list is fetched without a key
and stays current on its own. Keys are limited by session and weekly quotas
rather than requests per minute.

## Commands

| command | effect |
|---|---|
| `/bw-status` | Vault state, whether the session works, and per provider: `ready`, `found` (item exists, not registered), or `missing`. Also shows the last failure reason. |
| `/bw-unlock` | Unlocks via the Axon keychain-backed helper, drops cached keys, and re-reads the inventory. |
| `/bw-refresh` | Drops cached keys, re-reads the inventory, registers anything new, and re-runs model discovery. |

A failed key lookup is reported by pi as `Failed to resolve API key for provider
"x" from shell command: ...`. pi discards the command's stderr, so `/bw-status`
is where the actual reason appears.

## Configuration

| variable | default | meaning |
|---|---|---|
| `PI_BW_TTL` | `60` | Seconds a fetched key is cached on disk. `0` disables caching. |
| `PI_BW_BIN` | `bw` | Bitwarden CLI binary. |
| `PI_BW_TIMEOUT_MS` | `8000` | Total budget for a key lookup, including fallback `bw` calls. The extension kills the helper at 9s, so stay under it. |
| `PI_BW_ADMIN_TIMEOUT_MS` | `30000` | Per-`bw` timeout for the Rust CLI's inventory and status commands. |
| `PI_BW_FOLDER` | `Axon` | Folder preferred when a name is ambiguous. |
| `PI_BW_CACHE_DIR` | `~/.cache/pi-inference-keys` | Manifest and key cache. Never holds anything else. |
| `PI_BW_KEY` | unset | Optional override path to the Rust CLI launcher. |
| `PI_KEYCHAIN` | `1` | Set to `0` to skip the keychain read and use the vault only. Exists to compare the two paths. |
| `PI_KEYCHAIN_BIN` | `/usr/bin/security` | Keychain CLI. |
| `PI_KEYCHAIN_TIMEOUT_MS` | `5000` | Per-read timeout for the keychain. A miss is a normal answer, not an error. |

### On caching

A keychain hit returns before the cache is consulted, so a migrated key is never
copied to disk at all. Everything below applies only to slugs still served by the
vault.

pi resolves an `apiKey` command on every request, and a cold `bw get notes`
measured 1.2s idle and 5.7s on a loaded machine. `PI_BW_TTL=0` gives the
strictest behaviour, key material never touching disk, at the cost of that
latency on every request. The 60s default keeps it to roughly one lookup per
minute.

The cached file is mode 0600 in `~/.cache/pi-inference-keys`, which is the same
trust boundary as the Bitwarden session key `bwu` already caches at
`~/.cache/axon/bw-session`. Run `/bw-refresh` to clear it, or set `PI_BW_TTL=0`
if you would rather not have it at all. If you run `bw serve`, the per-call cost
drops to milliseconds and `PI_BW_TTL=0` becomes cheap.

## Gotchas

**A stale `BW_SESSION` is normal, and handled.** The shell init exports the
session `bwu` cached at the time that shell started, so a shell that has been
open a while carries a session that the vault has since invalidated. The gateway
therefore tries the on-disk session *before* the environment variable, and falls
through to the next candidate when one reports the vault locked. A locked report
is only believed when no candidate works.

**Items are values, not semantics.** A Secure Note whose body is prose, or a
login item holding a site password, is rejected rather than sent as a bearer
token: `vault item "x" holds 3 whitespace-separated tokens`. That guard is why
the vault's `huggingface.co` site login is *not* used as a Hugging Face token.

**Duplicated vault items.** This vault holds two copies of several legacy items
(two `Openrouter API Key`, two `Kimi K2 API Token`, both in a folder named
`Personal` which itself appears twice). The Rust gateway resolves these
deterministically: the item in the `Axon` folder wins, otherwise the most
recently revised copy. `/bw-status` flags them as ambiguous. Creating
`inference-<slug>-api-key` items in `Axon` removes the guesswork, and is what
`tools/materialize-inference-key` expects anyway.

**A locked vault is not a silent failure.** `bw` exits 0 with empty output when it
cannot read the vault, so every read is `--nointeraction` and empty output is
treated as an error. A locked vault makes providers fail loudly instead of
sending an empty bearer token. Run `/bw-unlock`, or `bwu` in a shell. The model
list survives it: discovered catalogs are cached, and an unreadable vault keeps
the previous inventory rather than replacing it with nothing.

**A model list that vanishes is a registration error, not an empty vault.**
`~/.cache/pi-inference-keys/registration-errors.json` records any provider pi
refused, and `/bw-status` prints it. pi's `--list-models` has no UI to warn
through, so without that file a broken entry would just be absent.

**`maxTokens` above a provider's limit fails the whole request.** pi sends it as
`max_completion_tokens`, and Cohere answers `400 TOO_MANY_TOKENS` for anything
above 8192. That is why the catalog default is a conservative 8192 and only the
entries that can take more raise it. Raise it per model when long answers get
truncated.

**A provider error can look like a success in one-shot mode.** `pi -p` exits 0
even when the provider rejected the request, so an empty answer is not proof it
worked. Ask for a known reply, or read the session file's assistant
`stopReason`, which is where `400 status code (no body)` actually appears. That
message on Cohere meant the output cap above, and on NVIDIA meant a model id the
catalog still listed but the API had retired.

**`auth.json` wins over the vault.** If a provider has a stored credential from
`/login`, pi uses that and ignores this extension. `nvidia` and `opencode` are
currently in `auth.json`, so run `/logout nvidia` if you want the vault to supply
it.

**Cloudflare Workers AI is not included.** Its endpoint embeds an account id in
the path, and the account id is not a secret this extension has access to.

**The starter model ids go stale.** pi's own built-in Groq catalog still lists
`llama-3.1-8b-instant`, which Groq now answers with a 404, and its free tier
allows 8000 tokens per minute while pi's system prompt alone is over 10000. Both
are pi-catalog and plan limits rather than anything this extension controls; use
`discover`-backed providers for agent work on free tiers.

**`bwu` fails when the vault's server is unreachable.** The vault is self-hosted, so
`bw unlock` needs its server before it will accept the master password at all: with
Tailscale disconnected it reports `cannot reach the Bitwarden server at <url> — check
the network (Tailscale), then run bwu` rather than blaming the password. Connect
Tailscale and re-run `bwu`; the master password and the keychain entry are fine.

**Free tiers churn.** Context windows and `reasoning` flags in the catalog were
copied from freellm.net and `ollama.com/api/tags` in September 2026. They are the
first thing to re-check when a model behaves oddly, and `reasoning: true` on a
model that does not think is harmless.

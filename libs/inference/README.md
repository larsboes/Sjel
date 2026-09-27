# libs/inference

One home for **which model answers which job on this machine**.

A shared library, not a capability: no domain of its own, no upstream verdict, no CLI
(README.md#three-architectural-nouns). Consumers declare an `sjel-inference` path
dependency in the workspace.

## Why it exists

The same fact had four homes: `comms`' `SummarizerConfig` and `RelevanceConfig`,
`scouting`'s `EmbedConfig`, the since-deleted `libs/ai-client`'s `RouterConfig`, and
`tools/graphify.sh`'s `GRAPHIFY_BACKEND`. Each knew a base URL and a model name, and moving
a machine between runtimes meant editing all of them. `ai-client` was the last holdout and
went on 2026-08-12: it had no consumers, and its README claimed its local provider was the
vault-class safe path while its `LlmRequest` carried no data class at all and its router
failed over to Gemini on any local error.

`systems.toml` already stated the rule, in the oMLX entry:

> Referenced by id, not by URL, because host/port/model differ per machine.

This implements it.

## The shape

Two levels. Callers only ever touch the second.

- A **backend** is a server: an API shape (`openai` or `ollama`), a base URL, optionally a
  file to read a bearer key out of, and optionally the id of the host that serves it
  (`provided_by`). Declared once.
- A **role** is a job: `embedding`, `reranking`, `summarization`, `ocr`, or an explicitly named
  `cloud_*` task. It names a backend, the model on it, that model's input conventions, and
  optionally what the same job is called on another local runtime (`on_backend`).

`ocr` is named above and declared nowhere. It is rung 3 of the extraction ladder
(PRD Q63 → B30), `libs/extraction/src/ocr_role.rs` is its only caller, and no engine has
cleared the frozen DE/EN corpus at `libs/extraction/eval/` — the same gate that admitted
`multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` and rejected
`multilingual-e5-small-mlx`. Declaring it early would point a real dispatch at an unmeasured
model.

## Three tiers, not two (PRD Q39, 2026-08-25)

An endpoint used to be loopback or cloud, and `is_cloud_endpoint` was literally *https and
not loopback*. `upstreams.toml` chose Tailscale over NetBird for `tailscale cert` — so the
moment a peer gets a real certificate, **the operator's own MacBook reads as a cloud
provider** and demands a reviewed `providers.toml` entry. Getting the transport security
right made the classification wrong.

| Tier | Predicate | May see any class | Shares this GPU |
|---|---|---|---|
| Loopback | `is_loopback()` | yes | yes |
| Trusted peer | `is_trusted_peer()` | yes | **no** |
| Cloud | `is_cloud_endpoint()` | only through a reviewed policy | no |

**Declared, never inferred.** Trust is an intersection of two facts in two files, which is
the pattern `tools/lib/external-ref.sh` already owns for a capability this machine consumes
but does not run:

```
<overlay>/config/systems.local.toml   [lars-mac]  owner = "self"
<overlay>/config/inference.json       backends.peer.provided_by = "lars-mac"
```

An address is not a permission and neither is a name. Being reachable on the tailnet is not
either — a shared node is reachable. And presence in `systems.local.toml` cannot be the
test: `[nvidia-nim]` is in that file and is a cloud provider.

The resolver is the shell's, not this crate's. `external-ref.sh` reads the pair and
`tools/service-runner.sh` exports the answer as `SJEL_INFERENCE_TRUSTED_PEERS`, the same way
it already exports `[inference] backend` — Q39 says extend that resolver rather than invent
a second one. **Unset means no trusted peers**, which is a single-host deployment's normal
state and is byte-for-byte the behaviour that shipped before Q39.

**Ask `trusted_for_every_class()`, not `is_loopback()`, at a data-class gate.** One
predicate carried both questions while loopback and trusted-hardware were the same set. A
trusted peer is the first endpoint for which they differ: it may see any class, and it must
*not* queue behind this host's GPU admission gate, because it has its own. `libs/summarize`
carries the same split as `Target { loopback, operator_owned }`.

```rust
let role = InferenceConfig::load(overlay_config).role("embedding");
let vectors = role.embed(&texts, TextRole::Query)?;
```

A capability asks for a role and never learns whether it just talked to oMLX or Ollama.
That is the point: oMLX needs Metal and cannot exist on the family Pi, Ollama runs
anywhere, and moving between them is a config edit rather than a code change.

## Implementation

`src/lib.rs` serves the Rust capabilities that need model roles (Scouting and Comms today). It reads
`inference.json`, honours the backend override, and resolves bearer keys from the referenced
private file. Consumers receive a resolved role without hardcoding a URL or model. Comms also
uses the resolved role for model readiness, mixed query/document embedding batches and
OpenAI-compatible chat completion routing. Reranking roles use the Cohere/Jina-compatible
`/v1/rerank` shape, restore sorted results to input order and reject incomplete, duplicate or
out-of-range scores before a consumer can persist them. The Ollama-native API has no equivalent
route, so a reranking role names no Ollama model and simply does not resolve on a machine that
declares that backend — callers degrade from `None` rather than from a failed request.

## Config

`<overlay>/config/inference.json`, or `SJEL_INFERENCE_CONFIG` to point somewhere else.
Field docs live in `inference.config.example.json` beside this file.

A missing config is not an error. Every consumer is expected to degrade to something that
still works offline — scouting falls back to hash embedding — so a machine with no
inference set up keeps running instead of failing at startup.

Cloud-capable UI lists only roles whose names start with `cloud_`, resolve to a non-loopback
HTTPS backend, and declare `provider_name`, `cloud_data_tier`, `billing_mode`, a non-zero
`max_requests_per_day` and `max_input_tokens`. Supported
tiers are `public` and `pseudonymized_personal`; supported billing boundaries are `free_only`
and `prepaid_credit`. There is deliberately no unbounded pay-as-you-go mode. The public API may
expose that safe policy, role, model and protocol label, but never the backend URL, account ID,
key-file path or key value. A configured role remains unavailable until its private key file
contains a value. Selecting one records provider intent; a consumer must still implement an
explicit execution boundary before any request is made.

The explicitly selected role runs first. `failover_priority` then orders only roles with the
same exact `cloud_data_tier`; a Public role can never become a pseudonymized-Personal target or
the reverse. `prepaid_credit` additionally requires a valid `credit_expires_on` date and becomes
inert after that UTC day. UTF-8 request bytes plus a fixed prompt allowance form a conservative
provider-independent token upper bound. These local ceilings are hard stops, not claims about a
provider account's billing configuration, which the operator must also keep free-only or prepaid.

Relative `api_key_file` paths resolve beside the private `inference.json`. This lets an overlay
reference a gitignored `runtime-secrets/` file without recording one workstation's absolute
path. `tools/materialize-inference-key` is the human-run bridge from the matching Vaultwarden
Secure Note into that local file; it never prints the value.

## Machine override

`machine.toml`'s `[inference] backend` names the one local model runtime this machine has.
`service-runner.sh` exports it as `SJEL_INFERENCE_BACKEND` for every process it starts —
the same path `[capability.<name>] port` already takes to reach a process. An Intel,
Raspberry Pi or Linux machine says so once, in the file that already holds machine-local
facts, and no capability config changes.

It moves a role only when both of these hold, and each one is a defect it prevents:

- **The role's declared backend is loopback.** The override states which *local* runtime
  exists here. A hosted backend answers from every machine, so there is nothing
  machine-local to replace, and a reviewed `cloud_*` role is never dragged onto whatever
  this host happens to run.
- **The role names a model for that backend**, under `on_backend`. Model ids are
  backend-specific: swapping the backend alone leaves `multilingual-e5-base-mlx` addressed
  to Ollama, which 404s every request. A role that names none resolves to `None` — the
  documented degrade path, taken at resolution rather than at the first failed call.

```json
"embedding": {
  "backend": "omlx", "model": "multilingual-e5-base-mlx",
  "query_prefix": "query: ", "document_prefix": "passage: ",
  "on_backend": {
    "ollama": { "model": "nomic-embed-text",
                "query_prefix": "search_query: ", "document_prefix": "search_document: " }
  }
}
```

Prefixes are deliberately not inherited: they belong to the model, and the entry names a
different one. `cache_key()` still separates the two producers — `omlx:multilingual-e5-base-mlx`
against `ollama:nomic-embed-text` — so a cache written on one machine cannot be served on the
other.

**Rejected alternative: per-machine role definitions.** A `[roles]` block in `machine.toml`,
or a second `inference.<machine>.json`, would put the model and its prefixes in as many places
as there are machines. The pairing of a model with its input conventions is the one fact this
library exists to keep in a single home, and duplicating it per host is how it drifts. The
machine states only what is genuinely machine-local — which runtime it has — and the shared
file keeps every model name.

## Two things portability actually requires

**Models are not interchangeable.** `multilingual-e5-*` wants `query: ` and `passage: `
role prefixes; `nomic-embed-text` wants `search_query: ` and `search_document: `. Sending
the wrong ones costs retrieval quality and raises no error, so the prefixes belong to the
role, beside the model, and travel with it.

**Cached vectors belong to the model that produced them.** A cache keyed on the input alone
will serve e5 vectors to a nomic run after a backend switch: every score wrong, nothing
logged. `ResolvedRole::cache_key()` returns `backend:model` so a cache can name its
producer and refuse a mismatch. `capabilities/scouting/src/embed.rs` is the worked example.

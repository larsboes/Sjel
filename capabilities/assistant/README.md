# Assistant

An OpenAI-compatible chat client for the dashboard and native Ask. Local is the default. Optional
cloud roles may be configured in the private overlay; they appear in Ask only when the role,
provider review, billing policy, and credential are ready.

## Configure the role

Add a dedicated role to the overlay's `config/inference.json`; do not repurpose
`summarization`:

```json
"assistant": {
  "backend": "ollama",
  "model": "qwen3:8b",
  "max_input_tokens": 8192
}
```

The backend must resolve to a loopback URL. `max_input_tokens` is required; the service enforces
its value as a conservative UTF-8 byte ceiling, and the dashboard uses that ceiling when planning
fits. The shared example is in
[`libs/inference/inference.config.example.json`](../../libs/inference/inference.config.example.json).

## Two local rungs, and why

A machine that runs two local models can name a ladder instead of one role: `escalates_to` on
`assistant` names the strong rung, and nothing in the code knows either model's name.

```json
"assistant":        { "backend": "foundation-models", "model": "apple-foundationmodel",
                      "max_input_tokens": 8192, "escalates_to": "assistant_strong" },
"assistant_strong": { "backend": "omlx", "model": "Qwen3.5-4B-MLX-4bit",
                      "max_input_tokens": 8192 }
```

Measured on 2026-10-24, same three prompts and same tools offered: the on-device rung invented
`calendar_entries{"date": …, "status": "ready"}` for "reply with the single word: ready" and
returned a bare `502` once in three, while the 4B answered correctly on all three at about four
extra seconds. The ladder is therefore a correctness choice, not a cost one, and it is decided by
a rule table rather than a model:

- `capabilities/assistant/src/routing.rs` sends a prompt to the strong rung when it names Sjel
  state, a calendar word, a weekday, or a project/trip noun, and when the conversation already has
  history. Everything else starts fast.
- **The fast rung is never offered a tool.** A model that cannot call one cannot invent one, which
  removes the measured failure class instead of retrying it.
- A fast-rung attempt that fails for a reason the caller can prove — transport error, empty reply,
  or a refusal — is retried once on the strong rung, only while nothing has been streamed. A
  streamed answer cannot be recalled, so the streaming route relies on the router deciding the rung
  up front. The response's `source` is `local` or `local-escalated`, and `model` names the role
  that actually answered.
- A ladder refuses a prompt that does not fit its *tightest* rung, so an escalation cannot turn a
  fitting prompt into a rejected one.

Misrouting is a word in the router's vocabulary plus a line in its fixture table, never a prompt
rewrite. A deployment that declares one role has a ladder of one and keeps the tools.

## Routes

- `GET /health`: liveness only.
- `GET /ready`: reports whether the local role is configured, which model is the strong rung, and
  the tightest input ceiling across the rungs; does not return credentials.
- `GET /api/ask/providers`: lists configured cloud roles without endpoint URLs or credential paths.
- `POST /api/generate`: `{ "prompt": "...", "instructions": "...", "max_tokens": 512 }`.
- `POST /api/ask`: `{ "prompt": "...", "history": [{ "role": "user|assistant", "content": "..." }], "instructions": "..." }` — multi-turn session powered by [`libs/agent`](../../libs/agent) with read-only Sjel capability tools (`sjel_health`, `calendar_entries`, `projects_list`, `project_records`, `trips_plans`) and no `bash` or filesystem tools. Returns `{ "text", "model", "source", "tools" }`.
- `POST /api/ask/stream`: same request as `/api/ask`; returns server-sent JSON events for streamed text and tool calls/results. The `x-sjel-ask-run` response header identifies the active run.
- `POST /api/ask/{run_id}/steer`: `{ "message": "..." }` queues guidance for the next model request after the current tool batch. Runs have a bounded input queue and the service allows at most eight active streamed runs.
- `POST /api/ask/{run_id}/stop`: requests cooperative cancellation of an active streamed run; cancellation takes effect after the current model stream or tool batch yields.

The service binds loopback through `sjel_server::serve_local`. Protected requests require the
shared inbound token. The shell injects that token on its server-to-server proxy hop; it is never
sent to browser JavaScript. On iOS, the signed native bridge reaches `/assistant/` through the
same shell proxy.

For a one-off public prompt, a caller may send `provider_role: "cloud_<name>"` and
`confirm_public_send: true`. Cloud Ask sends only the visible prompt with a fixed server-owned
instruction; it refuses local history and caller-supplied instructions, tool schemas/results,
extensions, steering, and follow-up messages. The agent checks the provider's
current `providers.toml` review, role billing window, credential readiness, input ceiling and a
cross-process daily request ledger in the private overlay immediately before each send. The
operator must still classify the prompt accurately; this is an explicit c0 attestation, not an
automatic content classifier. c1 derivatives, c2, c3 and unknown data remain unavailable to this
general-purpose route. Use the reviewed derivative queue for c1.

Add provider roles only to the private overlay's `config/inference.json`, with a `cloud_assistant_` role
name, an HTTPS OpenAI Chat Completions backend, an `api_key_file` reference, provider name,
`public` or `pseudonymized_personal` tier, billing mode, non-zero daily ceiling and input limit.
The provider must also have a current `providers.toml` review. No provider is enabled by installing
the UI or by adding an API key alone. This is an extensible endpoint profile, not a collection of
provider-specific SDKs; endpoints that do not speak the supported chat-completions protocol need
a separate reviewed adapter.

The shared inference example includes reviewed Ask profiles for NVIDIA NIM's current free
DeepSeek V4.1 Flash endpoint, Google Gemini 3.6 Flash, and OpenAI GPT-4.1 mini. The NIM and
Google profiles are capped at one request per day and use Sjel-only credential file references;
the files are intentionally absent from the example. The OpenAI profile carries an expired credit
date so it stays unavailable until the operator records the actual prepaid-credit expiry. These
are setup templates, not proof that the corresponding Pi logins are connected.
DeepSeek-direct and OpenCode Go remain absent until their providers receive Sjel privacy reviews.

Provider reviews and provider logins are separate: `providers.toml` records which services may
receive each data class, while the private overlay defines the concrete endpoint, model, key
reference, billing mode and limits. Pi's saved logins are not discovered or imported, so no
provider role appears until its Sjel profile is configured. The generic chat-completions route can
cover OpenAI, DeepSeek, NVIDIA NIM, Gemini's OpenAI
compatibility endpoint, OpenRouter, and OpenCode Go models whose catalog entry uses
`/v1/chat/completions`. DeepSeek documents a compatible chat-completions endpoint; NIM documents
streaming chat completions and tool calling; Gemini documents its OpenAI-compatible chat endpoint.
OpenCode Go is model-specific: its current catalog uses Chat Completions for some models, but
Responses or Messages for others, and those other protocols are not supported by this route.
See the [DeepSeek chat API](https://api-docs.deepseek.com/api/create-chat-completion/),
[NVIDIA NIM API](https://docs.nvidia.com/nim/large-language-models/latest/reference/api-reference.html),
[Gemini compatibility guide](https://ai.google.dev/gemini-api/docs/openai), and
[OpenCode Go endpoint catalog](https://dev.opencode.ai/v2/docs/console/go/).

Responses identify the selected provider/model rung. The UI displays the exact prompt in the
transcript and repeats the `public c0` confirmation on every cloud send. Local remains the default.

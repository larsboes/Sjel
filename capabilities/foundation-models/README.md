# foundation-models

Apple's on-device model, served over the OpenAI chat-completions shape on loopback.

## Why this exists

Sjel's digest ladder picks a shape from how much source there is: a few bullets for a
short article, grouped sections for a long transcript. Until now every rung ran on the
same 26B model through oMLX, which meant a two-paragraph digest of a short page competed
for the same GPU memory as a forty-page paper.

On 2026-08-05 that competition produced a real failure. Four concurrent prefills pushed
oMLX past its hard memory watermark and it aborted all four, two of them feed digests.
Apple's model does not sit in that budget at all. It runs in the OS, answers a short
digest in about two seconds against twelve to twenty for the 26B, and taking the cheap
rungs off the shared GPU is worth more than the quality difference at those rungs.

## The 4,096-token window is the feature

`SystemLanguageModel.contextSize` is 4096, shared between prompt and reply. That is not
a limitation to work around; it is what makes this a rung-selector rather than a
downgrade. Roughly:

| Rung | Source | Fits? |
|---|---|---|
| Brief | under ~2,500 chars | yes |
| Standard | ~2,500 to 9,000 chars | yes |
| Sectioned | above ~9,000 chars | no |

The digest that started this work needed 3,568 input tokens plus 1,000 of reply. It
cannot run here, and `libs/summarize::fits_context` knows that before the request is
made, so the strong model takes it.

Over-window requests are refused, never truncated. A silently shortened prompt produces
a digest of the first half of a document and says nothing about it, which is worse than
no digest because the reader cannot tell.

## The other local rung is oMLX

oMLX serves every local role this one cannot: the sectioned rung above, plus embedding
and reranking. It speaks the same OpenAI-compatible shape on loopback and is declared as
a backend id in the same map this capability's role joins — see
`libs/inference/inference.config.example.json`, which owns the ids, the addresses and the
Metal/Apple-Silicon constraint.

It is not an Sjel capability. There is no `service.toml` for it and Sjel neither installs
nor supervises it: it is a `systems.toml` entry (`[omlx]`, host-native because Metal is
unavailable inside the Linux runtime), and `upstreams.toml` carries the verdicts for the models it serves
(`multilingual-e5-base-mlx`, `bge-reranker-v2-m3-mlx`) rather than for the server. A machine
either has it running or configures roles that do not name it.

Recorded here on 2026-08-25 because `capabilities/learning` was deleted (PRD D6). That
capability was a README and an ISA with no code, and the one claim in it nothing else owned
was that oMLX is the local-AI rung Sjel builds on.

## Wiring it up

Add a second summarization role to `inference.json` in the overlay:

```json
"summarization_light": {
  "backend": "foundation-models",
  "model": "apple-foundationmodel",
  "max_input_tokens": 4096
}
```

`apple-foundationmodel` is the only id apfel serves, and it refuses any other with
`model_not_found` — a 404 that arrives at request time, not at startup, so a wrong name here
reads as "the light rung never summarizes anything" rather than as a typo. `apple-on-device` was
the name the retired Swift server answered to and is the shape of that mistake.

with the matching backend:

```json
"foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:8091/v1" }
```

`max_input_tokens` is required. Without it comms cannot tell whether a source fits and
skips the role entirely, because guessing produces a context error instead of a digest.

Nothing else changes. `comms` picks the light role when the source demonstrably fits and
falls back to the strong one otherwise, and a machine with no light role configured
behaves exactly as it did before.

`inference.json` is shared across the overlay's machines, and this backend exists on exactly
one class of them. A machine that declares another local runtime in `machine.toml`
(`[inference] backend`) resolves this role to nothing unless it names a model there under
`on_backend` — which is the correct outcome, not a gap: `comms` then takes the strong role,
the same as a machine with no light role at all.

## Requirements

macOS 26+, Apple Silicon, Apple Intelligence enabled. `brew install apfel`; the verdict lives in
`upstreams.toml [apfel]`, and `toolchain.toml` declares the binary under
`needed_by = ["capability:foundation-models"]`, so an enabled capability with it missing is
reported rather than found later in a watchdog log.

```
$ curl -s 127.0.0.1:8091/health
{"model":"apple-foundationmodel","model_available":true,"context_window":4096,"status":"ok", ...}
```

The Pi never enables this capability. There is no manifest field for "Mac only".

**Without Apple Intelligence, apfel starts anyway.** The retired `afm-server` refused to, and that
is the one contract lost in the swap. apfel binds the port and reports
`"status": "model_unavailable"` — its own source says an unavailable model "never crashes
startup" — so on a misconfigured host this capability reads as up while answering nothing. The
failure still surfaces, one layer later: `libs/inference` resolves the role through a readiness
probe and `comms` falls back to the strong summarization role when the light one does not
resolve. `ready_path` is the seam if that ever proves optimistic; apfel has no endpoint that
fails on unavailability today, which is why this is a paragraph and not a manifest field.

## Design notes

**Over-window requests now answer 400, not 200.** The second behavioural change from the swap,
and it inverts a decision this file used to argue for. `afm-server` returned 200 with an error
envelope precisely because "the one consumer reads any non-2xx as *server down* rather than
*this request will not fit*". apfel returns a correct OpenAI `context_length_exceeded` 400, which
`libs/inference` turns into `Err("... returned HTTP 400")`.

Measured, and it is a backstop rather than the normal path: `libs/summarize::fits_context`
decides whether a source fits **before** the request is made, so a well-behaved rung never sends
one. What changed is the diagnostic when that estimate is wrong — a status code instead of a
typed cause. Worse to read, still loud, and not worth 311 lines of Swift to keep.

**Loopback by default.** apfel binds `127.0.0.1` unless `--host` says otherwise, the same posture
the retired shim enforced by having no option at all. `--port ${SJEL_PORT}` in the manifest keeps
it off apfel's 11434 default, which this deployment has already given to Ollama.

## Sources

- [Foundation Models framework](https://developer.apple.com/documentation/foundationmodels)
  — `SystemLanguageModel`, `LanguageModelSession`, `GenerationOptions`. The authority for
  `contextSize` and `exceededContextWindowSize`. Note the documentation site is a JS
  application and does not fetch as text; the `.swiftinterface` in the installed SDK is
  the more reliable read.
- [apple/foundation-models-utilities](https://github.com/apple/foundation-models-utilities)
  — emerging patterns. Its `ChatCompletionsLanguageModel` runs the opposite direction to
  this shim (a Swift session talking *to* an OpenAI server), which is useful for the wire
  shape but is not the mechanism here. Its history modifiers — `summarizeHistory()`,
  `rollingWindow()`, `droppingCompletedToolCalls()` — are the thing to reach for if this
  ever holds a conversation rather than answering one-shot prompts against 4,096 tokens.
- [rudrankriyam/Foundation-Models-Framework-Lab](https://github.com/rudrankriyam/Foundation-Models-Framework-Lab)
  — a workbench of runnable examples: `@Generable`/`@Guide` structured output, tool
  calling, streaming, availability gating. This used to say structured output was "the obvious
  next step and deliberately not in this version". It arrived by adopting apfel rather than by
  writing it, along with streaming, `/v1/responses` and MCP tool calling — which is the whole
  argument for the swap.

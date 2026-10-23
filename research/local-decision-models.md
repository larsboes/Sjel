# Can a local model make typed decisions for Sjel?

First written 2026-09-30.

## What the sources show

Contrastive Language Models provide a released, self-hostable System One candidate scorer.
CLM-v0.1-8B uses two learned projection heads, one for states and one for actions, on a frozen
Qwen3-8B encoder. The authors describe contrastive training that brings matching states and
actions together. They published their [article](https://contrastive-lm.notion.site/) on
2026-09-23 and released the [CLM-v0.1-8B model card and checkpoint](https://huggingface.co/Contrastive-LM/CLM-v0.1-8B).
The code and checkpoint are Apache-2.0; the model card names the same license for the base encoder.

CLM scores supplied candidates rather than generating text. Its API supports choices, boolean
questions and scores, as well as direct candidate ranking. States and actions are encoded
separately, which allows fixed candidate embeddings to be reused. This is a different deployment
option from [TypeSafe's hosted Jev](https://typesafe.ai/), even though the
[CLM implementation](https://github.com/Contrastive-LM/CLM#api-reference) provides a
TypeSafe-compatible interface.

The candidate set limits the answer. The model card explicitly says its probabilities are
relative to the candidates supplied. The API documents `confidence` as the top probability minus
the mean of the others. Neither quantity establishes a calibrated probability of correctness on
Sjel's tasks. A system must allow an unresolved result; giving a scorer only unsuitable choices
does not make its best choice suitable.

The encoder is part of the model, not an interchangeable embedding service. The released heads
require Qwen3-8B last-token-pooled embeddings. BGE embeddings, Qwen3-4B chat responses, or cosine
similarity between unprojected Qwen vectors are not this checkpoint. The
[reference serving recipe](https://github.com/Contrastive-LM/CLM#quickstart) uses a vLLM
Qwen3-8B pooling server and the separate `clm-serve` process. That recipe does not establish a
working Apple Silicon deployment or equivalence after changing the encoder runtime or precision.

The released 8B model operates on text states, not raw photos or video. The model card describes
a future multimodal release, and the repository lists vision support on its roadmap. Those plans
are not an available capability of this checkpoint. The authors' computer-use, gaming and
verifier results also do not measure media organization. The reported agentic verifier results
use fine-tuned heads, not the reference checkpoint zero-shot
([model-card limitations](https://huggingface.co/Contrastive-LM/CLM-v0.1-8B#limitations),
[upstream results](https://github.com/Contrastive-LM/CLM#results)). Sjel has not reproduced them.

## What already exists in Sjel

The [harness Pack](../Packs/harness/README.md#local-clm-classifier-experimental-pi-extension)
contains an [experimental Pi adapter](../Packs/harness/extensions/clm-classifier.ts). It registers
the classifier only when `SJEL_CLM_ENABLE=1`, fixes its request endpoint to loopback, and validates
typed answers. Its always-registered `/clm-probe` command checks for a working encoder and the
reference head before asking one synthetic question.

The [wire-contract tests](../tools/pi-clm-classifier.test.ts) use synthetic responses. They check
registration, answer conversion and rejection of malformed responses. They do not run the
encoder or heads, establish classification accuracy, or prove that a real service has passed the
probe. This adapter is also separate from the shared Rust
[inference-role resolver](../libs/inference/README.md); its existence does not make CLM available
to a capability through that resolver.

## What belongs in code, and what could use a model

For [Media](../capabilities/media/README.md), Rust should retain ownership of measurements, file
identity, approved organization rules, path checks and any eventual file operations. Reuse the
existing metadata and exact-byte machinery rather than asking a model to invent dates or decide
whether files are identical.

A local CLM could rank an explicit shortlist of categories or help identify which collections
need review. Its input would be a bounded text description of known metadata and collection
context. It cannot supply missing visual context. Human-reviewed mappings remain authoritative,
and a ranking grants no permission to move, rename or delete a file. If a model is unavailable,
the deterministic preview and manual review must still work.

This is a possible use of the candidate, not an implemented Media integration or an admission
decision for a model or provider.

## What the sources do not show

- The 4-bit community MLX port loaded and scored locally on this Mac, but that does not establish
  parity with the upstream vLLM path or portability to another serving stack.
- One balanced synthetic Documents set compares CLM, Apple Foundation Models and current keyword
  rules. It does not establish accuracy on real documents or general superiority for Sjel tasks.
  CLM and the current rules also lack evidence spans required by the proposal contract.
- That candidate probabilities or the documented confidence margin justify an autonomy threshold
  such as 0.95 on these tasks. Task-specific accuracy, abstention and calibration need measurement.
- That a text-only model can infer an event, person or subject when the supplied filenames and
  metadata contain no evidence for it. Missing GPS or capture dates remain missing evidence.

## Measured 2026-09-30

The same adapter now reaches CLM or Ollama 0.35's System One API, because both take the same
request. Ollama served three decision models on an Apple M4 Pro for the
[decisions benchmark](benchmarks/decisions/README.md): 43 synthetic, author-labelled cases.

| Model | Correct | p50 | p95 |
|---|---|---|---|
| `tev1` (4B) | 43/43 | 350 ms | 440 ms |
| `nimble` (9B) | 42/43 | 740 ms | 893 ms |
| `tev1:0.8b` | 36/43 | 71 ms | 86 ms |

The result files are in [`benchmarks/decisions/results/`](benchmarks/decisions/results/). The two
larger models are at the ceiling of this small set, so it does not separate them. Harder or
real-shaped cases are needed before a choice between them. Ollama's `confidence` for a choice
differs from CLM's documented definition (top probability minus the mean of the others), so the
two cannot be compared directly.

## Measured 2026-10-22 — synthetic Documents categories

A separate 48-item English/German corpus was used for a cross-source comparison. It was hand-made,
balanced across Finance, Events and Reference, and split into clear and overlapping examples. The
CLM event description and Apple FM evidence prompt were selected on separate development sets;
this comparison corpus was evaluated with those prompts fixed. No personal documents or `c3` content
entered any prompt.

| Source | Result on the 48-item comparison set | What remains unavailable |
|---|---|---|
| Deterministic keyword rules | 13 category suggestions; 11 correct, 2 wrong | No exact source span; only 1 Events suggestion, incorrect |
| CLM, 4-bit MLX port | 40/48 correct; Events 11/16 | Category scores only; confidence is uncalibrated |
| Apple Foundation Models via local `apfel` | 47 correct category outputs of 48 samples; one safety block | 45/48 exact, bounded evidence proposals, all correct; no confidence; two unusable quotes |

The Apple model blocked one synthetic tax-summary request with its safety guardrail. The evaluator
recorded no proposal and did not rephrase or retry. Across sources, category outputs conflicted on
10/48 samples. At least one source returned the expected category on all 48, but only 45 samples had
a correct proposal with evidence meeting the selected 20-word / 160-character limit. These are
small synthetic results, not a measured production error rate. No result is wired to Documents,
and the shared review/fusion behavior has not been implemented.

The frozen inputs and reports are in
[`capabilities/documents/eval/`](../capabilities/documents/eval/):
`clm-corpus-combined-holdout-v1.json`, `clm-eval-combined-holdout-v1-2026-10-22.json`,
`apple-fm-eval-combined-holdout-v1-2026-10-22.json`,
`rule-baseline-v2-holdout-v1-2026-10-22.json`, and
`cross-source-eval-combined-holdout-v1-2026-10-22.json`. The comparison set is spent; do not tune
prompts or rules on it.

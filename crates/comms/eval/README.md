# Comms quality evaluation

## Not here: the OCR engine gate

`libs/extraction/eval/` holds the frozen DE/EN corpus that decides which OCR engine may join the
extraction ladder. It judges an **engine** — can this reader read a page at all — and both
consumers of `libs/extraction` have to be able to cite it, so it lives beside the trait.

This corpus stays here because it judges something else: the text passed from an extractor into
`normalize`, which is a comms decision about comms text. The two are not substitutes and neither
subsumes the other.

## Extraction and normalization

`extraction-corpus.json` is the frozen, offline gate for the text passed from each
extractor class into normalization. Its synthetic stored-page snapshots cover an
article, repository, paper, client-rendered page, captured page and PDF text. Each
fixture names exact text that must survive and exact boilerplate that must not.

Run it from `capabilities/comms`:

```sh
cargo run --bin comms-extraction-eval -- eval/extraction-corpus.json
```

The runner reports raw and normalized character counts, total retention, useful
retention and boilerplate leakage per fixture. The fixed gate requires 100% of
the judged useful text and 0% of the judged boilerplate; a miss exits non-zero.
It also requires every declared input class and every inspectable normalization
rule to have fixture coverage. A host adapter change refreshes or adds its stored
snapshot before implementation; a normalization rule change adds its expectation
to a fixture. Change judgements before examining the new result, never to make a
failed implementation pass. Append every accepted baseline under `results/`.

The snapshots begin at the extractor/normalizer seam, not at the network. They
therefore remain deterministic and measure the canonical text contract while the
live fetchers keep their separate HTTP tests. The PDF fixture is stored extracted
text because xberg adoption and byte-level PDF extraction remain separately
blocked by their dependency cooldown.

## Relevance

This directory owns the small public quality baseline for semantic Feed–TELOS ranking. It is
separate from unit tests: the runner calls the real local oMLX server, while the committed
corpus contains only synthetic text and explicit human judgements.

Run it from the Sjel root:

```sh
bun capabilities/comms/eval/run-relevance.ts
```

The cross-encoder candidate is gated against the same corpus and unchanged judgements:

```sh
bun capabilities/comms/eval/run-reranking.ts
```

That runner calls the loopback `/v1/rerank` route once per lens and validates the returned index
and closed `0..=1` score contract. `OMLX_RERANKING_MODEL` selects another installed model. The
embedding baseline remains the candidate-retrieval gate; the reranking run deliberately isolates
the second stage without rewriting the corpus around a model's scores. A disposable, loopback-only
test server with authentication disabled may set `OMLX_NO_AUTH=1`; the normal path still requires
the configured key reference.

The runner reads `.auth.api_key` from `~/.omlx/settings.json`, sends one batch to the loopback
`/v1/embeddings` endpoint and never prints the key or vectors. `OMLX_SETTINGS_PATH`,
`OMLX_BASE_URL` and `OMLX_EMBEDDING_MODEL` override machine-specific details; E5-base is the
default. A non-loopback endpoint is rejected because this evaluation is intentionally local. A
loopback server that enforces no key of its own may set `OMLX_NO_AUTH=1`, the same spelling
`run-reranking.ts` uses; the normal path still requires the configured key reference.
`OMLX_QUERY_PREFIX` and `OMLX_DOCUMENT_PREFIX` override the `query: `/`passage: ` pair, under the
field names the inference role uses for the same thing. A model that wants no instruction prefix
is as ordinary as one that wants E5's, and measuring a candidate under prefixes it will not run
with measures a configuration nobody deploys. The empty string is a real value here.

An explicit first argument selects another schema-compatible corpus. Private real-world corpora
and their results stay in the private overlay and are never copied into this directory:

```sh
bun capabilities/comms/eval/run-relevance.ts "$SJEL_PERSONAL_ROOT/config/private-corpus.json"
```

A query may provide named `text_variants`. Setting
`RELEVANCE_EVAL_QUERY_VARIANT=<name>` evaluates that representation while keeping candidates,
judgements and acceptance thresholds unchanged. This exists for controlled input-shape
experiments; a missing variant is an error, and variants must never be used to rewrite
judgements after seeing model scores. Candidate IDs need to be unique within a query, not across
queries, so one real Feed snapshot can be judged independently against multiple TELOS lenses.

Apple's built-in sentence embeddings run against the same corpus:

```sh
xcrun swift capabilities/comms/eval/run-apple-nlembedding.swift
```

For every query, the runner chooses Apple's German or English sentence model and asks that
single model to rank both German and English candidates. This makes cross-language behavior an
observed result rather than assuming that separate language-specific vector spaces align. It
preflights both languages and records each system model's revision and dimensions because macOS
may update those assets. On this machine the German revision-1 model has 640 dimensions and the
English revision-1 model has 512. Both run, but neither preserves the corpus's intended
cross-language ordering.

Apple's shared Latin contextual model is the executable native comparison:

```sh
xcrun swift capabilities/comms/eval/run-apple-contextual.swift
```

It declares [both German and English in one Latin vector space](https://developer.apple.com/documentation/naturallanguage/nlcontextualembedding/languages).
The runner mean-pools Apple's
[subword vectors](https://developer.apple.com/documentation/naturallanguage/nlcontextualembeddingresult/enumeratetokenvectors%28in%3Ausing%3A%29)
into one vector per text, passes each text's actual language and applies the unchanged acceptance
gate. Missing system assets are never downloaded implicitly; an intentional first run may ask
macOS for them:

```sh
xcrun swift capabilities/comms/eval/run-apple-contextual.swift --request-assets
```

If Command Line Tools and the active Swift SDK do not match, select the installed full Xcode
for either command with
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`. The native comparison is
experimental: Apple's contextual API supports pooling, but the sentence-embedding API is its
purpose-built semantic-similarity surface.

## Why this shape: judgements before scores

Each query has four candidates rated from 0 (unrelated) to 3 (direct match), with a written
rationale. The cases balance German and English and make the strongest match cross-language.
Acceptance requires every top-ranked candidate to be useful, at least 75% correctly ordered
unequal pairs and mean nDCG of at least 0.90.

Add or revise a judgement from its meaning and rationale before looking at a model's score.
Never tune a label merely to turn a failing model green. Private TELOS notes, real Feed items
and their result records belong in the private overlay, not this public baseline.

Results are append-only evidence under [`results/`](results/). The first run,
[`2026-07-30-multilingual-e5-small-mlx.md`](results/2026-07-30-multilingual-e5-small-mlx.md),
fails the fixed acceptance gate and records the two ranking errors instead of weakening the
judgements. The larger
[`multilingual-e5-base-mlx` run](results/2026-07-30-multilingual-e5-base-mlx.md) then passes
the unchanged gate and records the measured resource delta that justified promotion. Both
native Apple variants are retained as tested failures:
[`NLEmbedding`](results/2026-07-30-apple-nlembedding.md) uses the language-specific sentence
models, while the shared [`NLContextualEmbedding`](results/2026-07-30-apple-contextual.md)
runs after an explicit asset request and mean-pools subword vectors. Neither clears the same
corpus.

The second-stage
[`bge-reranker-v2-m3-mlx` run](results/2026-08-04-bge-reranker-v2-m3-mlx.md) uses the same
corpus unchanged and passes at 6/6 useful top-1, `0.912` pairwise accuracy and `0.993` mean nDCG.
Its result is kept separately because cross-encoder scores and embedding cosine scores are not
the same scale.

That second stage is **measured but not running**. No `reranking` role is declared in this
deployment, and `relevance.rs` enters the stage only when one resolves — so every Feed ranking
is first-stage only. It is not a regression from oMLX leaving the host: neither pre-change
backup of the inference config declares the role either. Restoring it needs oMLX, because
`ResolvedRole::rerank_endpoint` returns `Err` for an Ollama backend — the Ollama-native API
exposes no `/v1/rerank`, and Ollama is what holds the local roles now.

[`bge-m3` served by Ollama](results/2026-08-30-bge-m3-ollama.md) cleared the unchanged gate on
2026-08-30 at 6/6 useful top-1, `0.941` pairwise accuracy and `0.994` mean nDCG, prefix-free, and
**is now the adopted first stage**. E5-base remains the selection on a machine whose `[inference]`
backend is oMLX, and its record stands as the evidence it was. That record also carries why the
question was reopened at all: the configured oMLX backend is absent from the host, so ranking had
been falling back to the deterministic `lexical` control.

## Mail stream classification (the local model rung)

`comms-mail-model-eval` decides whether the local model rung may move a category
at all. **It makes no model call.** It joins a frozen corpus of hand-written
labels to the verdicts a shadow pass already stored, keyed by triage id, so
re-scoring is free and the number is the same every run. All the model time
lives in the pass, which has its own receipt and its own bounded retry; the
scoring stays pure, and drift between the two is impossible because the runner
builds no prompt. That is the same property that makes the redaction gate above
a gate rather than a report.

The corpus is **not in this repository**. It holds real mail, so it lives in the
private overlay at `config/comms-mail-stream-shadow.json` beside
`comms-redaction-shadow.json`, with the companion
`comms-mail-stream-shadow.md` holding the write-up — the same `.json` + `.md`
pair the redaction and relevance corpora already use. The shape, with synthetic
fixtures only, is `schemas/comms-mail-stream-shadow.example.json`. The runner
takes the corpus path as `argv[1]`; the in-repo default path deliberately does
not exist.

```sh
comms mail corpus --out "$SJEL_PERSONAL_ROOT/config/comms-mail-stream-shadow.json"
#   one fixture per fallback row, `label` and `urgency_band` EMPTY. Fill both by hand,
#   in one pass, BEFORE the next line runs. Refuses to overwrite without --force.
comms mail classify --shadow          # fill the verdict table, ~2s per thread
cargo run --bin comms-mail-model-eval -- "$SJEL_PERSONAL_ROOT/config/comms-mail-stream-shadow.json"
```

`comms mail corpus` writes the skeleton and nothing else — no verdict is read while it runs,
so a labeller cannot be shown the answer they are meant to write. The one field it guesses is
`language`, from an umlaut or a German function word, and the corpus's own `_method` says so;
the split by language is what makes one English prompt over a mixed mailbox measurable, and
121 blank language fields would cost the labeller a judgement they can make faster by
correcting one. `evaluate_file` **refuses** a corpus with any empty `label`, naming the
count: an empty label scored as a stream name disagrees with every verdict, and a half-filled
corpus would report a low agreement that reads like a measurement of the model.

**What it measures.** Model agreement against the labels, split by language,
because one English prompt over a mixed mailbox is exactly the assumption that
deserves a number. The **control** needs no model either: every fixture the rung
sees is a fallback row, so the deterministic classifier answered `aktiv` for all
of them, and its agreement is simply the share of labels reading `aktiv` —
computable from the corpus before anything runs. Then the **false eviction rate**
from `aktiv`, which is the one failure here that costs a decision: every applied
write is an eviction, and a correctly-`aktiv` mail moved out of `aktiv`
disappears from the operator's ladder. Then the **urgency band error**, mapping
the model's continuous 0–10000 self-report onto the corpus's four ordinal bands.

**What the first run's job is.** To find out what the number IS. Three of the
four acceptance thresholds ship `null`, and they stay null until the first run
has been read — a threshold invented before the measurement is a number chosen
to be met, the same discipline that set the redaction corpus's
`minimum_recall_percent` after its second measurement. `max_false_eviction_percent`
is different in kind and carries a value from the start, because it is a stated
policy judgement rather than a measurement. The flip from shadow to live also
needs model agreement at least ten points above the control; the ten is a
judgement, recorded and reversible.

A fixture whose stored verdict is missing, or was reached against a different
subject, sender domain, class or prompt, is **skipped with a named reason**
rather than scored — an all-skipped corpus is a FAIL, never a perfect score. The
output prints fixture ids, stream names and counts, and never a subject, a
preview or a rationale: unlike the redaction runner, whose leaked value IS the
finding, the finding here is a category.

## Digest quality (the summarize ladder)

`comms-digest-eval` is the gate `libs/summarize` did not have. PRD D16 recorded what its
absence cost: on 2026-08-30 the strong local rung became a 4B where it had been a 9B, and
Cohere entered the roster as a third public-tier provider, and **neither quality change could
be measured**. Both were taken on availability and cost alone, and a ladder whose steps are
unmeasured is an ordering nobody has checked.

Built on the redaction shadow's shape, the third of these corpora to use it:

```sh
comms digest corpus --out "$SJEL_PERSONAL_ROOT/config/comms-digest-quality.json"
#   N generated digests per rung (default 20), `faithful` and `useful_band` null.
#   Read the SOURCE for each row, then judge. Refuses to overwrite without --force.
cargo run --bin comms-digest-eval -- "$SJEL_PERSONAL_ROOT/config/comms-digest-quality.json"
```

**One metric decides: the unfaithful rate.** A digest asserting what its source does not
support is read instead of the article, and nothing downstream can catch it.
`max_unfaithful_percent` carries 2.0 from the start because it is a policy judgement rather
than a measurement — the same distinction `max_false_eviction_percent` carries in the mail
corpus. `minimum_useful_percent` is null until the first run has been read.

**Usefulness is reported per producer and never gates.** Thin but true is a preference;
confident and false is a defect. A `useful_band` nobody wrote is absent from the mean rather
than counted as a zero.

The sample is **balanced across producers**, not drawn from the whole table: the question is
which rung is better, and an unbalanced sample answers which rung ran most. The rows come in
the store's own stable order, so a re-export from an unchanged database is the same file.

A fixture is **skipped with a named reason** when the store holds no digest for it, when the
text changed since it was judged, or when a different rung wrote it — and an all-skipped
corpus prints `FAIL — nothing was scored`, never a 0%. The runner refuses a corpus with any
unjudged row.

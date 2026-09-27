# libs/extraction

One home for **turning a fetched document into text**, and for the question of who gets to
read which kind of document.

A shared library, not a capability: no domain of its own, no store, no HTTP client, no upstream
verdict of its own (CONTRIBUTING.md#three-architectural-nouns). Consumers declare an
`sjel-extraction` path dependency in the workspace.

## Why it exists

It was `capabilities/comms/src/extraction.rs` until a second capability needed the same job.

`capabilities/transit` reads a ticket file into text plus optional Markdown, and had grown its
own `Document` / `DocumentBackend` vocabulary for it — the same job under a second set of names,
already diverged from the first. CONTRIBUTING.md#schemas-and-dependency-direction promotes code to
`libs/` at the second real consumer, provided it owns no domain of its own. This owns none.

The promotion is placement, not redesign: the two readers below are the ones comms had, byte for
byte, with their measurements and their tests.

## What is in and what is out

Extraction stops at **faithful bytes to text**. Cleaning is a separate stage and lives with its
consumer — `comms::normalize` owns it (#86). An extractor that stripped page furniture would be
making, silently, the judgement that stage exists to make inspectably.

Fetching is also out. The GitHub API, Reddit's JSON, yt-dlp, a plain GET and a ticket file
dropped on an HTTP endpoint speak five different protocols; what they all end up holding is a
document plus what kind of document it is, and only that second half is this crate's (#77).

So is `TranscriptSource`. Whether a source offered the document or a stand-in is a fetch-policy
judgement about the source, not about the reader, and it stayed in
`comms::provenance` when the readers moved.

## The rungs

For a document that HAS a text layer, `for_class` is the whole policy: cost-ordered, first match
wins, one reader per class.

| Class | Reader | Note |
|---|---|---|
| `Html`, `PlainText` | `Builtin` | Hand-rolled, no dependency. Keeps the line structure the normalizer's predicate table needs |
| `Pdf` | `Xberg` | **Rung 1.** Feature-gated, see below. Last-resort quality by measurement, not by reputation — see its doc comment |
| `Image` | `VisionOcr` | **Rung 2.** Apple Vision through `tools/visocr`, as a subprocess. macOS-only, and absent rather than erased where it is not |

`Builtin` deliberately keeps HTML. xberg returns more text on every URL in the recorded
benchmark and is still not the right reader for the class: it is an extractor and not a
readability cleaner, so navigation and share widgets survive it. Swapping the HTML path is a
change with a scorecard already in place — the `html` class of `capabilities/comms/eval/` — not
a side effect of wanting PDFs (#77).

A page with **no** text layer is the one case that needs a walk instead of a lookup, because
whether the next rung is required depends on what the last rung returned rather than on the
input class. `ladder::read_scanned` is that walk:

```text
rung 2 (Apple Vision)
  ├─ Unavailable ──────────────────────────► rung 3
  ├─ Engine failure ───────────────────────► stop, report it
  └─ Ok(text) → math detector
                  ├─ quiet ────────────────► stop, keep the text
                  └─ fires ────────────────► rung 3
```

**Rung 3 holds no engine.** `ocr_role` asks `libs/inference` for the `ocr` role and refuses
today with or without one, because no engine has cleared the frozen corpus at [`eval/`](eval/).
That is the same gate `multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` cleared and
`multilingual-e5-small-mlx` did not.

### Why rung 3 exists at all

Rung 2 does not fail on a page of notation. It succeeds and returns something wrong, which is
worse. `upstreams.toml [auge]` measured that twice, and
[`eval/results/2026-09-02-apple-vision-baseline.md`](eval/results/2026-09-02-apple-vision-baseline.md)
is this repository's own run: **100.0% prose recall, 58.8% notation**. On printed pages the
engine does not corrupt a displayed formula, it deletes it and hands back the surrounding prose
as though the page had none.

`math::inspect` is the only thing that reaches rung 3. It reads rung 2's TEXT — no image, no
model — and asks whether the shape of the answer is the shape of that failure. Its thresholds
are **not measured at scale**, and its doc comment says so in those words.

### Why `Pdf` is not rung 2's second class

`tools/visocr` builds its image with `NSImage(contentsOfFile:)`, which renders page one of a PDF
and nothing else. Registering the class would mean returning one page's text under a `producer`
claiming the document was read. Axon has no rasterizer, so the honest boundary is that rung 2
reads pixels; rasterizing a PDF into pages is a named follow-up, not something to fake.

## The `xberg` feature

On by default; a consumer that says nothing gets the whole ladder.

`capabilities/transit` is the one that opts out, with `default-features = false`. It shells out
to the xberg CLI on purpose rather than linking the crate — the reason is written at
`capabilities/transit/src/document.rs`, "Shells out to the xberg CLI" — and a dependency that
arrived by promotion would override a decision that capability made deliberately.

Without the feature there is no PDF rung, and the crate says so: `for_class(Pdf)` returns `None`
and `require(Pdf)` returns `NoExtractor`. Never an empty body, which is indistinguishable from
"this page had nothing".

## Errors distinguish "not here" from "failed here"

A ladder walker has to tell the two apart, or it swallows a real failure while falling through
to the next rung.

- `UnsupportedClass` / `NoExtractor` — nothing here reads this class. A rung above may.
- `Engine` — this reader ran on **this** document and failed on it. That is a fact about the
  document, and it stops the walk.

## The corpus gate

[`eval/`](eval/) holds the frozen DE/EN corpus **no OCR engine enters this ladder without
clearing**. Six synthetic pages committed as bytes, judgements written from their Typst sources
before any engine ran, fixed acceptance, append-only results including the failures.

```sh
cargo run -p sjel-extraction --bin extraction-gate -- libs/extraction/eval/ocr-corpus.json
```

Scoring is hermetic and lives in `src/gate.rs`, unit-tested against a recorded engine output, so
`cargo test` checks the rule on a host with no macOS and no engine. Running a live engine is the
binary's job and the operator's decision — the shape `comms-extraction-eval` and
`bun run-relevance.ts` already have. `eval/README.md` has the discipline and the acceptance
table.

## Dependency rule

`thiserror` and `sjel-inference`, plus `serde`/`serde_json` for the corpus, plus `xberg` and
`tokio` behind the feature above. Nothing else. Every consumer inherits this surface, so it
stays small enough to read — and `sjel-inference` was checked against that rule rather than
assumed: it adds no crate to `capabilities/comms` or `capabilities/transit` that each did not
already declare directly.

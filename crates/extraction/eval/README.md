# The frozen DE/EN OCR corpus gate

**No engine enters the extraction ladder without a passing record here.**

That rule is not new and this directory is not a new idea. It is the instrument that decided
Sjel's embedding and reranking adoptions, pointed at a different job:
`capabilities/comms/eval/README.md` holds the original and states the discipline that makes it
worth anything.

> Add or revise a judgement from its meaning and rationale before looking at a model's score.
> Never tune a label merely to turn a failing model green.

`multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` entered Sjel by clearing a corpus like
this one. `multilingual-e5-small-mlx` and both Apple native embedding variants did not, and
their failing runs are still on disk. The corpus was not weakened to accommodate any of them.

## Why the gate lives here and not in a capability

It judges an **engine**, not a capability, and both consumers of `libs/extraction` have to be
able to cite it. `capabilities/comms/eval/` keeps its own corpus and keeps it for a different
reason: that one scores the extractor→normalizer seam, which is a comms judgement about comms
text. This one scores whether a reader can read a page at all.

## Running it

```sh
cargo run -p sjel-extraction --bin extraction-gate -- libs/extraction/eval/ocr-corpus.json
```

By hand. `cargo test` never runs an engine: the scoring rule lives in
`libs/extraction/src/gate.rs`, is pure over the engine's text, and is unit-tested against the
recording under `recorded/`. That is what lets a host with no macOS, no Vision and no binary
still check the rule.

`SJEL_VISOCR_BIN` points at a `visocr` that is not on `PATH` — `tools/visocr/build.sh` puts one
in `target/tools/` by default. `--record <file>` writes the engine's verbatim output back into
`recorded/`, which is how the hermetic half stays honest.

The runner exits non-zero unless all three lines pass. **The Apple Vision baseline exits
non-zero on purpose.**

## What is frozen, and what regenerating costs

Six pages, committed as PNG bytes: `de-prose`, `en-prose`, `de-en-mixed`, `de-table`,
`de-math`, `en-math`. Fully synthetic — no scan, no personal data, safe in a public repository.

The bytes are frozen rather than rendered per run because comparability across engines and
across months is the entire product of a gate, and Typst font resolution differs per machine
and per OS. The `.typ` sources sit beside them as provenance, and `fixtures/render.sh` is the
recipe.

**Regenerating invalidates every earlier result.** A different rendering is a different corpus,
and an OCR score compared across two renderings is not a comparison. Nothing in `cargo test` or
in the gate binary calls `render.sh`; running it is a deliberate operator act, and the run
record that follows it must say so and name the `typst` version that produced the new bytes.

## The judgements

Written from the `.typ` sources before any engine ran, per fixture:

- **`must_survive`** — exact strings the page carries. Matched after collapsing whitespace on
  both sides, because an engine's line breaks are a fact about the renderer's line width and
  not about the characters it read.
- **`must_not_survive`** — strings that must NOT appear. Empty on the prose pages, and that is
  deliberate rather than lazy: `capabilities/comms/eval/`'s copy of this field catches page
  furniture, and a rendered page has none. Here it carries a recorded misreading instead.
- **`forbidden_confusions`** — ordered pairs, `{"expected": "=", "read_as": "-"}`. Reported only
  where the corpus can prove it: the judged string is absent AND the same string with the
  substitution applied is present. A candidate that reads a relation as a hyphen fails on the
  signature that decided the ladder, named, rather than on an aggregate that could hide it.
- **`detector_rule`** — what `libs/extraction/src/math.rs` must do on this page. `must_not_fire`
  on prose, a table and the mixed page. On notation, `must_fire_when_notation_failed`: the
  detector must fire exactly when the engine got the notation wrong. Stated as a coupling
  because a perfect engine needs no rung 3, and a detector that fired on it anyway would be
  wrong about that.

`de-table` asks only that every CELL survive. `upstreams.toml [auge]` already records that
Vision returns text observations and hands a table back column-major with the legs interleaved,
so row reconstruction is a known loss and no threshold here pretends to score it.

### One judgement revision, recorded rather than made quietly

The two notation fixtures were revised on 2026-09-02, after the first run, and the revision is
written into the corpus itself under `_judgement_revision`. The original judgements named only
inline text, so they could not see that the displayed equations were absent from the output
altogether — the run scored 90.9% on a page whose every formula had vanished. The revision adds
the operators the sources carry (`π`, `∫`, `∑`, `√`, `±`).

It makes the corpus **stricter**. Acceptance thresholds were not touched, and that is the line:
strengthening a judgement from the source is corpus work, weakening one to admit an engine is
the thing this file exists to prevent.

## Acceptance

Fixed, and unchanged between candidates:

| | |
|---|---|
| `minimum_must_survive_percent` | 100.0 |
| `maximum_must_not_survive` | 0 |
| `maximum_forbidden_confusions` | 0 |
| `require_detector_agreement` | true |

## Two verdict lines, never one number

Prose recall and notation fidelity are reported **apart**, because the whole ladder is built on
the fact that they diverge. A single aggregate would hide exactly the split that decides which
rung an engine is fit for.

- An engine may be adopted for **rung 2** on the prose line alone.
- Only a notation pass earns the **rung 3** `ocr` role.

A third line reports the detector's own agreement. It scores `math.rs`, not the engine, and it
is the only place a false positive on prose can be caught before it costs a rung-3 call on every
German article Sjel reads.

## Results are append-only, including the failures

Under [`results/`](results/), dated and named for the engine. The first record is
[`2026-09-02-apple-vision-baseline.md`](results/2026-09-02-apple-vision-baseline.md), which
clears the prose line at 100.0% and fails the notation line at 58.8%. Recording that here is
what makes rung 3 evidence-backed inside Sjel rather than a claim in somebody's memory.

## How an engine actually enters

1. A passing record under `results/`, on the unchanged corpus, reported on both verdict lines.
2. Its `upstreams.toml` entry quotes those numbers and points at the record file.
3. Only then is `ocr` declared in the deployment's `inference.json`.

`upstreams.toml [dolphin]` and `[ocrs]` are held as direction only and each points here. Neither
has a German measurement on this machine. No engine enters this ladder on a project's own
description of itself.

**A Linux run records rung 2 as absent, not as a failure.** Apple Vision is macOS-only, so on
that host the ladder is rung 1 then rung 3 with nothing in between — the case `upstreams.toml
[ocrs]` already names as the one that would revive that entry.

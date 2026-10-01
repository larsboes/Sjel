# Decisions benchmark

Can a local System One model make the small typed decisions Sjel needs? This suite asks
that question the same way for every model that serves `POST /v1/systemone`: CLM's reference
server and Ollama 0.35 or later. The adapter that uses these models in Pi is
[`Packs/harness/extensions/clm-classifier.ts`](../../../Packs/harness/extensions/clm-classifier.ts).
The evidence about the models themselves is in [the local decision entry](../../local-decision-models.md).

## The cases

[`cases.jsonl`](cases.jsonl) holds 43 cases in five tasks, shaped after decisions Sjel makes or plans:

| Task | Cases | Question |
|---|---|---|
| `triage` | 10 | Sort a message into act today, read later, archive or spam. Does it ask for a reply? |
| `media` | 8 | Pick a photo collection from metadata only. Are two files possible duplicates? |
| `role` | 8 | Which inference role does a job need? Does it need generated text? |
| `calendar` | 8 | What kind of event is this? Does it need travel time? |
| `alert` | 9 | Is a backup, disk or service alert info, warn or critical? Should it wake the operator? |

33 cases are `choice` with 3 to 5 options, and 10 are `noul` (yes/no). Each line is
`{id, task, state, question: {type, instructions, criteria}, expected}`.

The cases are synthetic and written by hand. No text comes from a real inbox, photo library or
calendar, because this repository is public. One author wrote the criteria and the expected
answers on 2026-09-30, before any model ran. Nobody else checked the labels.

## How a run works

```bash
bun tools/decision-bench.ts --backend ollama --model nimble
bun tools/decision-bench.ts --backend clm --model clm-latest --url http://127.0.0.1:8700/v1/
```

- **One request per case.** Each request carries one question, so each latency belongs to one case.
- **Warm-up.** The first case goes to the server once before timing starts, so model load time
  stays out of the latencies. Its answer is discarded.
- **Scoring.** A `noul` counts as yes at 0.5 or above.
- **Confidence.** For a `choice`, the result keeps the server's own `confidence`. The two
  servers may not define it the same way. For a `noul`, it keeps the probability of the answer given.
- **Failures.** A failed request counts as wrong. Its latency is left out of the percentiles.

Each run writes one file to [`results/`](results/), named
`<date>-<backend>-<model>.json`. The dashboard's `/research` Benchmarks tab reads these files.
A result file records what one run measured on one machine. Do not edit it after the run.
Rerun instead.

## Not run yet: CLM

CLM was not run on this Mac. Its reference serving recipe needs a vLLM Qwen3-8B pooling server
plus the separate `clm-serve` process. No run so far shows that recipe working on Apple Silicon
([the local decision entry](../../local-decision-models.md) has the detail). A CLM result file
appears here only after the real encoder and heads pass `/clm-probe`.

## What this does not show

- **Accuracy on real data.** Real mail, photos and calendars are longer, noisier and more
  ambiguous than these cases. A high score here is a floor test, not evidence for production use.
- **Label quality.** One person wrote every expected answer. A case every model gets wrong
  may be mislabelled rather than hard. Read those cases before trusting the score.
- **Precision.** 43 cases is a small sample. One case moves accuracy by about 2.3 points.
  Treat differences of a few points between models as noise.
- **Calibration.** Nothing here tests whether a confidence of 0.9 is right nine times in ten.
  An autonomy threshold needs that measurement first.
- **Other hardware.** Latency depends on the machine, the model load and whatever else is running.

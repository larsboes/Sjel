# multilingual-e5-small-mlx baseline — 2026-07-30

- Model: `multilingual-e5-small-mlx`
- Registered upstream pin: `5030c7625865046d350eeea28f427d80353d0ac0`
- Server: oMLX 0.5.0, `/v1/embeddings`
- Shape: 30 vectors in one batch, 384 dimensions, `query:`/`passage:` roles
- Result: **FAIL**

| Metric | Observed | Required |
|---|---:|---:|
| Useful top-1 | 0.667 (4/6) | 1.000 |
| Pairwise accuracy | 0.765 | 0.750 |
| Mean nDCG | 0.865 | 0.900 |

Four cases ranked a useful candidate first. The local-AI, migration-research and
career-visibility cases put the directly relevant cross-language passage first. Scholarship
eligibility put the useful but incomplete German announcement above the direct English match.

Two cases expose material weaknesses:

- `architecture-boundaries-en` ranked a same-language sentence advocating executable provider
  configuration above the German typed-boundary match. Its vocabulary is close but its stance
  is opposite, so this is not a judgement-label problem.
- `rail-reliability-en` ranked an English local-LLM sentence above every rail candidate. The
  direct German historical-data match placed third.

The oMLX adapter was inspected after the failure: this XLM-R/BERT path uses attention-mask
mean pooling followed by normalization, matching multilingual E5's expected pooling. The
failure is therefore retained as the small model's baseline, not hidden by changing labels or
acceptance thresholds. It remains operationally compatible but is not yet good enough to be
declared the quality default. The next comparison should run this unchanged corpus against a
stronger multilingual embedding model and Apple's native embedding where available.

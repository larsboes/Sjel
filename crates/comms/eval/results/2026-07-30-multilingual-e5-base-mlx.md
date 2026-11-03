# multilingual-e5-base-mlx baseline — 2026-07-30

- Model: `multilingual-e5-base-mlx`
- Registered upstream pin: `576fdf3eab52a419f6d126a0c4d7c59b3882ffde`
- Server: oMLX 0.5.0, `/v1/embeddings`
- Shape: 30 vectors in one batch, 768 dimensions, `query:`/`passage:` roles
- Result: **PASS**

| Metric | Observed | Required |
|---|---:|---:|
| Useful top-1 | 1.000 (6/6) | 1.000 |
| Pairwise accuracy | 0.882 | 0.750 |
| Mean nDCG | 0.986 | 0.900 |

The model ranked the directly relevant cross-language passage first in all six cases. It
corrected both material failures retained in the E5-small baseline: the German typed-boundary
passage beat the contradictory English configuration sentence, and the German historical
rail-data passage beat the unrelated English local-LLM sentence.

The exact snapshot occupies 547 MiB locally. The full runner completed in 1.14 seconds on this
machine; oMLX RSS immediately after the request was about 1.16 GB. Comms work is bursty and
revision-cached, so the model receives a 60-second per-model idle TTL instead of remaining
resident. This is a bounded resource increase over E5-small, not a reason to weaken oMLX's
memory guard.

Decision: promote E5-base to the Comms default and retain E5-small only as rejected baseline
evidence. Apple's native embedding comparison remains useful because it can remove even this
downloaded model, but it must clear the same cross-language corpus.

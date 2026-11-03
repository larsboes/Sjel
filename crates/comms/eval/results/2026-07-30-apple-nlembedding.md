# Apple NLEmbedding baseline — 2026-07-30

- API: `NaturalLanguage.NLEmbedding.sentenceEmbedding(for:)`
- German system model: revision `1`, 640 dimensions
- English system model: revision `1`, 512 dimensions
- Shape: 30 sentence vectors; every query-language model ranks both DE and EN candidates
- Result: **FAIL**

| Metric | Observed | Required |
|---|---:|---:|
| Useful top-1 | 0.500 (3/6) | 1.000 |
| Pairwise accuracy | 0.500 | 0.750 |
| Mean nDCG | 0.699 | 0.900 |

The German model ranked a completely unrelated same-language travel-map passage above the
directly relevant English MLX-memory passage. The English model similarly put weaker or
unrelated English candidates ahead of the intended German matches in the architecture, rail and
career cases. Three of six top results were useful, but none was the directly relevant
cross-language candidate.

The optimized standalone runner completed in 0.67 seconds and reported 76,660,736 bytes
(about 73 MiB) maximum RSS. This is materially lighter than E5-base through oMLX, but it does
not preserve bilingual relevance and misses every acceptance threshold.

Decision: do not adopt for Comms. Retain the runner because macOS can update the system models;
each future run must record both language revisions and continue to make one query-language
model rank both German and English candidates.

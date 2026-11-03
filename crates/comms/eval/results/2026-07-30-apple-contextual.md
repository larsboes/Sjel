# Apple NLContextualEmbedding baseline — 2026-07-30

- API: shared Latin `NaturalLanguage.NLContextualEmbedding`
- System model: `5C45D94E-BAB4-4927-94B6-8B5745C46289`, revision `1`
- Shape: 30 mean-pooled vectors, 512 dimensions, maximum sequence length 256
- Languages under test: German and English in one declared model
- Result: **FAIL**

| Metric | Observed | Required |
|---|---:|---:|
| Useful top-1 | 0.667 (4/6) | 1.000 |
| Pairwise accuracy | 0.618 | 0.750 |
| Mean nDCG | 0.776 | 0.900 |

The runner requested the Apple-managed Latin asset explicitly, loaded it, passed each text's
actual language and mean-pooled all documented subword vectors. It then applied the unchanged
human judgements and acceptance thresholds.

Only the career case ranked the strongest cross-language candidate first. In five of six cases
the top result used the query language; four of those same-language winners were weaker than
the intended cross-language match. This includes ranking the unrelated local-LLM passage above
the directly relevant German historical-rail passage. The observed shape is consistent with a
material cross-language ranking weakness on this corpus, not merely one close score.

The optimized standalone runner completed in 0.75 seconds and reported 89,423,872 bytes
(about 85 MiB) maximum RSS for all 30 vectors. That is substantially lighter than the warm oMLX
process observed after E5-base, but the native model does not clear the quality floor.

Decision: retain the runner to detect future system-model revisions, but do not replace
E5-base. `NLEmbedding` remains the separate purpose-built semantic-similarity comparison; it
also ran and failed, so this contextual mean-pooling experiment must not be presented as a
substitute for a missing sentence-model result.

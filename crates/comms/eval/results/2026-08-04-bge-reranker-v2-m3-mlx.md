# bge-reranker-v2-m3-mlx baseline - 2026-08-04

- Model: `bge-reranker-v2-m3-mlx`
- Source: `soichisumi/bge-reranker-v2-m3-mlx`
- Revision: `b4577f49e18adb53ed9e557192094f69f3dc2c1c`
- License: Apache-2.0
- Runtime: oMLX 0.5.0, loopback `/v1/rerank`
- Corpus SHA-256: `74a54a928a62f9b676b5bcd36d596a71613e2ef0aed23583cd307bb0541cf701`
- Weight SHA-256: `80be6e38dfd2156d865a5068cdd78774f29b4b91ce100acc9f331c382e2b18b4`

The pinned tree was inspected before execution. It contains XLM-RoBERTa
sequence-classification config, tokenizer data, one 1,135,556,833-byte safetensors shard and its
index. It has no `loader.py` or other executable model code. The downloaded weight matched the
Git LFS object hash before oMLX loaded it.

## Result

```text
local-ai-resources-de: top=mlx-memory-en score=0.264 judgement=3 nDCG=1.000
architecture-boundaries-en: top=contracts-de score=0.024 judgement=3 nDCG=1.000
migration-research-de: top=dsr-migration-en score=0.998 judgement=3 nDCG=1.000
rail-reliability-en: top=punctuality-data-de score=0.140 judgement=3 nDCG=0.993
scholarship-eligibility-de: top=funding-criteria-en score=0.856 judgement=3 nDCG=0.993
career-visibility-en: top=cfp-open-source-de score=0.013 judgement=3 nDCG=0.972
top1-relevant=1.000 pairwise=0.912 mean-nDCG=0.993 result=PASS
```

The model clears all unchanged acceptance thresholds: every top-ranked candidate is useful,
pairwise accuracy exceeds 0.75, and mean nDCG exceeds 0.90. Scores also span `0.000`-`0.998`,
which removes the narrow high-cosine band that motivated the reranking stage.

The first complete six-query run, including the model's cold load, took about 2.6 seconds. The
temporary oMLX process reported about 1.68 GiB RSS after the run. Those figures are evidence for
this Apple Silicon host, not a portable resource guarantee.
